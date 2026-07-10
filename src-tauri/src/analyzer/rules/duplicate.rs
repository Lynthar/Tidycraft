use crate::analyzer::{AnalysisResult, Issue, Severity};
use crate::scanner::AssetInfo;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Calculate SHA256 hash of a file
fn calculate_file_hash(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer).ok()?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let hash = hasher.finalize();
    Some(format!("{:x}", hash))
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

    // For files with same size, calculate hash
    for (_, same_size_assets) in by_size {
        if same_size_assets.len() < 2 {
            continue;
        }

        // Calculate hashes for potential duplicates
        let mut by_hash: HashMap<String, Vec<&AssetInfo>> = HashMap::new();
        for asset in same_size_assets {
            if let Some(hash) = calculate_file_hash(Path::new(&asset.path)) {
                by_hash.entry(hash).or_default().push(asset);
            }
        }

        // Report duplicates (ordering fixed after the loops — both grouping
        // maps iterate in random order)
        for (_hash, duplicates) in by_hash {
            if duplicates.len() < 2 {
                continue;
            }

            // ONE issue per content group, carrying the full member list
            // (original first — the group arrives path-sorted from the
            // scan). An earlier revision emitted one issue per extra copy
            // with the member list cloned onto each: quadratic in group
            // size, and a real asset library (Kenney all-in-one: one 3178-
            // file group) ballooned the IPC payload past 1 GB and OOM'd
            // the webview. The group card in the UI never needed per-copy
            // issues anyway.
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
            });
        }
    }

    // Both grouping maps above are HashMaps, so issue order was random per
    // run — the report reshuffled on every analysis while every sibling rule
    // emits deterministically. Pin it by path. (Members within a group are
    // already path-ordered: `assets` arrives sorted from the scan, so each
    // group's "original" is the lexicographically first path.)
    result.issues.sort_by(|a, b| a.asset_path.cmp(&b.asset_path));

    result
}
