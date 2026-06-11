use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A tag that can be assigned to assets.
///
/// `description` is optional context the user can fill in via TagManager.
/// AI Learning passes it to the LLM as part of the project context bundle
/// so the model knows what each tag means in this user's vocabulary.
/// Empty / None descriptions fall back to "show the model 5 sample paths
/// where this tag is currently applied" — see `lib.rs::llm_suggest_tags`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Skipped on serialize when None so existing `.tidycraft-tags.json`
    /// files stay byte-clean unless the user actually fills a description.
    /// `default` lets us load older files without the field present.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

/// Tags storage - persisted to a JSON file in the project root
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TagsData {
    /// All defined tags
    pub tags: Vec<Tag>,
    /// Mapping from asset path to list of tag IDs
    pub asset_tags: HashMap<String, Vec<String>>,
}

const TAGS_FILE: &str = ".tidycraft-tags.json";

impl TagsData {
    /// Load tags from the project directory.
    ///
    /// A missing file is the normal "no tags yet" state → empty data. But a
    /// file that EXISTS yet fails to parse (a truncated write from before
    /// atomic saves, or a bad hand-edit) must NOT silently degrade to empty:
    /// the very next `save` would then overwrite the (possibly recoverable)
    /// file with empty data and the user's tags would be gone for good. So we
    /// back the corrupt file up first, then start fresh.
    pub fn load(project_path: &Path) -> Self {
        let tags_file = project_path.join(TAGS_FILE);
        if tags_file.exists() {
            match fs::read_to_string(&tags_file) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(data) => return data,
                    Err(e) => {
                        // Preserve the corrupt file so the user can recover.
                        // Keep the first backup (most likely the complete one)
                        // if a `.corrupt` already exists from an earlier run.
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

    /// Save tags to the project directory.
    ///
    /// Atomic write: serialize to a sibling temp file, then rename it over the
    /// target. `std::fs::rename` uses `MoveFileEx(REPLACE_EXISTING)` on Windows
    /// and `rename(2)` on Unix, so a crash mid-write can never leave the tags
    /// file truncated — a reader sees either the old complete file or the new
    /// one. (The previous direct `fs::write` could truncate, and combined with
    /// `load`'s empty-fallback that meant losing every tag.) The `.tmp` sibling
    /// is a dotfile, so the scanner and watcher both skip it.
    pub fn save(&self, project_path: &Path) -> Result<(), String> {
        let tags_file = project_path.join(TAGS_FILE);
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp_file = project_path.join(format!("{}.tmp", TAGS_FILE));
        fs::write(&tmp_file, content).map_err(|e| e.to_string())?;
        fs::rename(&tmp_file, &tags_file).map_err(|e| {
            // Don't leave the temp file behind on a failed rename.
            let _ = fs::remove_file(&tmp_file);
            e.to_string()
        })
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
    /// `None` means "leave unchanged". To explicitly clear a description,
    /// callers pass `Some(Some(""))` and the empty-string normalization
    /// happens here.
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
                // Treat empty/whitespace-only strings as "no description"
                // so we don't ship blank values to the LLM context.
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
        // Verify tag exists
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

    /// Move every tag binding from `old_path` to `new_path`. If `new_path`
    /// already had bindings they're merged (union of tag IDs). No-op when
    /// `old_path` had no bindings. Used when a file is renamed or moved
    /// from inside Tidycraft so its tags follow it to the new location.
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

    /// Drop every tag binding for `path`. Used when a file is deleted
    /// (via trash or by the watcher noticing it vanished externally) so
    /// the tags file doesn't accumulate orphan path entries.
    pub fn remove_path(&mut self, path: &str) {
        self.asset_tags.remove(path);
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
    #[allow(dead_code)]
    pub fn get_assets_with_tag(&self, tag_id: &str) -> Vec<String> {
        // Sorted for determinism. `asset_tags` is a HashMap, so its iteration
        // order is randomized per process; these paths feed the per-asset LLM
        // cache key (via `llm::cache::hash_context`), so an unstable order
        // would change the key on every app restart and silently invalidate
        // the whole suggestion cache. Callers only use the result as prompt
        // context behind a `truncate(5)`, so pinning the order is free.
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
        // `asset_tags` is a HashMap (per-process randomized iteration order),
        // but the result feeds the LLM cache context hash, so it must be
        // deterministic. Insert in a deliberately non-sorted order and assert
        // the output is sorted and excludes other tags' assets.
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
        data.add_tag_to_asset("a/x.png", &tag.id);
        data.save(dir.path()).unwrap();

        let loaded = TagsData::load(dir.path());
        assert_eq!(loaded.tags.len(), 1);
        assert_eq!(loaded.get_asset_tags("a/x.png").len(), 1);
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
