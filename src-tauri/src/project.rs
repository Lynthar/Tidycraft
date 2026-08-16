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
    /// Whether the most recent scan honored `.gitignore` / `.ignore` (the
    /// per-machine "Respect .gitignore" setting). Recorded on each scan so
    /// the filesystem watcher applies the same exclusions and doesn't re-add
    /// scan-excluded files on FS events. Defaults to true (matches the
    /// frontend default) until the first scan overwrites it.
    pub respect_gitignore: bool,
    /// Live filesystem watcher. Dropping this stops the background watch.
    pub watcher: Option<ProjectWatcher>,
    /// Rules from the most recent AI-learning run, staged in memory until the
    /// user confirms them in the review panel. `save_ai_rules` takes this and
    /// writes it to `tidycraft.ai.toml` — the single commit point — so closing
    /// the panel without saving leaves the project untouched and the rules
    /// never reach `suggest_tags`. Replaced wholesale by the next learning
    /// run; dropped with the project state (an unsaved run does not survive
    /// closing the project — accepted trade-off of review-before-commit).
    pub pending_ai_rules: Option<crate::llm::rule_store::AiRulesDoc>,
    /// GUID → package-asset index from `Library/PackageCache`, cached with
    /// the key it was built for (the sorted package-dir listing — those dirs
    /// are immutable, so the listing changing is the only staleness signal).
    /// Built lazily by `lib.rs::package_index_for`; `None` until first use.
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

    /// Tags, loaded from disk on first use and kept in memory after.
    ///
    /// Anything that *migrates* a binding — rename, move, undo, the watcher's
    /// rename pairing — must call this unconditionally rather than skipping the
    /// work when `tags_data` is still `None`. Skipping leaves the binding on a
    /// path that no longer exists, and the watcher's orphan cleanup deletes it
    /// the moment something else loads the file. Whether the tag panel happened
    /// to be opened first is not something tag preservation may depend on, and
    /// the load it costs is one small file, once per session.
    ///
    /// Deleting bindings is the one case that may stay lazy: an orphan nobody
    /// has loaded sits harmlessly on disk, so reading the file purely to strip
    /// it is churn (watcher.rs makes that distinction explicitly).
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

/// 注册(或查出)一个项目。id 已存在且路径相同时返回既有条目,这是每次
/// `openProject` 都会走的幂等路径。
///
/// **路径不同则整体重建**:同一个 id 指向另一个文件夹,意味着它现在是另一个
/// 项目。曾经这里只改 `root_path` 一个字段,于是旧项目的 undo 历史、标签、
/// watcher 和缓存扫描会留给新路径——那个分支当时不可达(前端按路径查已有
/// 条目,id 相同必然路径相同),重新定位是第一个走进去的调用者。
pub fn register(project_id: String, root_path: String) -> Arc<Mutex<ProjectState>> {
    // The registry lock is global — every project-scoped command passes through
    // it — so nothing slow may happen while it is held. Two things here are
    // slow: `ProjectState::new` reads the persisted undo history off disk, and
    // `entry.lock()` waits on the *project* lock, which a scan, a watcher batch
    // or a dependency-graph rebuild can hold for seconds. Doing either under the
    // registry lock made one busy project stall commands for all of them,
    // cancel_scan included.
    let existing = registry().lock().get(&project_id).cloned();
    let entry = match existing {
        Some(e) => e,
        None => {
            // Built outside the lock; if another thread registered the same id
            // meanwhile, `or_insert` keeps theirs and this one is dropped.
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
        // 锁外构造:`ProjectState::new` 要读一次 undo 历史,而项目锁里不放慢活
        // 是全仓的规矩(注册表锁那段同理)。
        let fresh = ProjectState::new(project_id, root_path.clone());
        let mut state = entry.lock();
        // 锁外构造的这段时间,另一个并发调用可能已经把这个 id 重建到了同一个
        // 新路径——不重新核对就覆盖,会把那次构造(同样读了一遍 undo 历史)
        // 出来的状态悄悄丢掉。这里落空就让 fresh 直接被丢弃,谁先到就用谁的。
        if state.root_path != root_path {
            // 旧路径上的扫描还在跑就取消它:它的结果属于一个这个 id 已经不再指向
            // 的文件夹,回来只会覆盖新项目的状态。
            if let Some(scan) = state.scan_state.as_ref() {
                scan.cancel();
            }
            *state = fresh;
        }
    }
    entry
}

/// Drop a project's state, cancelling any scan still running for it.
///
/// The cancel is the whole reason this isn't a one-line `remove`: the scan
/// holds its own clone of the `Arc<ScanState>` for its entire run, so after
/// the registry entry is gone nothing can reach the flag and the walk
/// finishes at its own pace — minutes, on a project big enough for the user
/// to have given up and closed it. The scan then returns `Cancelled`, whose
/// error path is a no-op in the frontend because the closed project is
/// already out of the projects Map.
///
/// The `remove` releases the registry lock before the project lock is taken
/// (registry → project is the order `with_mut` uses; taking them the other
/// way round would stall every project on one busy one).
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

    // Neither test below exercises the TOCTOU re-check in the rebuild branch
    // (`if state.root_path != root_path` after the lock-free construction) —
    // both drive `register` from a single thread, so they pass whether or not
    // that re-check is there. Proving it would need a barrier injected into
    // production code to force a second `register` call to land mid-construction;
    // this note exists so the two green tests below aren't mistaken for coverage.

    /// 换路径 = 这个 id 现在是另一个项目。只换 `root_path` 会把旧项目的标签、
    /// 缓存扫描和 watcher 留给新路径,于是新项目一开就带着别人的标签。
    #[test]
    fn registering_a_new_path_under_the_same_id_rebuilds_the_state() {
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        let id = "relocate_test_project".to_string();

        // 旧项目:载入标签、加一条绑定、装一份缓存扫描。
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

        // 同一个 id、新路径。
        register(id.clone(), s(new.path()));

        with_ref(&id, |st| {
            assert_eq!(
                st.root_path,
                s(new.path()),
                "root path follows the new folder"
            );
            assert!(st.cached_scan.is_none(), "the old scan must not survive");
            assert!(st.tags_data.is_none(), "the old tags must not survive");
            // Documents intent, not a guarantee: installing a watcher needs an
            // `AppHandle`, which a unit test has no way to provide, so this field
            // is `None` whether or not the rebuild actually drops the old one.
            assert!(st.watcher.is_none(), "the old watcher must not survive");
            assert!(
                st.respect_gitignore,
                "a rebuilt state is back at its defaults"
            );
            Ok(())
        })
        .unwrap();

        // 新根上重新载入标签,拿到的是空的——旧绑定没有跟过来。
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

    /// 同一个路径重复注册仍然是幂等的——这是每次 openProject 都会走的路。
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

    /// Closing a project has to stop its scan, not just forget about it.
    /// `scan_project_incremental` clones the `Arc<ScanState>` before it starts
    /// and holds that clone for the whole run, so once the registry entry is
    /// gone the cancel flag is unreachable — a big project would keep walking
    /// the disk with nobody left to want the result.
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
