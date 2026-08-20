//! Per-project filesystem watcher. `notify-debouncer-full` coalesces OS events,
//! affected files are re-parsed, and one `fs-change-{project_id}` Tauri event is
//! emitted per debounce window. Dropping `ProjectState.watcher` tears it down.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecursiveMode, Watcher as _};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::project;
use crate::scanner::{self, AssetInfo, AssetType, DirectoryNode, ProjectType};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

/// One external rename the watcher recognized: a stitched pair from the
/// debouncer or a remove+create joined by identity. The scan delta still travels
/// as `removed` (old path) + `updated` (new entry); `renamed` is additive.
#[derive(Debug, Clone, Serialize)]
pub struct RenamedPair {
    pub from: String,
    pub to: String,
    pub is_dir: bool,
}

/// Payload for the per-project `fs-change-{project_id}` event.
#[derive(Debug, Clone, Serialize)]
pub struct FsChangeEvent {
    /// Assets that were added or modified (frontend merges by `path`).
    pub updated: Vec<AssetInfo>,
    /// Paths that were deleted.
    pub removed: Vec<String>,
    /// Renames recognized in this batch (see [`RenamedPair`]).
    pub renamed: Vec<RenamedPair>,
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
    respect_gitignore: bool,
) -> Result<ProjectWatcher, String> {
    let root_buf = PathBuf::from(&root_path);
    if !root_buf.exists() {
        return Err(format!("Path does not exist: {}", root_path));
    }

    // Built once at watcher start so FS events apply the same `.gitignore`
    // exclusions the scan used. `None` when the project scanned with gitignore
    // off.
    let ignore_matcher = scanner::build_gitignore_matcher(&root_buf, respect_gitignore);

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();

    let mut debouncer = new_debouncer(DEBOUNCE_WINDOW, None, move |result: DebounceEventResult| {
        let _ = tx.send(result);
    })
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    debouncer
        .watcher()
        .watch(&root_buf, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch path: {}", e))?;

    // Populate the debouncer's file-id cache for everything already under the
    // root: on Windows and macOS the backends attach no tracker, so rename
    // stitching can only match halves by file id, and 0.3.x fills it here.
    debouncer
        .cache()
        .add_root(root_buf.clone(), RecursiveMode::Recursive);

    let thread_project_id = project_id.clone();
    let thread_root = root_buf.clone();
    let event_name = format!("fs-change-{}", project_id);

    thread::spawn(move || {
        // Cumulative across this watcher's lifetime; the frontend replaces
        // its entry per event, so the latest one always carries the total.
        let mut dropped_batches: usize = 0;
        // rx closes when the Debouncer is dropped; the loop exits cleanly.
        while let Ok(result) = rx.recv() {
            let events = match result {
                Ok(ev) => ev,
                Err(errors) => {
                    eprintln!(
                        "[watcher {}] errors from notify: {:?}",
                        thread_project_id, errors
                    );
                    dropped_batches += 1;
                    let detail = errors.first().map(|e| e.to_string()).unwrap_or_default();
                    crate::warning::emit_project_warning(
                        &app,
                        &thread_project_id,
                        &crate::warning::ProjectWarning::WatcherEventsDropped {
                            batches: dropped_batches,
                            detail,
                        },
                    );
                    continue;
                }
            };

            // Stitched rename pairs are handled as renames (tags follow, the
            // scan re-keys); every other event contributes bare paths to the
            // existence-driven pipeline below.
            let (raw_pairs, mut single_paths) = split_batch(&events);

            let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
            for (from, to) in raw_pairs {
                match route_pair(&from, &to, &thread_root) {
                    PairRoute::Rename => pairs.push((from, to)),
                    PairRoute::Fallback(paths) => single_paths.extend(paths),
                }
            }

            let mut candidates: HashSet<PathBuf> = HashSet::new();
            for path in single_paths {
                // A `.meta` change is a change to its host asset's Unity GUID —
                // remap to the host so it gets re-parsed. Godot's `.import` and
                // `.uid` get no remap: no `AssetInfo` field derives from them.
                if let Some(host) = meta_host_path(&path) {
                    candidates.insert(host);
                } else {
                    candidates.insert(path);
                }
            }

            let filtered: Vec<PathBuf> = candidates
                .into_iter()
                .filter(|p| {
                    if is_gitignored(p, &thread_root, ignore_matcher.as_ref()) {
                        return false;
                    }
                    if p.is_dir() {
                        // An existing directory event. A directory that appears
                        // wholesale is reported as ONE event on the directory
                        // path, with none for its children.
                        path_shape_trackable(p, &thread_root)
                    } else if p.exists() {
                        // Existing file: track only real asset files (extensioned).
                        is_trackable_path(p, &thread_root)
                    } else {
                        // Deletion: the path is gone. macOS coalesces a directory
                        // removal into one event on the extensionless directory
                        // path, so the extension requirement is dropped here.
                        path_shape_trackable(p, &thread_root)
                    }
                })
                .collect();

            if filtered.is_empty() && pairs.is_empty() {
                continue;
            }

            let mut op_warnings = Vec::new();
            let payload = apply_changes(
                &thread_project_id,
                &pairs,
                &filtered,
                &thread_root,
                ignore_matcher.as_ref(),
                &mut op_warnings,
            );

            if let Ok(ev) = payload {
                let _ = app.emit(&event_name, &ev);
            }
            for w in &op_warnings {
                crate::warning::emit_project_warning(&app, &thread_project_id, w);
            }
        }
    });

    Ok(ProjectWatcher {
        _debouncer: debouncer,
    })
}

/// Split a debounced batch into stitched rename pairs and plain single-path
/// candidates. Only `Modify(Name(RenameMode::Both))` with exactly two paths is a
/// pair; everything else feeds the existence-driven pipeline.
fn split_batch(events: &[DebouncedEvent]) -> (Vec<(PathBuf, PathBuf)>, Vec<PathBuf>) {
    let mut pairs = Vec::new();
    let mut singles = Vec::new();
    for ev in events {
        let e = &ev.event;
        match &e.kind {
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if e.paths.len() == 2 => {
                pairs.push((e.paths[0].clone(), e.paths[1].clone()));
            }
            _ => singles.extend(e.paths.iter().cloned()),
        }
    }
    (pairs, singles)
}

/// Where one stitched rename pair goes.
enum PairRoute {
    /// Track as a rename: tags migrate, the scan re-keys.
    Rename,
    /// Not a rename for our purposes — process these paths (possibly none)
    /// through the single-path pipeline instead.
    Fallback(Vec<PathBuf>),
}

/// Route a stitched pair. Pure computation, no syscalls; the filesystem probing
/// happens later in `apply_changes`.
fn route_pair(from: &Path, to: &Path, root: &Path) -> PairRoute {
    if from == to {
        // Same-string pair — nothing changed that can be expressed.
        return PairRoute::Fallback(Vec::new());
    }
    let is_sidecar = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(crate::sidecar::is_sidecar_name)
    };
    if is_sidecar(from) || is_sidecar(to) {
        // An engine renamed a sidecar. Sidecars are not assets, but a `.meta`
        // To-side must still refresh its host's guid, which the single-path
        // remap does — route both halves there.
        return PairRoute::Fallback(vec![from.to_path_buf(), to.to_path_buf()]);
    }
    if !path_shape_trackable(from, root) && !path_shape_trackable(to, root) {
        // Entirely between untracked shapes (`.git` lockfile churn and the
        // like) — nothing we track or tag can be affected.
        return PairRoute::Fallback(Vec::new());
    }
    PairRoute::Rename
}

