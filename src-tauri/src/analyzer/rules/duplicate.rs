use crate::analyzer::{issue_args, AnalysisResult, Issue, Severity};
use crate::scanner::AssetInfo;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// How much of a file the first pass reads. Files of one size are separated
/// by their opening bytes before anything is read in full — see
/// [`find_duplicates`] for why size alone doesn't narrow the field.
const PREFIX_BYTES: u64 = 8192;

/// SHA256 of a file, or of its first `limit` bytes when one is given.
fn calculate_file_hash(path: &Path, limit: Option<u64>) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    let mut remaining = limit.unwrap_or(u64::MAX);

    while remaining > 0 {
        let want = buffer.len().min(remaining as usize);
        let bytes_read = reader.read(&mut buffer[..want]).ok()?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        remaining -= bytes_read as u64;
    }

    let hash = hasher.finalize();
    Some(format!("{:x}", hash))
}

/// Group by a hash, dropping every group that ends up with a single member —
/// nothing that stands alone at any stage can be a duplicate.
fn group_by_hash(assets: Vec<&AssetInfo>, limit: Option<u64>) -> Vec<Vec<&AssetInfo>> {
    let mut by_hash: HashMap<String, Vec<&AssetInfo>> = HashMap::new();
    for asset in assets {
        if let Some(hash) = calculate_file_hash(Path::new(&asset.path), limit) {
            by_hash.entry(hash).or_default().push(asset);
        }
    }
    by_hash.into_values().filter(|g| g.len() > 1).collect()
}

/// Root-relative form of `path` for user-facing text. Both sides come from
/// the scanner's forward-slash normalization, so a plain prefix strip works;
/// falls back to the absolute path if it isn't under `root`.
fn rel<'a>(path: &'a str, root: &str) -> &'a str {
    path.strip_prefix(root)
        .map(|s| s.trim_start_matches('/'))
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Find duplicate files based on content hash. `root` is the scan root —
/// group paths and suggestions are reported root-relative so the frontend
/// and exports never show machine-specific prefixes.
pub fn find_duplicates(assets: &[AssetInfo], root: &str) -> AnalysisResult {
    let mut result = AnalysisResult::new();

    // Group files by size first (optimization)
    let mut by_size: HashMap<u64, Vec<&AssetInfo>> = HashMap::new();
    for asset in assets {
        by_size.entry(asset.size).or_default().push(asset);
    }

    // Then by the first few kilobytes, and only then in full. Size alone barely
    // narrows a texture library: block-compressed images of one size and format
    // have identical byte counts, and differing files differ in their header.
    for (_, same_size_assets) in by_size {
        if same_size_assets.len() < 2 {
            continue;
        }

        let candidates: Vec<Vec<&AssetInfo>> = group_by_hash(same_size_assets, Some(PREFIX_BYTES))
            .into_iter()
            .flat_map(|group| group_by_hash(group, None))
            .collect();

        // Report duplicates (ordering fixed after the loops — the grouping
        // map iterates in random order)
        for duplicates in candidates {
            // ONE issue per content group, carrying the full member list with the
            // original first (the group arrives path-sorted from the scan). One
            // issue per copy with the list cloned onto each is quadratic.
            let original = duplicates[0];
            let first_copy = duplicates[1];
            let group: Vec<String> = duplicates
                .iter()
                .map(|a| rel(&a.path, root).to_string())
                .collect();
            result.add_issue(Issue {
                rule_id: "duplicate".to_string(),
                rule_name: "Duplicate File".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "{} files share identical content (original: '{}')",
                    duplicates.len(),
                    original.name
                ),
                // Anchor on the first redundant copy — "locate" should land
                // on a file the user can act on, not the one to keep.
                asset_path: first_copy.path.clone(),
                suggestion: Some(format!(
                    "Keep '{}' and remove or consolidate the other {} file(s)",
                    rel(&original.path, root),
                    duplicates.len() - 1
                )),
                auto_fixable: false,
                related_paths: Some(group),
                // `file_count` not `count` — i18next reads `count` as a plural
                // selector. The suggestion interpolates two values the message does
                // not, so both must ship for a locale to reproduce the English.
                args: issue_args([
                    ("file_count", duplicates.len().to_string()),
                    ("original", original.name.clone()),
                    ("original_path", rel(&original.path, root).to_string()),
                    ("other_count", (duplicates.len() - 1).to_string()),
                ]),
            });
        }
    }

    // Both grouping maps are HashMaps, so issue order is otherwise random per run.
    // Pin it by path; members within a group are already path-ordered, so each
    // group's "original" is the lexicographically first path.
    result
        .issues
        .sort_by(|a, b| a.asset_path.cmp(&b.asset_path));

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::rules::dcc_source::tests::make_asset;
    use crate::scanner::AssetType;
    use std::path::PathBuf;

    /// `make_asset` declares size 1 for everything, so every fixture lands in
    /// one size bucket and reaches the hashing stages — which is the part
    /// under test here.
    fn write(dir: &Path, name: &str, body: Vec<u8>) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    fn assets(paths: &[PathBuf]) -> Vec<AssetInfo> {
        paths
            .iter()
            .map(|p| make_asset(&p.to_string_lossy(), AssetType::Texture))
            .collect()
    }

    #[test]
    fn prefix_hash_separates_files_that_differ_early() {
        // The whole point of the first pass: same length, different opening
        // bytes, settled without reading either file past 8 KB.
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.dds", vec![b'a'; 40_000]);
        let b = write(dir.path(), "b.dds", vec![b'b'; 40_000]);

        let ha = calculate_file_hash(&a, Some(PREFIX_BYTES)).unwrap();
        let hb = calculate_file_hash(&b, Some(PREFIX_BYTES)).unwrap();
        assert_ne!(ha, hb);

        assert!(
            find_duplicates(&assets(&[a, b]), dir.path().to_str().unwrap())
                .issues
                .is_empty()
        );
    }

    #[test]
    fn files_sharing_a_prefix_are_still_compared_in_full() {
        // Guards the second pass: two files with an identical 8 KB header and
        // different bodies hash the same in the first pass, and this rule's
        // suggestion is to delete one of them.
        let dir = tempfile::tempdir().unwrap();
        let mut body_a = vec![0u8; PREFIX_BYTES as usize];
        let mut body_b = body_a.clone();
        body_a.extend_from_slice(&[1u8; 4_000]);
        body_b.extend_from_slice(&[2u8; 4_000]);
        let a = write(dir.path(), "a.dds", body_a);
        let b = write(dir.path(), "b.dds", body_b);

        assert_eq!(
            calculate_file_hash(&a, Some(PREFIX_BYTES)),
            calculate_file_hash(&b, Some(PREFIX_BYTES)),
            "fixture must collide in the first pass or it tests nothing"
        );
        assert_ne!(calculate_file_hash(&a, None), calculate_file_hash(&b, None));
        assert!(
            find_duplicates(&assets(&[a, b]), dir.path().to_str().unwrap())
                .issues
                .is_empty()
        );
    }

    #[test]
    fn identical_files_larger_than_the_prefix_are_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..30_000u32).map(|i| i as u8).collect();
        let a = write(dir.path(), "a.dds", body.clone());
        let b = write(dir.path(), "b.dds", body);

        let issues = find_duplicates(&assets(&[a, b]), dir.path().to_str().unwrap()).issues;
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].related_paths.as_ref().unwrap().len(), 2);
    }
}
