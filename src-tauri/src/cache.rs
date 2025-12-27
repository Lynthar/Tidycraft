use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::scanner::{AssetInfo, ScanResult};

/// Cache entry for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub path: String,
    pub modified: u64,
    pub size: u64,
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
    const CACHE_VERSION: u32 = 1;

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

        fs::write(&cache_path, content)?;
        Ok(())
    }

    /// Check if a file needs re-scanning
    pub fn needs_rescan(&self, path: &str, modified: u64, size: u64) -> bool {
        match self.entries.get(path) {
            Some(entry) => entry.modified != modified || entry.size != size,
            None => true,
        }
    }

    /// Add or update an entry
    pub fn update_entry(&mut self, asset: AssetInfo, modified: u64) {
        let entry = CacheEntry {
            path: asset.path.clone(),
            modified,
            size: asset.size,
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

/// Get file modification time as unix timestamp
pub fn get_modified_time(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
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
        assert!(cache.needs_rescan("/test/file.png", 12345, 1000));
    }
}