/// A rename pair resolved against the filesystem (probe phase, no lock held).
struct ProbedPair {
    /// Normalized old path (tag key / scan key).
    from: String,
    from_path: PathBuf,
    /// Normalized new path.
    to: String,
    to_is_dir: bool,
    /// New path passes shape + gitignore — it belongs in the scan.
    to_visible: bool,
    /// The old path is occupied again by flush time — the safe-save pattern. The
    /// rename is physically real, but the tag belongs to the role at the old
    /// path, which is alive again.
    from_reoccupied: bool,
    /// Scan-side payload for a visible file target: re-keyed from the cached
    /// entry when the extension is unchanged, else freshly parsed.
    parsed_to: Option<AssetInfo>,
}

/// What one single-path candidate turned out to be on probing.
enum Probe {
    /// Existing file that parsed. Boxed: `AssetInfo` dwarfs the other
    /// variants and probes travel in a per-batch `Vec`.
    Parsed(Box<AssetInfo>),
    /// Existing directory — its trackable descendants, parsed. A directory that
    /// appears wholesale delivers ONE event on the directory path and none for
    /// its children, so they must be listed here.
    DirListing(Vec<AssetInfo>),
    /// Path is gone — remove every tracked asset at or under it.
    Vanished(PathBuf),
    /// Exists but nothing to track (unparseable file, empty directory).
    Nothing,
}

/// Recursively parse the trackable assets under `dir`, applying the same filters
/// the watcher applies to individual events. Depth-bounded: `read_dir` +
/// `is_dir` follow symlinks, so a link loop would otherwise recurse forever.
fn collect_dir_assets(
    dir: &Path,
    root: &Path,
    matcher: Option<&scanner::IgnoreMatcher>,
    project_type: &Option<ProjectType>,
    depth: u32,
    out: &mut Vec<AssetInfo>,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_gitignored(&path, root, matcher) {
            continue;
        }
        if path.is_dir() {
            if path_shape_trackable(&path, root) {
                collect_dir_assets(&path, root, matcher, project_type, depth - 1, out);
            }
        } else if is_trackable_path(&path, root) {
            if let Some(info) = scanner::parse_asset_file(&path, project_type) {
                out.push(info);
            }
        }
    }
}

/// One vanished asset's identity triple, captured before it left the scan.
struct OrphanInfo {
    path: String,
    name: String,
    size: u64,
    modified: u64,
}

struct OrphanMatch {
    /// `(from, to)` — vanished path paired 1:1 with an appeared path.
    pairs: Vec<(String, String)>,
    /// Vanished paths whose identity matched appeared files ambiguously:
    /// don't migrate their tags, but don't reap them either — rename the file
    /// back and the binding is still there.
    preserved: HashSet<String>,
}

/// Pair vanished assets with appeared ones by identity triple `(file name, size,
/// mtime)` — the shape a cross-directory move leaves on Windows. Only bijective
/// (1 vanished ↔ 1 appeared) triples pair up; ambiguity would misassign tags.
fn match_orphans(removed: &[OrphanInfo], added: &[OrphanInfo]) -> OrphanMatch {
    type Triple = (String, u64, u64);
    let mut by_triple: HashMap<Triple, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for (i, r) in removed.iter().enumerate() {
        by_triple
            .entry((r.name.clone(), r.size, r.modified))
            .or_default()
            .0
            .push(i);
    }
    for (i, a) in added.iter().enumerate() {
        by_triple
            .entry((a.name.clone(), a.size, a.modified))
            .or_default()
            .1
            .push(i);
    }

    let mut result = OrphanMatch {
        pairs: Vec::new(),
        preserved: HashSet::new(),
    };
    for (rs, adds) in by_triple.into_values() {
        if rs.is_empty() || adds.is_empty() {
            // Plain removals (reap candidates) or plain additions — no match.
            continue;
        }
        if rs.len() == 1 && adds.len() == 1 {
            result
                .pairs
                .push((removed[rs[0]].path.clone(), added[adds[0]].path.clone()));
        } else {
            for r in rs {
                result.preserved.insert(removed[r].path.clone());
            }
        }
    }
    result
}

/// A tag-binding move `apply_changes` owes the tags file.
enum TagMigration {
    File { from: String, to: String },
    DirPrefix { from: String, to: String },
}

/// Physically drop `assets[idx]`, keeping `path_to_idx` consistent —
/// `swap_remove` relocates the former last element into the hole.
fn swap_remove_asset(
    assets: &mut Vec<AssetInfo>,
    path_to_idx: &mut HashMap<String, usize>,
    idx: usize,
) {
    let removed = assets.swap_remove(idx);
    path_to_idx.remove(&removed.path);
    if idx < assets.len() {
        path_to_idx.insert(assets[idx].path.clone(), idx);
    }
}

