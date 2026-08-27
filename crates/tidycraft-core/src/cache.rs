use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::scanner::AssetInfo;

/// Cache entry for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub path: String,
    /// Mtime in **nanoseconds**, unlike the whole seconds `AssetInfo.modified`
    /// carries for display. Whole seconds cannot distinguish a same-length
    /// rewrite inside the recorded second; `size` catches only resizing ones.
    pub modified_nanos: u64,
    pub size: u64,
    /// Mtime of the asset's Unity `.meta` sidecar at scan time, nanoseconds as
    /// above (`None` = no sidecar). In the invalidation key so a sidecar-only
    /// rewrite re-parses the asset and `unity_guid` cannot go stale.
    pub meta_modified_nanos: Option<u64>,
    pub asset: AssetInfo,
}

/// Project scan cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanCache {
    pub version: u32,
    pub project_path: String,
    pub created: u64,
    pub entries: HashMap<String, CacheEntry>,
}

impl ScanCache {
    /// Bump whenever the set of extracted metadata fields or the invalidation
    /// key changes, so older caches are rejected and re-scanned. Costs users one
    /// full rescan.
    const CACHE_VERSION: u32 = 7;

    /// Create a new empty cache
    pub fn new(project_path: &str) -> Self {
        ScanCache {
            version: Self::CACHE_VERSION,
            project_path: project_path.to_string(),
            created: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            entries: HashMap::new(),
        }
    }

    /// Get the cache file path for a project
    pub fn cache_path(project_path: &str) -> Option<PathBuf> {
        let cache_dir = dirs::cache_dir()?.join("tidycraft").join("scans");

        // Create hash of project path for cache filename
        let mut hasher = Sha256::new();
        hasher.update(project_path.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        Some(cache_dir.join(format!("{}.json", &hash[..16])))
    }

    /// Load cache from disk
    pub fn load(project_path: &str) -> Option<Self> {
        let cache_path = Self::cache_path(project_path)?;
        let content = fs::read_to_string(&cache_path).ok()?;
        let cache: ScanCache = serde_json::from_str(&content).ok()?;

        // Validate cache version and project path
        if cache.version != Self::CACHE_VERSION || cache.project_path != project_path {
            return None;
        }

        Some(cache)
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<(), std::io::Error> {
        let cache_path = Self::cache_path(&self.project_path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No cache dir"))?;

        // Ensure directory exists
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Atomic (temp + rename), matching tags.rs / undo.rs. `load()` already
        // self-heals from a torn file, so this saves the rescan, not correctness.
        crate::fs_atomic::write_atomic(&cache_path, content.as_bytes())?;
        Ok(())
    }

    /// Whether a file needs re-scanning. Both mtimes are nanoseconds; any change
    /// to the `.meta` sidecar — created, rewritten or deleted — invalidates.
    pub fn needs_rescan(
        &self,
        path: &str,
        modified_nanos: u64,
        size: u64,
        meta_modified_nanos: Option<u64>,
    ) -> bool {
        match self.entries.get(path) {
            Some(entry) => {
                entry.modified_nanos != modified_nanos
                    || entry.size != size
                    || entry.meta_modified_nanos != meta_modified_nanos
            }
            None => true,
        }
    }

    /// Add or update an entry
    pub fn update_entry(
        &mut self,
        asset: AssetInfo,
        modified_nanos: u64,
        meta_modified_nanos: Option<u64>,
    ) {
        let entry = CacheEntry {
            path: asset.path.clone(),
            modified_nanos,
            size: asset.size,
            meta_modified_nanos,
            asset,
        };
        self.entries.insert(entry.path.clone(), entry);
    }

    /// Remove entries for files that no longer exist
    pub fn prune(&mut self, existing_paths: &[String]) {
        let existing_set: std::collections::HashSet<&String> = existing_paths.iter().collect();
        self.entries.retain(|path, _| existing_set.contains(path));
    }

    /// Get all cached assets
    pub fn get_assets(&self) -> Vec<AssetInfo> {
        self.entries.values().map(|e| e.asset.clone()).collect()
    }

    /// Clear the cache
    pub fn clear(project_path: &str) -> Result<(), std::io::Error> {
        if let Some(cache_path) = Self::cache_path(project_path) {
            if cache_path.exists() {
                fs::remove_file(cache_path)?;
            }
        }
        Ok(())
    }
}

/// File mtime in nanoseconds since the epoch — the cache's invalidation stamp,
/// distinct from the whole-second `AssetInfo.modified` the interface renders.
/// `u64` nanoseconds run out in 2554.
pub fn mtime_nanos(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_path_generation() {
        let path = ScanCache::cache_path("/test/project");
        assert!(path.is_some());
    }

    #[test]
    fn test_needs_rescan() {
        let cache = ScanCache::new("/test");
        assert!(cache.needs_rescan("/test/file.png", 12345, 1000, None));
    }

    fn dummy_asset(path: &str, size: u64) -> AssetInfo {
        AssetInfo {
            path: path.to_string(),
            name: "file.png".to_string(),
            extension: "png".to_string(),
            asset_type: crate::scanner::AssetType::Texture,
            size,
            modified: 0,
            metadata: None,
            unity_guid: None,
        }
    }

    #[test]
    fn needs_rescan_tracks_sidecar_meta_mtime() {
        let mut cache = ScanCache::new("/test");
        cache.update_entry(dummy_asset("/test/file.png", 1000), 12345, Some(50));

        // Unchanged on all three axes → cached.
        assert!(!cache.needs_rescan("/test/file.png", 12345, 1000, Some(50)));
        // Sidecar rewritten (mtime moved) → rescan.
        assert!(cache.needs_rescan("/test/file.png", 12345, 1000, Some(60)));
        // Sidecar deleted → rescan.
        assert!(cache.needs_rescan("/test/file.png", 12345, 1000, None));

        // Entry recorded without a sidecar; one appearing later → rescan.
        cache.update_entry(dummy_asset("/test/new.png", 500), 111, None);
        assert!(!cache.needs_rescan("/test/new.png", 111, 500, None));
        assert!(cache.needs_rescan("/test/new.png", 111, 500, Some(70)));
    }
}
