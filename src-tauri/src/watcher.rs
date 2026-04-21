//! Per-project filesystem watcher.
//!
//! Uses `notify-debouncer-full` to coalesce OS filesystem events (editors often
//! write-and-rename in bursts), then re-parses affected files and emits a
//! single `fs-change-{project_id}` Tauri event per debounce window.
//!
//! The watcher holds its `Debouncer` handle inside `ProjectState.watcher` — when
//! the project is unregistered, the state (and thus the debouncer) drops, which
//! tears down the OS watch and closes the event channel; the processing thread
//! exits naturally on channel close.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::project;
use crate::scanner::{self, AssetInfo, AssetType, DirectoryNode};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

/// Payload for the per-project `fs-change-{project_id}` event.
#[derive(Debug, Clone, Serialize)]
pub struct FsChangeEvent {
    /// Assets that were added or modified (frontend merges by `path`).
    pub updated: Vec<AssetInfo>,
    /// Paths that were deleted.
    pub removed: Vec<String>,
    /// Freshly rebuilt directory tree (Plan A: full rebuild per event batch).
    pub directory_tree: DirectoryNode,
    pub total_count: usize,
    pub total_size: u64,
    pub type_counts: HashMap<String, usize>,
}

/// Holds the debouncer handle. Dropping this stops the watcher.
pub struct ProjectWatcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, FileIdMap>,
}

/// Start watching `root_path` for this project. Spawns a background thread that
/// forwards coalesced events as `fs-change-{project_id}` via `app.emit`.
pub fn start(
    app: AppHandle,
    project_id: String,
    root_path: String,
) -> Result<ProjectWatcher, String> {
    let root_buf = PathBuf::from(&root_path);
    if !root_buf.exists() {
        return Err(format!("Path does not exist: {}", root_path));
    }

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();

    let mut debouncer = new_debouncer(
        DEBOUNCE_WINDOW,
        None,
        move |result: DebounceEventResult| {
            let _ = tx.send(result);
        },
    )
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    debouncer
        .watcher()
        .watch(&root_buf, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch path: {}", e))?;

    let thread_project_id = project_id.clone();
    let thread_root = root_buf.clone();
    let event_name = format!("fs-change-{}", project_id);

    thread::spawn(move || {
        // rx closes when the Debouncer is dropped; the loop exits cleanly.
        while let Ok(result) = rx.recv() {
            let events = match result {
                Ok(ev) => ev,
                Err(errors) => {
                    eprintln!(
                        "[watcher {}] errors from notify: {:?}",
                        thread_project_id, errors
                    );
                    continue;
                }
            };

            let mut candidates: HashSet<PathBuf> = HashSet::new();
            for event in events {
                for path in &event.event.paths {
                    candidates.insert(path.clone());
                }
            }

            let filtered: Vec<PathBuf> = candidates
                .into_iter()
                .filter(|p| is_trackable_path(p, &thread_root))
                .collect();

            if filtered.is_empty() {
                continue;
            }

            let payload = apply_changes(&thread_project_id, &filtered);

            if let Ok(ev) = payload {
                let _ = app.emit(&event_name, &ev);
            }
        }
    });

    Ok(ProjectWatcher {
        _debouncer: debouncer,
    })
}

/// Apply a batch of candidate paths to the project's cached scan result.
///
/// For each path:
/// - if it exists and parses → add or replace in `cached_scan.assets`
/// - if it doesn't exist and was previously tracked → remove
///
/// Returns an `FsChangeEvent` describing the net change, or `Err` if nothing
/// ended up changing or the project had no cached scan yet.
fn apply_changes(project_id: &str, candidates: &[PathBuf]) -> Result<FsChangeEvent, String> {
    project::with_mut(project_id, |state| {
        let scan_result = state
            .cached_scan
            .as_mut()
            .ok_or_else(|| "No cached scan to patch".to_string())?;

        let project_type = scan_result.project_type.clone();

        let mut path_to_idx: HashMap<String, usize> = scan_result
            .assets
            .iter()
            .enumerate()
            .map(|(i, a)| (a.path.clone(), i))
            .collect();

        let mut updated: Vec<AssetInfo> = Vec::new();
        let mut removed: Vec<String> = Vec::new();
        let mut removed_indices: Vec<usize> = Vec::new();

        for path in candidates {
            // Must match the normalization scanner.rs uses for AssetInfo.path;
            // otherwise HashMap lookups miss on Windows (backslash vs forward).
            let path_str = scanner::path_to_string(path);

            if path.is_file() {
                if let Some(asset) = scanner::parse_asset_file(path, &project_type) {
                    if let Some(&idx) = path_to_idx.get(&path_str) {
                        scan_result.assets[idx] = asset.clone();
                    } else {
                        scan_result.assets.push(asset.clone());
                        path_to_idx.insert(path_str.clone(), scan_result.assets.len() - 1);
                    }
                    updated.push(asset);
                }
            } else if !path.exists() {
                if let Some(&idx) = path_to_idx.get(&path_str) {
                    removed_indices.push(idx);
                    removed.push(path_str);
                }
            }
            // else: path exists but is a directory (mkdir event) — nothing to track.
        }

        if updated.is_empty() && removed.is_empty() {
            return Err("No effective changes".to_string());
        }

        removed_indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in removed_indices {
            scan_result.assets.swap_remove(idx);
        }

        scan_result
            .assets
            .sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));

        scan_result.total_count = scan_result.assets.len();
        scan_result.total_size = scan_result.assets.iter().map(|a| a.size).sum();

        let mut type_counts: HashMap<String, usize> = HashMap::new();
        for asset in &scan_result.assets {
            let type_key = asset_type_key(&asset.asset_type);
            *type_counts.entry(type_key).or_insert(0) += 1;
        }
        scan_result.type_counts = type_counts.clone();

        let new_tree = scanner::build_directory_tree(
            Path::new(&scan_result.root_path),
            &scan_result.assets,
        );
        scan_result.directory_tree = new_tree.clone();

        Ok(FsChangeEvent {
            updated,
            removed,
            directory_tree: new_tree,
            total_count: scan_result.total_count,
            total_size: scan_result.total_size,
            type_counts,
        })
    })
}