/// Add-or-replace one parsed asset in the scan, recording it in `updated`
/// (frontend merge payload) and — when the path is genuinely new — in `added`
/// (identity pool for `match_orphans`).
fn upsert_asset(
    assets: &mut Vec<AssetInfo>,
    path_to_idx: &mut HashMap<String, usize>,
    updated: &mut Vec<AssetInfo>,
    added: &mut Vec<OrphanInfo>,
    info: &AssetInfo,
) {
    if let Some(&idx) = path_to_idx.get(&info.path) {
        assets[idx] = info.clone();
    } else {
        added.push(OrphanInfo {
            path: info.path.clone(),
            name: info.name.clone(),
            size: info.size,
            modified: info.modified,
        });
        path_to_idx.insert(info.path.clone(), assets.len());
        assets.push(info.clone());
    }
    updated.push(info.clone());
}

/// Reject a batch whose project has been re-registered against a different root
/// since the events were collected. Both sides go through `path_to_string` rather
/// than comparing raw: a separator mismatch would fail every batch, silently, and
/// this function is what stands between the old root's events and the new root's
/// scan and tags file.
fn batch_still_owns_project(state: &project::ProjectState, root_key: &str) -> Result<(), String> {
    if scanner::path_to_string(Path::new(&state.root_path)) == root_key {
        Ok(())
    } else {
        Err("project root changed under this batch".to_string())
    }
}

