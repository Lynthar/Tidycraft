//! Per-project backend state. Each project the frontend opens is registered
//! under a unique id with its own scan state, git manager, undo history and
//! tags.

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
    /// Mirrors the project_id key in the registry. Not currently read.
    #[allow(dead_code)]
    pub id: String,
    pub root_path: String,
    pub scan_state: Option<Arc<ScanState>>,
    pub cached_scan: Option<ScanResult>,
    pub git_manager: Option<GitManager>,
    pub undo_manager: UndoManager,
    pub tags_data: Option<TagsData>,
    /// Whether the most recent scan honored `.gitignore` / `.ignore`. The
    /// watcher applies the same exclusions. Defaults to true until the first
    /// scan overwrites it.
    pub respect_gitignore: bool,
    /// Live filesystem watcher. Dropping this stops the background watch.
    pub watcher: Option<ProjectWatcher>,
    /// Rules from the most recent AI-learning run, staged in memory until the
    /// review panel's Save writes them to `tidycraft.ai.toml`. Replaced
    /// wholesale by the next run; dropped with the project state.
    pub pending_ai_rules: Option<crate::llm::rule_store::AiRulesDoc>,
    /// GUID → package-asset index from `Library/PackageCache`, cached with the
    /// sorted package-dir listing it was built for. Built lazily by
    /// `lib.rs::package_index_for`.
    pub package_index: Option<(Vec<String>, Arc<crate::unity::PackageGuidIndex>)>,
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
            respect_gitignore: true,
            watcher: None,
            pending_ai_rules: None,
            package_index: None,
        }
    }

    /// Tags, loaded from disk on first use and kept in memory after. Anything
    /// that migrates a binding must call this unconditionally; only deleting
    /// bindings may stay lazy.
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

/// Register (or look up) a project. An existing id with the same path returns
/// the existing entry; a different path rebuilds the whole state, because that
/// id now names a different project.
pub fn register(project_id: String, root_path: String) -> Arc<Mutex<ProjectState>> {
    // Nothing slow may run under the global registry lock: `ProjectState::new`
    // reads the undo history off disk, and `entry.lock()` can wait seconds on a
    // scan or a watcher batch.
    let existing = registry().lock().get(&project_id).cloned();
    let entry = match existing {
        Some(e) => e,
        None => {
            // Built outside the lock; a concurrent registration of the same id wins.
            let fresh = Arc::new(Mutex::new(ProjectState::new(
                project_id.clone(),
                root_path.clone(),
            )));
            let mut map = registry().lock();
            map.entry(project_id.clone()).or_insert(fresh).clone()
        }
    };

    // Registry lock is released by here — only the project lock is taken.
    let needs_rebuild = entry.lock().root_path != root_path;
    if needs_rebuild {
        // Constructed outside the lock: it reads the undo history off disk.
        let fresh = ProjectState::new(project_id, root_path.clone());
        let mut state = entry.lock();
        // Another caller may have rebuilt the same id meanwhile; first one wins.
        if state.root_path != root_path {
            // A scan on the old path would come back and overwrite the new state.
            if let Some(scan) = state.scan_state.as_ref() {
                scan.cancel();
            }
            *state = fresh;
        }
    }
    entry
}

