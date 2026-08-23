use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A tag that can be assigned to assets. `description` is optional context the
/// user fills in via TagManager; AI Learning passes it to the LLM, falling back
/// to sample paths where the tag is applied when it is empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Absent from the file unless the user fills it in; `default` loads older
    /// files written before the field existed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

/// Tags storage - persisted to a JSON file in the project root. In memory the
/// keys are the scanner's absolute forward-slash paths; only the file on disk
/// uses project-relative keys (see [`TagsFile`]).
#[derive(Debug, Clone, Default)]
pub struct TagsData {
    pub tags: Vec<Tag>,
    /// Mapping from asset path to list of tag IDs
    pub asset_tags: BTreeMap<String, Vec<String>>,
}

const TAGS_FILE: &str = ".tidycraft-tags.json";

/// On-disk shape (version 2): keys are project-relative with `/` separators so
/// the file keeps working when the folder moves or is cloned. An absolute key
/// is v1 legacy or another root's binding — preserved verbatim, never guessed at.
#[derive(Serialize, Deserialize)]
struct TagsFile {
    /// `default` loads v1 files written before the field existed.
    #[serde(default)]
    version: u32,
    tags: Vec<Tag>,
    asset_tags: BTreeMap<String, Vec<String>>,
}

/// Forward-slash form of the project root. Both sides of the relative↔absolute
/// conversion must use this — mixed separators would fail every prefix match
/// and silently orphan all bindings.
fn root_key(project_path: &Path) -> String {
    let root = crate::scanner::path_to_string(project_path);
    root.trim_end_matches('/').to_string()
}

/// `/`-rooted, drive-letter or UNC. Deliberately platform-independent string
/// rules: a file written on Windows must classify the same way on macOS.
fn is_absolute_key(key: &str) -> bool {
    let b = key.as_bytes();
    key.starts_with('/')
        || key.starts_with("\\\\")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'/' || b[2] == b'\\'))
}

impl TagsData {
    /// Load tags from the project directory. A missing file is the normal "no
    /// tags yet" state; a file that exists but will not parse is backed up to
    /// `.corrupt` first, so the next save cannot overwrite recoverable data.
    pub fn load(project_path: &Path) -> Self {
        let tags_file = project_path.join(TAGS_FILE);
        if tags_file.exists() {
            match fs::read_to_string(&tags_file) {
                Ok(content) => match serde_json::from_str::<TagsFile>(&content) {
                    Ok(file) => return Self::from_file(file, project_path),
                    Err(e) => {
                        // Keep the first backup — the likeliest complete one.
                        let backup = project_path.join(format!("{}.corrupt", TAGS_FILE));
                        if !backup.exists() {
                            let _ = fs::rename(&tags_file, &backup);
                        }
                        eprintln!(
                            "[tags] {} failed to parse ({e}); backed up to {}",
                            TAGS_FILE,
                            backup.display()
                        );
                    }
                },
                Err(e) => eprintln!("[tags] failed to read {}: {e}", TAGS_FILE),
            }
        }
        Self::default()
    }