/// Apply a batch of stitched rename pairs plus single-path candidates to the
/// project's cached scan result. Returns an `FsChangeEvent` describing the net
/// change, or `Err` when nothing changed, the project has no cached scan, or the
/// project has moved to a different root since these events were collected.
fn apply_changes(
    project_id: &str,
    pairs: &[(PathBuf, PathBuf)],
    candidates: &[PathBuf],
    root: &Path,
    ignore_matcher: Option<&scanner::IgnoreMatcher>,
    // Out-param rather than an AppHandle: the tag-migration tests call this
    // directly and have no Tauri app. The thread loop owns the emit.
    warnings_out: &mut Vec<crate::warning::ProjectWarning>,
) -> Result<FsChangeEvent, String> {
    // The root these events were collected under. Re-checked at every lock below,
    // not just here: phase 1 parses outside the lock for as long as the batch is
    // large, which is room enough for a relocation to land between the phases.
    let root_key = scanner::path_to_string(root);

    // ---- Phase 0: snapshot what the probe needs (brief lock) -------------
    let from_keys: HashSet<String> = pairs
        .iter()
        .map(|(f, _)| scanner::path_to_string(f))
        .collect();
    let (project_type, old_infos) = project::with_ref(project_id, |state| {
        batch_still_owns_project(state, &root_key)?;
        let scan = state
            .cached_scan
            .as_ref()
            .ok_or_else(|| "No cached scan to patch".to_string())?;
        let old_infos: HashMap<String, AssetInfo> = if from_keys.is_empty() {
            HashMap::new()
        } else {
            scan.assets
                .iter()
                .filter(|a| from_keys.contains(&a.path))
                .map(|a| (a.path.clone(), a.clone()))
                .collect()
        };
        Ok((scan.project_type.clone(), old_infos))
    })?;

    // ---- Phase 1: probe + parse, outside the lock ------------------------
    // Parsing decodes image headers, model geometry and audio streams, and one
    // 500ms batch can carry hundreds — the project lock is not held for it.
    let probed_pairs: Vec<ProbedPair> = pairs
        .iter()
        .map(|(from, to)| {
            let from_str = scanner::path_to_string(from);
            let to_str = scanner::path_to_string(to);
            let to_is_dir = to.is_dir();
            let to_visible = !is_gitignored(to, root, ignore_matcher)
                && if to_is_dir {
                    path_shape_trackable(to, root)
                } else {
                    is_trackable_path(to, root)
                };
            let from_reoccupied = from.exists();
            let parsed_to = if to_visible && !to_is_dir {
                let new_ext = to
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default();
                match old_infos.get(&from_str) {
                    // Renames don't change bytes: re-key the cached entry rather
                    // than re-read the file, but only while the extension is
                    // unchanged — `metadata`'s shape belongs to the asset type.
                    Some(old) if old.extension.eq_ignore_ascii_case(&new_ext) => {
                        Some(scanner::rekey_asset_info(old, to, &project_type))
                    }
                    _ => scanner::parse_asset_file(to, &project_type),
                }
            } else {
                None
            };
            ProbedPair {
                from: from_str,
                from_path: from.clone(),
                to: to_str,
                to_is_dir,
                to_visible,
                from_reoccupied,
                parsed_to,
            }
        })
        .collect();

    let probed: Vec<Probe> = candidates
        .iter()
        .map(|path| {
            if path.is_file() {
                match scanner::parse_asset_file(path, &project_type) {
                    Some(info) => Probe::Parsed(Box::new(info)),
                    None => Probe::Nothing,
                }
            } else if path.is_dir() {
                let mut assets = Vec::new();
                collect_dir_assets(path, root, ignore_matcher, &project_type, 32, &mut assets);
                if assets.is_empty() {
                    Probe::Nothing
                } else {
                    Probe::DirListing(assets)
                }
            } else if !path.exists() {
                Probe::Vanished(path.clone())
            } else {
                Probe::Nothing
            }
        })
        .collect();

    // Tag moves owed by the stitched pairs, decided before the scan pass so they
    // happen even when the scan itself has nothing visible to change.
    let mut tag_migrations: Vec<TagMigration> = Vec::new();
    for p in &probed_pairs {
        if p.to_is_dir {
            // Tags always follow a renamed directory: the files under it moved,
            // so a binding left at the old prefix is dead. No editor does
            // directory-level backup renames, so the safe-save guard is moot.
            tag_migrations.push(TagMigration::DirPrefix {
                from: p.from.clone(),
                to: p.to.clone(),
            });
        } else if !p.from_reoccupied {
            tag_migrations.push(TagMigration::File {
                from: p.from.clone(),
                to: p.to.clone(),
            });
        }
    }

    // ---- Phase 2: patch the scan (project lock) --------------------------
    let (event, reap, tier2_pairs) = project::with_mut(project_id, |state| {
        batch_still_owns_project(state, &root_key)?;
        let scan_result = state
            .cached_scan
            .as_mut()
            .ok_or_else(|| "No cached scan to patch".to_string())?;

        let mut path_to_idx: HashMap<String, usize> = scan_result
            .assets
            .iter()
            .enumerate()
            .map(|(i, a)| (a.path.clone(), i))
            .collect();

        let mut updated: Vec<AssetInfo> = Vec::new();
        let mut payload_removed: HashSet<String> = HashSet::new();
        let mut renamed: Vec<RenamedPair> = Vec::new();
        // Identity pools for match_orphans — candidate-driven only; pair
        // re-keys are already renames and must not re-enter the matcher.
        let mut orphan_removed: Vec<OrphanInfo> = Vec::new();
        let mut added: Vec<OrphanInfo> = Vec::new();
        // Subtrees to sweep out of the scan. Pair-origin sweeps carry no
        // reap: their tag bindings have a migration recorded already.
        let mut pair_vanish: Vec<&Path> = Vec::new();
        let mut orphan_vanish: Vec<&Path> = Vec::new();

        // -- Stitched pairs first, so a fresh parse from the candidate pass
        // (a file renamed AND modified in one burst) overrides the re-key,
        // never the other way around.
        for pair in &probed_pairs {
            if pair.to_is_dir && pair.to_visible {
                // Directory rename: one OS event, zero child events. Re-key every
                // tracked descendant or the subtree falls out of the scan until
                // the next full rescan.
                for idx in 0..scan_result.assets.len() {
                    if path_within(&scan_result.assets[idx].path, &pair.from_path) {
                        let old_path = scan_result.assets[idx].path.clone();
                        // Both keys are scanner-normalized and path_within
                        // guarantees a component-boundary prefix — splice as
                        // strings.
                        let new_path = format!("{}{}", pair.to, &old_path[pair.from.len()..]);
                        scan_result.assets[idx].path = new_path.clone();
                        path_to_idx.remove(&old_path);
                        path_to_idx.insert(new_path, idx);
                        payload_removed.insert(old_path);
                        updated.push(scan_result.assets[idx].clone());
                    }
                }
                renamed.push(RenamedPair {
                    from: pair.from.clone(),
                    to: pair.to.clone(),
                    is_dir: true,
                });
            } else if let Some(new_info) = pair.parsed_to.as_ref() {
                // File rename with a visible, parseable target. Drop the old entry
                // first (and the clobbered target on a rename-over), then land the
                // new one, so the index map never aliases.
                if let Some(&idx) = path_to_idx.get(&pair.from) {
                    // Drops the map entry for `from` and re-points whatever
                    // swap_remove relocated into the hole.
                    swap_remove_asset(&mut scan_result.assets, &mut path_to_idx, idx);
                    payload_removed.insert(pair.from.clone());
                }
                if let Some(&idx) = path_to_idx.get(&new_info.path) {
                    scan_result.assets[idx] = new_info.clone();
                } else {
                    path_to_idx.insert(new_info.path.clone(), scan_result.assets.len());
                    scan_result.assets.push(new_info.clone());
                }
                updated.push(new_info.clone());
                if !pair.from_reoccupied {
                    renamed.push(RenamedPair {
                        from: pair.from.clone(),
                        to: pair.to.clone(),
                        is_dir: false,
                    });
                }
            } else {
                // Target invisible (renamed into an ignored or hidden path) or
                // gone again — the asset left the visible set. Its binding still
                // migrates and comes back if the file does.
                pair_vanish.push(&pair.from_path);
            }
        }

        // -- Existence-driven candidates, semantics unchanged.
        for probe in &probed {
            match probe {
                Probe::Parsed(info) => upsert_asset(
                    &mut scan_result.assets,
                    &mut path_to_idx,
                    &mut updated,
                    &mut added,
                    info,
                ),
                Probe::DirListing(list) => {
                    for info in list {
                        upsert_asset(
                            &mut scan_result.assets,
                            &mut path_to_idx,
                            &mut updated,
                            &mut added,
                            info,
                        );
                    }
                }
                Probe::Vanished(path) => orphan_vanish.push(path),
                Probe::Nothing => {}
            }
        }

        // -- Sweep vanished subtrees: an exact-match file, or every tracked
        // descendant when macOS coalesces a directory removal into one event on
        // the extensionless directory path.
        for (gone, is_orphan) in pair_vanish
            .iter()
            .map(|p| (*p, false))
            .chain(orphan_vanish.iter().map(|p| (*p, true)))
        {
            let victims: Vec<String> = scan_result
                .assets
                .iter()
                .filter(|a| path_within(&a.path, gone))
                .map(|a| a.path.clone())
                .collect();
            for victim in victims {
                if let Some(&idx) = path_to_idx.get(&victim) {
                    if is_orphan {
                        let a = &scan_result.assets[idx];
                        orphan_removed.push(OrphanInfo {
                            path: a.path.clone(),
                            name: a.name.clone(),
                            size: a.size,
                            modified: a.modified,
                        });
                    }
                    payload_removed.insert(victim.clone());
                    swap_remove_asset(&mut scan_result.assets, &mut path_to_idx, idx);
                }
            }
        }

        // -- Join vanished+appeared candidates that share an identity triple:
        // a cross-directory move on Windows arrives exactly like this.
        let matches = match_orphans(&orphan_removed, &added);
        for (from, to) in &matches.pairs {
            renamed.push(RenamedPair {
                from: from.clone(),
                to: to.clone(),
                is_dir: false,
            });
        }
        let matched_from: HashSet<&String> = matches.pairs.iter().map(|(f, _)| f).collect();
        let reap: Vec<String> = orphan_removed
            .iter()
            .map(|o| &o.path)
            .filter(|p| !matched_from.contains(p) && !matches.preserved.contains(*p))
            .cloned()
            .collect();

        if updated.is_empty() && payload_removed.is_empty() && renamed.is_empty() {
            // Nothing visible changed; hand back the tag work (if any) and
            // let the caller skip the emit.
            return Ok((None, reap, matches.pairs));
        }

        // The frontend merge applies `removed` before `updated`, so a path in
        // both nets out correctly. Deduplicate `updated` by path, last write
        // wins, and drop claims about paths no longer tracked.
        let mut dedup_idx: HashMap<String, usize> = HashMap::new();
        let mut deduped: Vec<AssetInfo> = Vec::new();
        for info in updated {
            if let Some(&i) = dedup_idx.get(&info.path) {
                deduped[i] = info;
            } else {
                dedup_idx.insert(info.path.clone(), deduped.len());
                deduped.push(info);
            }
        }
        deduped.retain(|a| path_to_idx.contains_key(&a.path));

        scan_result.assets.sort_by_key(|a| a.path.to_lowercase());

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
            ignore_matcher,
        );
        scan_result.directory_tree = new_tree.clone();

        Ok((
            Some(FsChangeEvent {
                updated: deduped,
                removed: payload_removed.into_iter().collect(),
                renamed,
                directory_tree: new_tree,
                total_count: scan_result.total_count,
                total_size: scan_result.total_size,
                type_counts,
            }),
            reap,
            matches.pairs,
        ))
    })?;

    for (from, to) in tier2_pairs {
        tag_migrations.push(TagMigration::File { from, to });
    }

    // ---- Phase 3: tag bookkeeping (migrate first, then reap) -------------
    if !tag_migrations.is_empty() || !reap.is_empty() {
        let _ = project::with_mut(project_id, |state| {
            // Last and most important of the three: this phase writes the tags
            // file, so an old root's migrations and reaps landing on a relocated
            // project's tags is a data loss, not just a stale row.
            batch_still_owns_project(state, &root_key)?;
            if !tag_migrations.is_empty() {
                // Migrations are data preservation — load the tags file even if it
                // was untouched this session. The reap-only branch stays lazy: an
                // unloaded orphan sits harmlessly on disk.
                let tags = state.ensure_tags();
                for m in &tag_migrations {
                    match m {
                        TagMigration::File { from, to } => tags.rename_path(from, to),
                        TagMigration::DirPrefix { from, to } => tags.rename_dir(from, to),
                    }
                }
                // Reap after migrating: a reap entry whose binding just moved
                // away is a no-op instead of a loss.
                for path in &reap {
                    tags.remove_path(path);
                }
                if let Err(e) = state.save_tags() {
                    eprintln!(
                        "[watcher] failed to save tags after rename migration: {}",
                        e
                    );
                    warnings_out.push(crate::warning::ProjectWarning::TagsNotSaved {
                        detail: e.to_string(),
                    });
                }
            } else if state.tags_data.is_some() {
                let tags = state.ensure_tags();
                for path in &reap {
                    tags.remove_path(path);
                }
                if let Err(e) = state.save_tags() {
                    eprintln!("[watcher] failed to save tags after orphan cleanup: {}", e);
                    warnings_out.push(crate::warning::ProjectWarning::TagsNotSaved {
                        detail: e.to_string(),
                    });
                }
            }
            Ok(())
        });
    }

    event.ok_or_else(|| "No effective changes".to_string())
}