/// Mirrors the scanner's discovery filters: skip hidden path components (e.g.
/// `.git/`, `.vscode/`), `.meta` sidecars, and files without an extension.
fn is_trackable_path(path: &Path, root: &Path) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return false,
    };

    for component in rel.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.starts_with('.') {
            return false;
        }
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if file_name.is_empty() || file_name.starts_with('.') || file_name.ends_with(".meta") {
        return false;
    }

    path.extension().is_some()
}

fn asset_type_key(t: &AssetType) -> String {
    match t {
        AssetType::Texture => "texture",
        AssetType::Model => "model",
        AssetType::Audio => "audio",
        AssetType::Video => "video",
        AssetType::Animation => "animation",
        AssetType::Material => "material",
        AssetType::Prefab => "prefab",
        AssetType::Scene => "scene",
        AssetType::Script => "script",
        AssetType::Data => "data",
        AssetType::Other => "other",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackable_skips_hidden_components() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(
            Path::new("/proj/.git/HEAD"),
            root
        ));
        assert!(!is_trackable_path(
            Path::new("/proj/sub/.hidden/file.png"),
            root
        ));
    }

    #[test]
    fn trackable_skips_meta_and_dotfiles() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(
            Path::new("/proj/foo.png.meta"),
            root
        ));
        assert!(!is_trackable_path(Path::new("/proj/.env"), root));
    }

    #[test]
    fn trackable_requires_extension() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(Path::new("/proj/Makefile"), root));
        assert!(is_trackable_path(Path::new("/proj/sub/foo.png"), root));
    }

    #[test]
    fn trackable_rejects_outside_root() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(Path::new("/other/foo.png"), root));
    }

    #[test]
    fn asset_type_key_matches_scanner_buckets() {
        assert_eq!(asset_type_key(&AssetType::Texture), "texture");
        assert_eq!(asset_type_key(&AssetType::Model), "model");
        assert_eq!(asset_type_key(&AssetType::Other), "other");
    }
}