    /// Rejoin relative keys to this root. The (rare) collision between a joined
    /// key and a preserved absolute one merges instead of dropping a side.
    fn from_file(file: TagsFile, project_path: &Path) -> Self {
        let root = root_key(project_path);
        let mut asset_tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, ids) in file.asset_tags {
            let abs = if is_absolute_key(&key) {
                key
            } else {
                format!("{root}/{key}")
            };
            let entry = asset_tags.entry(abs).or_default();
            for id in ids {
                if !entry.contains(&id) {
                    entry.push(id);
                }
            }
        }
        Self {
            tags: file.tags,
            asset_tags,
        }
    }

    /// Save tags to the project directory. Atomic (temp + rename), so a crash
    /// mid-write can never truncate the file. The temp sibling keeps the dotfile
    /// prefix, so the scanner and watcher skip it.
    pub fn save(&self, project_path: &Path) -> Result<(), String> {
        let tags_file = project_path.join(TAGS_FILE);
        let prefix = format!("{}/", root_key(project_path));
        let file = TagsFile {
            version: 2,
            tags: self.tags.clone(),
            asset_tags: self
                .asset_tags
                .iter()
                .map(|(k, ids)| {
                    (
                        k.strip_prefix(&prefix).unwrap_or(k).to_string(),
                        ids.clone(),
                    )
                })
                .collect(),
        };
        let content = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        crate::fs_atomic::write_atomic(&tags_file, content.as_bytes()).map_err(|e| e.to_string())
    }

    /// Create a new tag
    pub fn create_tag(&mut self, name: String, color: String) -> Tag {
        let id = uuid::Uuid::new_v4().to_string();
        let tag = Tag {
            id,
            name,
            color,
            description: None,
        };
        self.tags.push(tag.clone());
        tag
    }

    /// Delete a tag and remove it from all assets
    pub fn delete_tag(&mut self, tag_id: &str) {
        self.tags.retain(|t| t.id != tag_id);
        for tags in self.asset_tags.values_mut() {
            tags.retain(|id| id != tag_id);
        }
    }

    /// Update a tag. Each field is patched only when its argument is `Some`;
    /// `Some(Some(""))` clears the description.
    pub fn update_tag(
        &mut self,
        tag_id: &str,
        name: Option<String>,
        color: Option<String>,
        description: Option<Option<String>>,
    ) -> Option<Tag> {
        if let Some(tag) = self.tags.iter_mut().find(|t| t.id == tag_id) {
            if let Some(n) = name {
                tag.name = n;
            }
            if let Some(c) = color {
                tag.color = c;
            }
            if let Some(d) = description {
                // Empty or whitespace-only means "no description".
                tag.description = match d {
                    Some(s) if s.trim().is_empty() => None,
                    Some(s) => Some(s),
                    None => None,
                };
            }
            return Some(tag.clone());
        }
        None
    }

    /// Add a tag to an asset
    pub fn add_tag_to_asset(&mut self, asset_path: &str, tag_id: &str) {
        if !self.tags.iter().any(|t| t.id == tag_id) {
            return;
        }

        let tags = self.asset_tags.entry(asset_path.to_string()).or_default();
        if !tags.contains(&tag_id.to_string()) {
            tags.push(tag_id.to_string());
        }
    }

    /// Remove a tag from an asset
    pub fn remove_tag_from_asset(&mut self, asset_path: &str, tag_id: &str) {
        if let Some(tags) = self.asset_tags.get_mut(asset_path) {
            tags.retain(|id| id != tag_id);
        }
    }

    /// Move every tag binding from `old_path` to `new_path`, merging into any
    /// bindings already there. No-op when `old_path` had none.
    pub fn rename_path(&mut self, old_path: &str, new_path: &str) {
        if old_path == new_path {
            return;
        }
        let old_ids = match self.asset_tags.remove(old_path) {
            Some(ids) => ids,
            None => return,
        };
        let entry = self.asset_tags.entry(new_path.to_string()).or_default();
        for id in old_ids {
            if !entry.contains(&id) {
                entry.push(id);
            }
        }
    }

    /// Drop every tag binding for `path`, so the tags file does not accumulate
    /// orphan entries.
    pub fn remove_path(&mut self, path: &str) {
        self.asset_tags.remove(path);
    }

    /// Move every binding under `old_dir` to the same relative position under
    /// `new_dir`. Matching is component-wise, so renaming `…/Tex` never captures
    /// `…/Textures/*`. Each key goes through [`Self::rename_path`].
    pub fn rename_dir(&mut self, old_dir: &str, new_dir: &str) {
        if old_dir == new_dir {
            return;
        }
        let prefix = format!("{}/", old_dir.trim_end_matches('/'));
        let moved: Vec<String> = self
            .asset_tags
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        let new_base = new_dir.trim_end_matches('/');
        for old_key in moved {
            let new_key = format!("{}/{}", new_base, &old_key[prefix.len()..]);
            self.rename_path(&old_key, &new_key);
        }
    }

    /// Get tags for an asset
    pub fn get_asset_tags(&self, asset_path: &str) -> Vec<Tag> {
        if let Some(tag_ids) = self.asset_tags.get(asset_path) {
            tag_ids
                .iter()
                .filter_map(|id| self.tags.iter().find(|t| &t.id == id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all assets with a specific tag
    pub fn get_assets_with_tag(&self, tag_id: &str) -> Vec<String> {
        // Sorted for determinism: `asset_tags` is a HashMap, and these paths feed
        // the per-asset LLM cache key, which an unstable order would invalidate
        // on every restart.
        let mut paths: Vec<String> = self
            .asset_tags
            .iter()
            .filter(|(_, tags)| tags.contains(&tag_id.to_string()))
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort();
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The in-memory key shape production uses: the scanner's forward-slash
    /// absolute path under the project root.
    fn abs_key(root: &Path, rel: &str) -> String {
        format!("{}/{}", crate::scanner::path_to_string(root), rel)
    }

    #[test]
    fn save_writes_version_2_with_project_relative_sorted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        data.add_tag_to_asset(&abs_key(dir.path(), "z/last.png"), &tag.id);
        data.add_tag_to_asset(&abs_key(dir.path(), "a/first.png"), &tag.id);
        data.save(dir.path()).unwrap();

        let raw = std::fs::read_to_string(dir.path().join(TAGS_FILE)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["version"], 2);
        // Keys are project-relative — the machine-specific root must not be in
        // the file — and sorted, so repeated saves diff cleanly.
        assert!(
            !raw.contains(&crate::scanner::path_to_string(dir.path())),
            "absolute root leaked into the file:\n{raw}"
        );
        assert!(json["asset_tags"]["a/first.png"].is_array());
        assert!(json["asset_tags"]["z/last.png"].is_array());
        assert!(raw.find("a/first.png").unwrap() < raw.find("z/last.png").unwrap());
    }

    #[test]
    fn tags_file_travels_with_the_folder() {
        let old_root = tempfile::tempdir().unwrap();
        let new_root = tempfile::tempdir().unwrap();
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        data.add_tag_to_asset(&abs_key(old_root.path(), "Assets/stone.png"), &tag.id);
        data.save(old_root.path()).unwrap();

        // Simulate the folder moving: the file arrives at the new root untouched.
        std::fs::copy(
            old_root.path().join(TAGS_FILE),
            new_root.path().join(TAGS_FILE),
        )
        .unwrap();

        let moved = TagsData::load(new_root.path());
        assert_eq!(
            moved
                .get_asset_tags(&abs_key(new_root.path(), "Assets/stone.png"))
                .len(),
            1,
            "bindings must follow the folder to its new location"
        );
    }

    #[test]
    fn v1_absolute_keys_under_the_root_still_load_and_upgrade_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let key = abs_key(dir.path(), "a/x.png");
        let v1 = format!(
            r##"{{"tags":[{{"id":"t1","name":"Hero","color":"#f00"}}],"asset_tags":{{"{key}":["t1"]}}}}"##
        );
        std::fs::write(dir.path().join(TAGS_FILE), v1).unwrap();

        let data = TagsData::load(dir.path());
        assert_eq!(data.get_asset_tags(&key).len(), 1, "v1 binding resolves");

        data.save(dir.path()).unwrap();
        let raw = std::fs::read_to_string(dir.path().join(TAGS_FILE)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["version"], 2);
        assert!(
            !raw.contains(&crate::scanner::path_to_string(dir.path())),
            "v1 keys under the root must relativize on the next save"
        );
    }

    #[test]
    fn keys_outside_the_root_are_preserved_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        // Bindings from another machine or root: never rewritten (guessing could
        // mis-attach them), never dropped (they are someone's data).
        data.add_tag_to_asset("C:/other/proj/x.png", &tag.id);
        data.add_tag_to_asset("/somewhere/else/y.png", &tag.id);
        data.save(dir.path()).unwrap();

        let reloaded = TagsData::load(dir.path());
        assert_eq!(reloaded.get_asset_tags("C:/other/proj/x.png").len(), 1);
        assert_eq!(reloaded.get_asset_tags("/somewhere/else/y.png").len(), 1);
    }

    #[test]
    fn saving_the_same_data_twice_is_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        for name in ["m/mid.png", "z/last.png", "a/first.png"] {
            data.add_tag_to_asset(&abs_key(dir.path(), name), &tag.id);
        }
        data.save(dir.path()).unwrap();
        let first = std::fs::read(dir.path().join(TAGS_FILE)).unwrap();

        TagsData::load(dir.path()).save(dir.path()).unwrap();
        let second = std::fs::read(dir.path().join(TAGS_FILE)).unwrap();
        assert_eq!(first, second, "same data must serialize to the same bytes");
    }

    #[test]
    fn test_create_tag() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Important".to_string(), "#ff0000".to_string());

        assert_eq!(tag.name, "Important");
        assert_eq!(tag.color, "#ff0000");
        assert_eq!(data.tags.len(), 1);
    }

    #[test]
    fn test_add_remove_tag_from_asset() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Test".to_string(), "#00ff00".to_string());

        data.add_tag_to_asset("/path/to/asset.png", &tag.id);
        assert_eq!(data.get_asset_tags("/path/to/asset.png").len(), 1);

        data.remove_tag_from_asset("/path/to/asset.png", &tag.id);
        assert_eq!(data.get_asset_tags("/path/to/asset.png").len(), 0);
    }

    #[test]
    fn test_rename_path_carries_tags() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        data.add_tag_to_asset("/old.png", &tag.id);

        data.rename_path("/old.png", "/new.png");
        assert_eq!(data.get_asset_tags("/old.png").len(), 0);
        assert_eq!(data.get_asset_tags("/new.png").len(), 1);
    }

    #[test]
    fn test_rename_path_merges_into_existing() {
        let mut data = TagsData::default();
        let a = data.create_tag("A".to_string(), "#aa0000".to_string());
        let b = data.create_tag("B".to_string(), "#00aa00".to_string());
        data.add_tag_to_asset("/old.png", &a.id);
        data.add_tag_to_asset("/new.png", &b.id);

        data.rename_path("/old.png", "/new.png");
        // /new.png keeps its own B and gains A; /old.png gone
        let tags = data.get_asset_tags("/new.png");
        assert_eq!(tags.len(), 2);
        assert_eq!(data.get_asset_tags("/old.png").len(), 0);
    }

    #[test]
    fn rename_dir_moves_descendants_component_wise() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        data.add_tag_to_asset("C:/proj/Tex/a.png", &tag.id);
        data.add_tag_to_asset("C:/proj/Tex/sub/deep.png", &tag.id);
        // Sibling with the old dir as a STRING prefix — must not move.
        data.add_tag_to_asset("C:/proj/Textures/b.png", &tag.id);

        data.rename_dir("C:/proj/Tex", "C:/proj/Art");

        assert_eq!(data.get_asset_tags("C:/proj/Art/a.png").len(), 1);
        assert_eq!(data.get_asset_tags("C:/proj/Art/sub/deep.png").len(), 1);
        assert_eq!(data.get_asset_tags("C:/proj/Tex/a.png").len(), 0);
        // The string-prefix sibling stayed put.
        assert_eq!(data.get_asset_tags("C:/proj/Textures/b.png").len(), 1);
    }

    #[test]
    fn rename_dir_merges_into_existing_destination_bindings() {
        let mut data = TagsData::default();
        let a = data.create_tag("A".to_string(), "#aa0000".to_string());
        let b = data.create_tag("B".to_string(), "#00aa00".to_string());
        data.add_tag_to_asset("C:/proj/Old/x.png", &a.id);
        data.add_tag_to_asset("C:/proj/New/x.png", &b.id);

        data.rename_dir("C:/proj/Old", "C:/proj/New");

        // Union at the destination, source gone — rename_path semantics.
        assert_eq!(data.get_asset_tags("C:/proj/New/x.png").len(), 2);
        assert_eq!(data.get_asset_tags("C:/proj/Old/x.png").len(), 0);
    }

    #[test]
    fn test_remove_path_drops_bindings() {
        let mut data = TagsData::default();
        let tag = data.create_tag("X".to_string(), "#ff00ff".to_string());
        data.add_tag_to_asset("/gone.png", &tag.id);

        data.remove_path("/gone.png");
        assert_eq!(data.get_asset_tags("/gone.png").len(), 0);
        // The tag definition itself is untouched
        assert_eq!(data.tags.len(), 1);
    }

    #[test]
    fn get_assets_with_tag_is_sorted() {
        // The result feeds the LLM cache context hash, so it must be
        // deterministic. Insertion order here is unsorted.
        let mut data = TagsData::default();
        let hero = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        let other = data.create_tag("Other".to_string(), "#00ff00".to_string());
        data.add_tag_to_asset("z/last.png", &hero.id);
        data.add_tag_to_asset("a/first.png", &hero.id);
        data.add_tag_to_asset("m/mid.png", &hero.id);
        data.add_tag_to_asset("a/other.png", &other.id);
        assert_eq!(
            data.get_assets_with_tag(&hero.id),
            vec!["a/first.png", "m/mid.png", "z/last.png"]
        );
    }

    #[test]
    fn save_then_load_roundtrips_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = TagsData::default();
        let tag = data.create_tag("Hero".to_string(), "#ff0000".to_string());
        let key = abs_key(dir.path(), "a/x.png");
        data.add_tag_to_asset(&key, &tag.id);
        data.save(dir.path()).unwrap();

        let loaded = TagsData::load(dir.path());
        assert_eq!(loaded.tags.len(), 1);
        assert_eq!(loaded.get_asset_tags(&key).len(), 1);
        // The atomic-write temp sibling must not survive a successful save.
        assert!(!dir.path().join(format!("{}.tmp", TAGS_FILE)).exists());
    }

    #[test]
    fn load_backs_up_corrupt_file_instead_of_silently_emptying() {
        let dir = tempfile::tempdir().unwrap();
        // A file that exists but can't parse (e.g. a truncated pre-atomic write).
        std::fs::write(dir.path().join(TAGS_FILE), "{ not valid json").unwrap();

        let loaded = TagsData::load(dir.path());
        // Degrades to empty so the app keeps running...
        assert!(loaded.tags.is_empty());
        // ...but the unparseable data is preserved for recovery, and the live
        // file is renamed away so the next save can't clobber the backup.
        assert!(dir.path().join(format!("{}.corrupt", TAGS_FILE)).exists());
        assert!(!dir.path().join(TAGS_FILE).exists());
    }
}