/// Host asset for a Unity `.meta` sidecar path (`foo.png.meta` → `foo.png`),
/// or `None` when the path isn't a sidecar. A bare `".meta"` has no host.
fn meta_host_path(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let host = name.strip_suffix(".meta")?;
    if host.is_empty() {
        return None;
    }
    Some(path.with_file_name(host))
}

/// Path-shape checks shared by tracked asset files and tracked-path deletions:
/// inside `root`, no hidden path components, and a file name that is neither a
/// dotfile nor an engine sidecar. Unlike `is_trackable_path`, no extension needed.
fn path_shape_trackable(path: &Path, root: &Path) -> bool {
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

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    !(file_name.is_empty()
        || file_name.starts_with('.')
        || crate::sidecar::is_sidecar_name(file_name))
}

/// Mirrors the scanner's discovery filters: skip hidden path components (e.g.
/// `.git/`, `.vscode/`), engine sidecars, and files without an extension.
fn is_trackable_path(path: &Path, root: &Path) -> bool {
    path_shape_trackable(path, root) && path.extension().is_some()
}

/// Whether `asset_path` names a file at or under `deleted`. Component-wise (via
/// `Path::starts_with`) so a deleted `…/Tex` does not sweep away `…/Textures/*`
/// and a deleted directory removes every tracked file beneath it.
fn path_within(asset_path: &str, deleted: &Path) -> bool {
    Path::new(asset_path).starts_with(deleted)
}

