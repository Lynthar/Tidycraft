//! Per-project backend state.
//!
//! Replaces the previous global `Mutex<Option<...>>` singletons in `lib.rs`.
//! Each project the frontend opens is registered here with a unique id and gets
//! its own `ScanState`, `ScanResult`, `GitManager`, `UndoManager`, and `TagsData`.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::git::GitManager;
use crate::scanner::{ScanResult, ScanState};
use crate::tags::TagsData;
use crate::undo::UndoManager;
use crate::watcher::ProjectWatcher;

pub struct ProjectState {
    /// Mirrors the project_id key in the registry. Kept on the state itself
    /// so future emit-from-inside-state code paths don't need the key
    /// passed through; not currently read.
    #[allow(dead_code)]
    pub id: String,
    pub root_path: String,
    pub scan_state: Option<Arc<ScanState>>,
    pub cached_scan: Option<ScanResult>,
    pub git_manager: Option<GitManager>,
    pub undo_manager: UndoManager,
    pub tags_data: Option<TagsData>,
    /// Live filesystem watcher. Dropping this stops the background watch.
    pub watcher: Option<ProjectWatcher>,
}

impl ProjectState {
    pub fn new(id: String, root_path: String) -> Self {
        // Load cross-session undo history keyed by the project's root path.
        let undo_manager = UndoManager::load_for_project(Path::new(&root_path), 50);
        Self {
            id,
            root_path,
            scan_state: None,
            cached_scan: None,
            git_manager: None,
            undo_manager,
            tags_data: None,
            watcher: None,
        }
    }

    pub fn ensure_tags(&mut self) -> &mut TagsData {
        if self.tags_data.is_none() {
            self.tags_data = Some(TagsData::load(Path::new(&self.root_path)));
        }
        self.tags_data.as_mut().expect("tags_data just initialized")
    }

    pub fn save_tags(&self) -> Result<(), String> {
        if let Some(ref tags) = self.tags_data {
            tags.save(Path::new(&self.root_path))?;
        }
        Ok(())
    }

    pub fn require_scan(&self) -> Result<&ScanResult, String> {
        self.cached_scan
            .as_ref()
            .ok_or_else(|| "No scan result available. Please scan the project first.".to_string())
    }
}

type ProjectMap = HashMap<String, Arc<Mutex<ProjectState>>>;

static REGISTRY: OnceLock<Mutex<ProjectMap>> = OnceLock::new();

fn registry() -> &'static Mutex<ProjectMap> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register (or look up) a project. If the id is already registered, the
/// existing entry is returned and `root_path` is ignored — callers can rely on
/// idempotency. If `root_path` differs from what's stored, the stored value is
/// updated (handles the case where the frontend re-opens the same project at a
/// different path).
pub fn register(project_id: String, root_path: String) -> Arc<Mutex<ProjectState>> {
    let mut map = registry().lock();
    let entry = map
        .entry(project_id.clone())
        .or_insert_with(|| Arc::new(Mutex::new(ProjectState::new(project_id, root_path.clone()))))
        .clone();
    {
        let mut state = entry.lock();
        if state.root_path != root_path {
            state.root_path = root_path;
        }
    }
    entry
}

pub fn unregister(project_id: &str) -> bool {
    let mut map = registry().lock();
    map.remove(project_id).is_some()
}

pub fn get(project_id: &str) -> Option<Arc<Mutex<ProjectState>>> {
    registry().lock().get(project_id).cloned()
}

/// Run a closure with mutable access to a project's state.
pub fn with_mut<F, R>(project_id: &str, f: F) -> Result<R, String>
where
    F: FnOnce(&mut ProjectState) -> Result<R, String>,
{
    let proj = get(project_id).ok_or_else(|| format!("Project not registered: {}", project_id))?;
    let mut state = proj.lock();
    f(&mut state)
}

/// Run a closure with read access to a project's state.
pub fn with_ref<F, R>(project_id: &str, f: F) -> Result<R, String>
where
    F: FnOnce(&ProjectState) -> Result<R, String>,
{
    let proj = get(project_id).ok_or_else(|| format!("Project not registered: {}", project_id))?;
    let state = proj.lock();
    f(&state)
}