/// Drop a project's state, cancelling any scan still running for it. The
/// `remove` releases the registry lock before the project lock is taken,
/// matching the registry → project order `with_mut` uses.
pub fn unregister(project_id: &str) -> bool {
    let removed = registry().lock().remove(project_id);
    let Some(entry) = removed else {
        return false;
    };
    if let Some(scan) = entry.lock().scan_state.as_ref() {
        scan.cancel();
    }
    true
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn s(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    /// A different path means a different project: the old tags, cached scan
    /// and watcher must not carry over.
    #[test]
    fn registering_a_new_path_under_the_same_id_rebuilds_the_state() {
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        let id = "relocate_test_project".to_string();

        // Old project: load tags, add a binding, install a cached scan.
        fs::write(old.path().join("hero.png"), b"x").unwrap();
        register(id.clone(), s(old.path()));
        with_mut(&id, |st| {
            let tag = st.ensure_tags().create_tag("wip".into(), "#ff0000".into());
            st.ensure_tags().add_tag_to_asset("hero.png", &tag.id);
            st.cached_scan = Some(
                crate::scanner::scan_directory_with_state(&s(old.path()), None, true).unwrap(),
            );
            st.respect_gitignore = false;
            Ok(())
        })
        .unwrap();

        // Same id, new path.
        register(id.clone(), s(new.path()));

        with_ref(&id, |st| {
            assert_eq!(
                st.root_path,
                s(new.path()),
                "root path follows the new folder"
            );
            assert!(st.cached_scan.is_none(), "the old scan must not survive");
            assert!(st.tags_data.is_none(), "the old tags must not survive");
            // A unit test cannot install a watcher, so this documents intent only.
            assert!(st.watcher.is_none(), "the old watcher must not survive");
            assert!(
                st.respect_gitignore,
                "a rebuilt state is back at its defaults"
            );
            Ok(())
        })
        .unwrap();

        // Re-loading tags at the new root yields nothing.
        with_mut(&id, |st| {
            assert!(
                st.ensure_tags().get_asset_tags("hero.png").is_empty(),
                "the old project's binding must not reach the new path"
            );
            Ok(())
        })
        .unwrap();

        unregister(&id);
    }

    /// The premise `scan_project_incremental`'s epilogue is fenced on: after a
    /// rebuild, the scan handle a running scan holds is no longer the one in the
    /// state, so `Arc::ptr_eq` tells "my scan" from "the scan that replaced me".
    /// If a rebuild ever carried `scan_state` over, that fence would pass for a
    /// scan it was written to stop, and the epilogue would strip a live scan of
    /// its cancel handle — silently, which is why this premise is pinned here.
    #[test]
    fn a_rebuild_leaves_a_running_scans_handle_no_longer_installed() {
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        let id = "rebuild_scan_handle_project".to_string();

        register(id.clone(), s(old.path()));
        let mine = Arc::new(crate::scanner::ScanState::new());
        with_mut(&id, |st| {
            st.scan_state = Some(mine.clone());
            Ok(())
        })
        .unwrap();

        register(id.clone(), s(new.path()));

        with_ref(&id, |st| {
            assert!(
                !st.scan_state
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &mine)),
                "the pre-rebuild scan handle must not still be the installed one"
            );
            Ok(())
        })
        .unwrap();
        assert!(
            mine.is_cancelled(),
            "the rebuild cancels the scan it orphans"
        );

        unregister(&id);
    }

    /// Re-registering the same path is idempotent.
    #[test]
    fn registering_the_same_path_twice_keeps_the_state() {
        let dir = tempdir().unwrap();
        let id = "idempotent_test_project".to_string();

        register(id.clone(), s(dir.path()));
        with_mut(&id, |st| {
            st.cached_scan = Some(
                crate::scanner::scan_directory_with_state(&s(dir.path()), None, true).unwrap(),
            );
            Ok(())
        })
        .unwrap();

        register(id.clone(), s(dir.path()));

        with_ref(&id, |st| {
            assert!(
                st.cached_scan.is_some(),
                "re-registering the same path must not throw away the scan"
            );
            Ok(())
        })
        .unwrap();

        unregister(&id);
    }

    /// Closing a project must stop its scan: `scan_project_incremental` holds
    /// its own clone of the `Arc<ScanState>`, so once the registry entry is
    /// gone the cancel flag is unreachable.
    #[test]
    fn unregister_cancels_an_in_flight_scan() {
        let dir = tempdir().unwrap();
        let id = "test_unregister_cancels_in_flight_scan";
        register(id.to_string(), dir.path().to_string_lossy().to_string());

        // Stand in for the scan's own handle on the state.
        let scan = Arc::new(ScanState::new());
        with_mut(id, |s| {
            s.scan_state = Some(scan.clone());
            Ok(())
        })
        .unwrap();
        assert!(!scan.is_cancelled());

        assert!(unregister(id));
        assert!(
            scan.is_cancelled(),
            "closing the project left its scan running"
        );
    }

    #[test]
    fn unregister_reports_whether_it_removed_anything() {
        let dir = tempdir().unwrap();
        let id = "test_unregister_return_value";
        assert!(!unregister(id), "nothing registered under this id yet");
        register(id.to_string(), dir.path().to_string_lossy().to_string());
        assert!(unregister(id));
        assert!(!unregister(id), "second unregister has nothing to remove");
    }
}