/// Whether `path` is excluded by the project's `.gitignore` rules, using the
/// matcher built at watcher start. A `None` matcher never ignores. Matching is
/// lexical, so deletions of never-tracked paths are rejected correctly too.
fn is_gitignored(path: &Path, root: &Path, matcher: Option<&scanner::IgnoreMatcher>) -> bool {
    let Some(matcher) = matcher else {
        return false;
    };
    let Ok(rel) = path.strip_prefix(root) else {
        // Outside root — is_trackable_path already rejects these.
        return false;
    };
    matcher.is_ignored(rel, path.is_dir())
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
    use crate::scanner::ScanResult;
    use notify::event::{CreateKind, Event};
    use std::fs;
    use std::time::Instant;
    use tempfile::tempdir;

    /// Create `files` under `dir`, parse them into a cached scan, register the
    /// project, and return the normalized asset paths. Content is the file's own
    /// relative name, so same-named files get equal sizes and others differ.
    fn setup_project(id: &str, dir: &Path, files: &[&str]) -> Vec<String> {
        let mut assets = Vec::new();
        let mut paths = Vec::new();
        for rel in files {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            fs::write(&p, name.as_bytes()).unwrap();
            let info = scanner::parse_asset_file(&p, &None).expect("test file must parse");
            paths.push(info.path.clone());
            assets.push(info);
        }
        project::register(id.to_string(), scanner::path_to_string(dir));
        project::with_mut(id, |state| {
            state.cached_scan = Some(ScanResult {
                root_path: scanner::path_to_string(dir),
                directory_tree: scanner::build_directory_tree(dir, &assets, None),
                total_count: assets.len(),
                total_size: assets.iter().map(|a| a.size).sum(),
                type_counts: HashMap::new(),
                project_type: None,
                warnings: Vec::new(),
                assets: assets.clone(),
            });
            Ok(())
        })
        .unwrap();
        paths
    }

    fn tag_asset(id: &str, path: &str) -> String {
        project::with_mut(id, |state| {
            let tags = state.ensure_tags();
            let tag = tags.create_tag("t".to_string(), "#fff".to_string());
            tags.add_tag_to_asset(path, &tag.id);
            Ok(tag.id)
        })
        .unwrap()
    }

    fn tags_at(id: &str, path: &str) -> usize {
        project::with_ref(id, |state| {
            Ok(state
                .tags_data
                .as_ref()
                .map(|t| t.get_asset_tags(path).len())
                .unwrap_or(0))
        })
        .unwrap()
    }

    fn scan_paths(id: &str) -> Vec<String> {
        project::with_ref(id, |state| {
            Ok(state
                .cached_scan
                .as_ref()
                .unwrap()
                .assets
                .iter()
                .map(|a| a.path.clone())
                .collect())
        })
        .unwrap()
    }

    // ---- pure helpers -----------------------------------------------------

    #[test]
    fn split_batch_extracts_both_pairs_and_passes_singles_through() {
        let both = DebouncedEvent::new(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(PathBuf::from("/p/a.png"))
                .add_path(PathBuf::from("/p/b.png")),
            Instant::now(),
        );
        let lone_from = DebouncedEvent::new(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
                .add_path(PathBuf::from("/p/gone.png")),
            Instant::now(),
        );
        let create = DebouncedEvent::new(
            Event::new(EventKind::Create(CreateKind::Any)).add_path(PathBuf::from("/p/new.png")),
            Instant::now(),
        );

        let (pairs, singles) = split_batch(&[both, lone_from, create]);
        assert_eq!(
            pairs,
            vec![(PathBuf::from("/p/a.png"), PathBuf::from("/p/b.png"))]
        );
        assert_eq!(
            singles,
            vec![PathBuf::from("/p/gone.png"), PathBuf::from("/p/new.png")]
        );
    }

    #[test]
    fn route_pair_sends_sidecar_pairs_back_to_the_single_pipeline() {
        let root = Path::new("/proj");
        // Engine renamed a sidecar alongside its host: both halves fall back
        // (the To-side .meta then refreshes its host through the remap).
        match route_pair(
            Path::new("/proj/a.png.meta"),
            Path::new("/proj/b.png.meta"),
            root,
        ) {
            PairRoute::Fallback(paths) => assert_eq!(paths.len(), 2),
            PairRoute::Rename => panic!("sidecar pair must not be a rename"),
        }
        // Ordinary asset rename stays a rename.
        assert!(matches!(
            route_pair(Path::new("/proj/a.png"), Path::new("/proj/b.png"), root),
            PairRoute::Rename
        ));
    }

    #[test]
    fn route_pair_drops_untracked_churn_and_same_path() {
        let root = Path::new("/proj");
        // `.git` lockfile churn: both shapes untracked → dropped outright.
        match route_pair(
            Path::new("/proj/.git/index.lock"),
            Path::new("/proj/.git/index"),
            root,
        ) {
            PairRoute::Fallback(paths) => assert!(paths.is_empty()),
            PairRoute::Rename => panic!(".git churn must not be a rename"),
        }
        match route_pair(Path::new("/proj/a.png"), Path::new("/proj/a.png"), root) {
            PairRoute::Fallback(paths) => assert!(paths.is_empty()),
            PairRoute::Rename => panic!("same-path pair must be dropped"),
        }
        // One visible side keeps the pair alive (rename out of / into hiding
        // migrates the binding so it can come back).
        assert!(matches!(
            route_pair(
                Path::new("/proj/a.png"),
                Path::new("/proj/.hidden/a.png"),
                root
            ),
            PairRoute::Rename
        ));
    }

    #[test]
    fn match_orphans_pairs_bijective_triples_only() {
        let gone = |path: &str, name: &str, size: u64, mtime: u64| OrphanInfo {
            path: path.to_string(),
            name: name.to_string(),
            size,
            modified: mtime,
        };
        let removed = vec![
            gone("/p/a.png", "a.png", 10, 100),   // unique triple → pairs
            gone("/p/x/d.png", "d.png", 20, 200), // duplicated triple → preserved
            gone("/p/y/d.png", "d.png", 20, 200),
            gone("/p/del.png", "del.png", 30, 300), // no counterpart → reapable
        ];
        let added = vec![
            gone("/p/sub/a.png", "a.png", 10, 100),
            gone("/p/z/d.png", "d.png", 20, 200),
            gone("/p/fresh.png", "fresh.png", 40, 400), // plain addition
        ];

        let m = match_orphans(&removed, &added);
        assert_eq!(
            m.pairs,
            vec![("/p/a.png".to_string(), "/p/sub/a.png".to_string())]
        );
        assert!(m.preserved.contains("/p/x/d.png"));
        assert!(m.preserved.contains("/p/y/d.png"));
        // The plain removal is in neither list — the caller reaps it.
        assert!(!m.preserved.contains("/p/del.png"));
        assert!(m.pairs.iter().all(|(f, _)| f != "/p/del.png"));
    }

    // ---- apply_changes integration (real files, synthetic pairs) ----------

    /// A batch collected under the old root must not reach a project that has
    /// been re-registered elsewhere since. The new root has a scan of its own by
    /// then, so without the fence the old root's entries patch it and the old
    /// root's migrations rewrite a tags file belonging to a different folder.
    #[test]
    fn a_batch_from_the_old_root_is_refused_after_the_project_moves() {
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        let id = "watcher_test_relocated_batch";

        let paths = setup_project(id, old.path(), &["hero.png"]);
        tag_asset(id, &paths[0]);

        let from = old.path().join("hero.png");
        let to = old.path().join("knight.png");
        fs::rename(&from, &to).unwrap();

        // The project is pointed at a different folder, which rebuilds its state,
        // and a scan of that folder lands before the old batch is applied.
        let new_paths = setup_project(id, new.path(), &["hero.png"]);

        let err = apply_changes(id, &[(from, to)], &[], old.path(), None, &mut Vec::new())
            .expect_err("a batch from the old root must be refused");
        assert!(err.contains("root changed"), "unexpected error: {err}");

        // The new root's scan is untouched: no old-root path was patched in, and
        // its own asset is still there.
        assert_eq!(scan_paths(id), new_paths);

        project::unregister(id);
    }

    #[test]
    fn stitched_rename_rekeys_scan_and_migrates_tags() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_stitched_rename";
        let paths = setup_project(id, dir.path(), &["hero.png"]);
        tag_asset(id, &paths[0]);

        let from = dir.path().join("hero.png");
        let to = dir.path().join("knight.png");
        fs::rename(&from, &to).unwrap();

        let ev = apply_changes(id, &[(from, to)], &[], dir.path(), None, &mut Vec::new()).unwrap();

        let to_str = scanner::path_to_string(&dir.path().join("knight.png"));
        assert_eq!(ev.renamed.len(), 1);
        assert_eq!(ev.renamed[0].from, paths[0]);
        assert_eq!(ev.renamed[0].to, to_str);
        assert!(!ev.renamed[0].is_dir);
        assert!(ev.removed.contains(&paths[0]));
        assert!(ev.updated.iter().any(|a| a.path == to_str));
        assert_eq!(scan_paths(id), vec![to_str.clone()]);
        // Tags followed; nothing left at the old key.
        assert_eq!(tags_at(id, &to_str), 1);
        assert_eq!(tags_at(id, &paths[0]), 0);
    }

    /// The safe-save pattern: an editor renames `a.png` → `a.png.bak`, then
    /// writes a fresh `a.png`. The rename is physically real, but the tag
    /// belongs to the role at `a.png` — it must NOT walk off to the backup.
    #[test]
    fn safe_save_keeps_tags_at_the_source_path() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_safe_save";
        let paths = setup_project(id, dir.path(), &["a.png"]);
        tag_asset(id, &paths[0]);

        let from = dir.path().join("a.png");
        let to = dir.path().join("a.png.bak");
        fs::rename(&from, &to).unwrap();
        fs::write(&from, b"fresh contents").unwrap(); // re-occupied

        let ev = apply_changes(
            id,
            &[(from.clone(), to)],
            &[from],
            dir.path(),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        // Not advertised as a rename, tags stayed at the living path.
        assert!(ev.renamed.is_empty());
        assert_eq!(tags_at(id, &paths[0]), 1);
        let bak = scanner::path_to_string(&dir.path().join("a.png.bak"));
        assert_eq!(tags_at(id, &bak), 0);
        // Scan holds both the fresh file and the backup.
        let mut got = scan_paths(id);
        got.sort();
        let mut want = vec![paths[0].clone(), bak];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn directory_rename_rekeys_subtree_and_migrates_tag_prefixes() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_dir_rename";
        let paths = setup_project(
            id,
            dir.path(),
            &["Tex/a.png", "Tex/sub/d.png", "Other/x.png"],
        );
        tag_asset(id, &paths[0]);

        let from = dir.path().join("Tex");
        let to = dir.path().join("Art");
        fs::rename(&from, &to).unwrap();

        let ev = apply_changes(id, &[(from, to)], &[], dir.path(), None, &mut Vec::new()).unwrap();

        assert_eq!(ev.renamed.len(), 1);
        assert!(ev.renamed[0].is_dir);
        let new_a = scanner::path_to_string(&dir.path().join("Art/a.png"));
        let new_d = scanner::path_to_string(&dir.path().join("Art/sub/d.png"));
        assert!(ev.removed.contains(&paths[0]));
        assert!(ev.removed.contains(&paths[1]));
        let got = scan_paths(id);
        assert!(got.contains(&new_a) && got.contains(&new_d));
        assert!(got.contains(&paths[2]), "sibling dir must be untouched");
        assert!(!got.contains(&paths[0]));
        // Tag followed the subtree.
        assert_eq!(tags_at(id, &new_a), 1);
        assert_eq!(tags_at(id, &paths[0]), 0);
    }

    /// Windows reports a cross-directory move as plain REMOVED+ADDED — no
    /// rename halves for the debouncer to stitch. The identity triple joins
    /// them instead.
    #[test]
    fn cross_directory_move_pairs_by_identity_and_migrates_tags() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_tier2_move";
        let paths = setup_project(id, dir.path(), &["a.png", "keep/k.png"]);
        tag_asset(id, &paths[0]);

        let from = dir.path().join("a.png");
        let to_dir = dir.path().join("moved");
        fs::create_dir_all(&to_dir).unwrap();
        let to = to_dir.join("a.png");
        fs::rename(&from, &to).unwrap(); // preserves size + mtime

        let ev = apply_changes(
            id,
            &[],
            &[from, to.clone()],
            dir.path(),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        let to_str = scanner::path_to_string(&to);
        assert_eq!(ev.renamed.len(), 1);
        assert_eq!(ev.renamed[0].from, paths[0]);
        assert_eq!(ev.renamed[0].to, to_str);
        assert_eq!(tags_at(id, &to_str), 1);
        assert_eq!(tags_at(id, &paths[0]), 0);
    }

    /// Two identical files (same name, size, mtime) moved in one batch: there is
    /// no telling which went where, so bindings stay at the old paths, unreaped.
    #[test]
    fn ambiguous_identity_preserves_bindings_unreaped() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_tier2_ambiguous";
        let paths = setup_project(id, dir.path(), &["x/a.png", "y/a.png"]);

        // Force-equal mtimes (whole second, so the parsed secs agree).
        let t = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for rel in ["x/a.png", "y/a.png"] {
            fs::File::options()
                .write(true)
                .open(dir.path().join(rel))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        // Re-cache with the forced mtimes.
        let assets: Vec<AssetInfo> = ["x/a.png", "y/a.png"]
            .iter()
            .map(|rel| scanner::parse_asset_file(&dir.path().join(rel), &None).unwrap())
            .collect();
        project::with_mut(id, |state| {
            state.cached_scan.as_mut().unwrap().assets = assets.clone();
            Ok(())
        })
        .unwrap();
        tag_asset(id, &paths[0]);
        tag_asset(id, &paths[1]);

        let moves = [("x/a.png", "x2/a.png"), ("y/a.png", "y2/a.png")];
        let mut candidates = Vec::new();
        for (from, to) in moves {
            let from = dir.path().join(from);
            let to = dir.path().join(to);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::rename(&from, &to).unwrap();
            candidates.push(from);
            candidates.push(to);
        }

        let ev = apply_changes(id, &[], &candidates, dir.path(), None, &mut Vec::new()).unwrap();

        assert!(ev.renamed.is_empty(), "ambiguous moves must not pair");
        // Bindings preserved at the OLD paths — not migrated, not reaped.
        assert_eq!(tags_at(id, &paths[0]), 1);
        assert_eq!(tags_at(id, &paths[1]), 1);
    }

    #[test]
    fn true_delete_still_reaps_the_binding() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_delete_reaps";
        let paths = setup_project(id, dir.path(), &["gone.png"]);
        tag_asset(id, &paths[0]);

        let p = dir.path().join("gone.png");
        fs::remove_file(&p).unwrap();

        let ev = apply_changes(id, &[], &[p], dir.path(), None, &mut Vec::new()).unwrap();
        assert!(ev.removed.contains(&paths[0]));
        assert_eq!(tags_at(id, &paths[0]), 0);
    }

    /// Renamed into a hidden directory: the asset leaves the visible set but
    /// the binding migrates with it — parked on the invisible path, restored
    /// if the file ever comes back.
    #[test]
    fn rename_into_hidden_path_parks_the_binding() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_hidden_park";
        let paths = setup_project(id, dir.path(), &["h.png"]);
        tag_asset(id, &paths[0]);

        let from = dir.path().join("h.png");
        let hidden = dir.path().join(".hidden");
        fs::create_dir_all(&hidden).unwrap();
        let to = hidden.join("h.png");
        fs::rename(&from, &to).unwrap();

        let ev = apply_changes(
            id,
            &[(from, to.clone())],
            &[],
            dir.path(),
            None,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(ev.renamed.is_empty(), "invisible target is not advertised");
        assert!(ev.removed.contains(&paths[0]));
        assert!(scan_paths(id).is_empty());
        // Binding parked on the hidden path, not reaped.
        assert_eq!(tags_at(id, &scanner::path_to_string(&to)), 1);
        assert_eq!(tags_at(id, &paths[0]), 0);
    }

    /// A directory that appears wholesale (moved or copied into the project)
    /// delivers one event on the directory path and none for its children —
    /// they must be listed from the event, not left for the next full rescan.
    #[test]
    fn appeared_directory_lists_its_children() {
        let dir = tempdir().unwrap();
        let id = "watcher_test_dir_appeared";
        setup_project(id, dir.path(), &["base.png"]);

        let pack = dir.path().join("pack");
        fs::create_dir_all(pack.join("sub")).unwrap();
        fs::write(pack.join("one.png"), b"one").unwrap();
        fs::write(pack.join("sub/two.png"), b"two").unwrap();

        let candidates = [pack.clone()];
        let ev = apply_changes(id, &[], &candidates, dir.path(), None, &mut Vec::new()).unwrap();

        let one = scanner::path_to_string(&pack.join("one.png"));
        let two = scanner::path_to_string(&pack.join("sub/two.png"));
        assert!(ev.updated.iter().any(|a| a.path == one));
        assert!(ev.updated.iter().any(|a| a.path == two));
        let got = scan_paths(id);
        assert!(got.contains(&one) && got.contains(&two));
    }

    #[test]
    fn trackable_skips_hidden_components() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(Path::new("/proj/.git/HEAD"), root));
        assert!(!is_trackable_path(
            Path::new("/proj/sub/.hidden/file.png"),
            root
        ));
    }

    #[test]
    fn meta_events_map_to_their_host_asset() {
        assert_eq!(
            meta_host_path(Path::new("/proj/tex.png.meta")),
            Some(PathBuf::from("/proj/tex.png"))
        );
        // Not a sidecar → no remap.
        assert_eq!(meta_host_path(Path::new("/proj/tex.png")), None);
        // A bare ".meta" has no host.
        assert_eq!(meta_host_path(Path::new("/proj/.meta")), None);
        // Extensionless host: remapped, but is_trackable_path drops it later
        // (mirrors the scanner, which only tracks extensioned files).
        let host = meta_host_path(Path::new("/proj/Makefile.meta")).unwrap();
        assert!(!is_trackable_path(&host, Path::new("/proj")));
    }

    #[test]
    fn trackable_skips_meta_and_dotfiles() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(Path::new("/proj/foo.png.meta"), root));
        assert!(!is_trackable_path(Path::new("/proj/.env"), root));
    }

    /// The watcher's filter has to agree with the scanner's, or a sidecar the
    /// scan refused to list gets added back the moment the engine touches it.
    #[test]
    fn trackable_skips_godot_sidecars() {
        let root = Path::new("/proj");
        assert!(!is_trackable_path(Path::new("/proj/hero.png.import"), root));
        assert!(!is_trackable_path(Path::new("/proj/player.gd.uid"), root));
        // The assets themselves stay tracked.
        assert!(is_trackable_path(Path::new("/proj/hero.png"), root));
        assert!(is_trackable_path(Path::new("/proj/player.gd"), root));
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

    // macOS coalesces `rm -rf <dir>` into a single event on the extensionless
    // directory path. That path must survive filtering so `apply_changes` can
    // drop the tracked files beneath it.
    #[test]
    fn deleted_directory_path_is_shape_trackable_despite_no_extension() {
        let root = Path::new("/proj");
        assert!(path_shape_trackable(Path::new("/proj/Models"), root));
        assert!(path_shape_trackable(Path::new("/proj/sub/Textures"), root));
        // But an *existing* extensionless path is still not a trackable asset.
        assert!(!is_trackable_path(Path::new("/proj/Models"), root));
    }

    #[test]
    fn path_shape_trackable_still_rejects_hidden_meta_and_outside_root() {
        let root = Path::new("/proj");
        assert!(!path_shape_trackable(Path::new("/proj/.git/HEAD"), root));
        assert!(!path_shape_trackable(
            Path::new("/proj/sub/.hidden/file.png"),
            root
        ));
        assert!(!path_shape_trackable(Path::new("/proj/tex.png.meta"), root));
        assert!(!path_shape_trackable(Path::new("/other/foo.png"), root));
    }

    #[test]
    fn path_within_removes_children_of_a_deleted_directory() {
        let dir = Path::new("/proj/Models");
        assert!(path_within("/proj/Models/cube.obj", dir));
        assert!(path_within("/proj/Models/sub/deep.png", dir));
    }

    #[test]
    fn path_within_matches_an_exact_deleted_file() {
        assert!(path_within("/proj/a.png", Path::new("/proj/a.png")));
    }

    #[test]
    fn path_within_is_component_wise_not_string_prefix() {
        // A deleted `/proj/Tex` must not sweep away `/proj/Textures/*`,
        // and `/proj/Models` must not match a sibling `/proj/ModelsX/*`.
        assert!(!path_within("/proj/Textures/t.png", Path::new("/proj/Tex")));
        assert!(!path_within(
            "/proj/ModelsX/y.png",
            Path::new("/proj/Models")
        ));
    }
}
