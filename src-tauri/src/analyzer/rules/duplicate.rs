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

/// Find duplicate files based on content hash
pub fn find_duplicates(assets: &[AssetInfo]) -> AnalysisResult {
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

        // Report duplicates
        for (_hash, duplicates) in by_hash {
            if duplicates.len() < 2 {
                continue;
            }

            // Report all but the first as duplicates
            let original = duplicates[0];
            for duplicate in &duplicates[1..] {
                result.add_issue(Issue {
                    rule_id: "duplicate".to_string(),
                    rule_name: "Duplicate File".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "File is a duplicate of '{}'",
                        original.name
                    ),
                    asset_path: duplicate.path.clone(),
                    suggestion: Some(format!(
                        "Consider removing this file or consolidating with '{}'",
                        original.path
                    )),
                    auto_fixable: false,
                });
            }
        }
    }

    result
}
