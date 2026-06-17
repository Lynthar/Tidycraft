mod analyzer;
mod cache;
mod git;
mod godot;
mod llm;
mod meta_sidecar;
mod project;
mod scanner;
mod tags;
mod thumbnail;
mod undo;
mod unity;
mod unreal;
mod watcher;

use analyzer::rules::RuleConfig;
use analyzer::tag_suggest::{HeuristicSuggester, TagGroup, TagSuggester};
use analyzer::{AnalysisResult, Analyzer};
use cache::ScanCache;
use git::{GitInfo, GitManager};
use scanner::{IncrementalStats, ScanProgress, ScanResult, ScanState};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// ============ Project Lifecycle ============

#[tauri::command]
fn register_project(project_id: String, path: String) -> Result<(), String> {
    project::register(project_id, path);
    Ok(())
}

#[tauri::command]
fn unregister_project(project_id: String) -> Result<(), String> {
    project::unregister(&project_id);
    Ok(())
}

// ============ Scan Commands ============

#[tauri::command]
fn scan_project(project_id: String, path: String) -> Result<ScanResult, String> {
    project::register(project_id.clone(), path.clone());

    // Legacy synchronous command — front-end uses scan_project_incremental
    // for the user-toggleable setting. Hardcoding `true` here matches the
    // "respect gitignore by default" semantics of the new flow.
    let result = scanner::scan_directory_with_state(&path, None, true).map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |state| {
        state.cached_scan = Some(result.clone());
        Ok(())
    })?;

    Ok(result)
}

/// Spawn a background thread that emits `scan-progress-{project_id}` events
/// every 100ms until the scan reaches a terminal phase OR the caller flips
/// `stop`. The `stop` flag matters: the scan function's early `Err` paths
/// (folder moved/missing, not a directory, cancel during discovery) return
/// without ever marking the phase `Completed`/`Cancelled`, so a phase-only loop
/// would spin forever and the caller's `join()` would deadlock — which surfaced
/// as the app hanging at "discovering files" with no error.
fn spawn_progress_reporter(
    app: AppHandle,
    project_id: String,
    state: Arc<ScanState>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    let event_name = format!("scan-progress-{}", project_id);
    thread::spawn(move || loop {
        let progress = state.get_progress();
        let is_done = matches!(
            progress.phase,
            scanner::ScanPhase::Completed | scanner::ScanPhase::Cancelled
        );

        let _ = app.emit(&event_name, &progress);

        if is_done || stop.load(Ordering::SeqCst) {
            break;
        }

        thread::sleep(Duration::from_millis(100));
    })
}

#[tauri::command]
async fn scan_project_async(
    app: AppHandle,
    project_id: String,
    path: String,
) -> Result<ScanResult, String> {
    project::register(project_id.clone(), path.clone());

    let state = Arc::new(ScanState::new());
    // In-flight guard: `scan_state` being `Some` means another scan already
    // owns this project. Reject the second one rather than overwriting the
    // first's state (which would drop its cancellation, interleave the two
    // progress reporters, and let an older scan's result clobber a newer one).
    // The check + set is atomic under the project lock held by `with_mut`.
    let already = project::with_mut(&project_id, |s| {
        if s.scan_state.is_some() {
            return Ok(true);
        }
        s.scan_state = Some(state.clone());
        Ok(false)
    })?;
    if already {
        return Err("A scan is already in progress for this project".to_string());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let progress_handle =
        spawn_progress_reporter(app.clone(), project_id.clone(), state.clone(), stop.clone());

    let state_for_scan = state.clone();
    let path_for_scan = path.clone();
    let join_result = tokio::task::spawn_blocking(move || {
        // Legacy async command — same default as scan_project; the
        // toggleable variant is scan_project_incremental.
        scanner::scan_directory_with_state(&path_for_scan, Some(state_for_scan), true)
    })
    .await;

    // Stop the reporter and join it BEFORE propagating any error: the scan's
    // early `Err` paths never mark a terminal phase, so otherwise `join()`
    // would block forever (the "stuck at discovering files" hang).
    stop.store(true, Ordering::SeqCst);
    let _ = progress_handle.join();

    let _ = project::with_mut(&project_id, |s| {
        s.scan_state = None;
        Ok(())
    });

    let scan_result = join_result
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |s| {
        s.cached_scan = Some(scan_result.clone());
        // Legacy command always scans with gitignore on (see above) — record
        // it so the watcher mirrors the same exclusions.
        s.respect_gitignore = true;
        Ok(())
    })?;

    Ok(scan_result)
}

#[tauri::command]
fn cancel_scan(project_id: String) -> bool {
    project::with_ref(&project_id, |s| {
        Ok(s.scan_state.as_ref().map(|st| st.cancel()).is_some())
    })
    .unwrap_or(false)
}

#[tauri::command]
fn get_scan_progress(project_id: String) -> Option<ScanProgress> {
    project::with_ref(&project_id, |s| {
        Ok(s.scan_state.as_ref().map(|st| st.get_progress()))
    })
    .ok()
    .flatten()
}

// ============ Incremental Scan Commands ============

#[derive(Serialize)]
pub struct IncrementalScanResult {
    pub result: ScanResult,
    pub stats: IncrementalStats,
}

#[tauri::command]
async fn scan_project_incremental(
    app: AppHandle,
    project_id: String,
    path: String,
    // Frontend-visible: when true (default), the scanner honors
    // `.gitignore` / `.ignore` files (and skips hidden dot dirs like
    // `.git/`). Toggle exposed via Settings → Maintenance for users
    // who need full coverage on a project with gitignored asset folders.
    respect_gitignore: bool,
) -> Result<IncrementalScanResult, String> {
    project::register(project_id.clone(), path.clone());

    let state = Arc::new(ScanState::new());
    // In-flight guard: `scan_state` being `Some` means another scan already
    // owns this project. Reject the second one rather than overwriting the
    // first's state (which would drop its cancellation, interleave the two
    // progress reporters, and let an older scan's result clobber a newer one).
    // The check + set is atomic under the project lock held by `with_mut`.
    let already = project::with_mut(&project_id, |s| {
        if s.scan_state.is_some() {
            return Ok(true);
        }
        s.scan_state = Some(state.clone());
        Ok(false)
    })?;
    if already {
        return Err("A scan is already in progress for this project".to_string());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let progress_handle =
        spawn_progress_reporter(app.clone(), project_id.clone(), state.clone(), stop.clone());

    let state_for_scan = state.clone();
    let path_for_scan = path.clone();
    let join_result = tokio::task::spawn_blocking(move || {
        scanner::scan_directory_incremental(&path_for_scan, Some(state_for_scan), respect_gitignore)
    })
    .await;

    // Stop the reporter and join it BEFORE propagating any error: the scan's
    // early `Err` paths (e.g. the project folder was moved/deleted) never mark a
    // terminal phase, so otherwise `join()` would block forever — the hang that
    // left the UI stuck at "discovering files" with no error.
    stop.store(true, Ordering::SeqCst);
    let _ = progress_handle.join();

    let _ = project::with_mut(&project_id, |s| {
        s.scan_state = None;
        Ok(())
    });

    let (scan_result, stats) = join_result
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |s| {
        s.cached_scan = Some(scan_result.clone());
        s.respect_gitignore = respect_gitignore;
        Ok(())
    })?;

    Ok(IncrementalScanResult {
        result: scan_result,
        stats,
    })
}

#[tauri::command]
fn clear_scan_cache(path: String) -> Result<(), String> {
    ScanCache::clear(&path).map_err(|e| e.to_string())
}

// ============ Filesystem Watcher ============

#[tauri::command]
fn start_watching(app: AppHandle, project_id: String) -> Result<(), String> {
    let (root_path, respect_gitignore) =
        project::with_ref(&project_id, |s| Ok((s.root_path.clone(), s.respect_gitignore)))?;
    let w = watcher::start(app, project_id.clone(), root_path, respect_gitignore)?;
    project::with_mut(&project_id, |s| {
        s.watcher = Some(w);
        Ok(())
    })
}

#[tauri::command]
fn stop_watching(project_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |s| {
        s.watcher = None;
        Ok(())
    })
}

#[tauri::command]
async fn get_thumbnail(path: String, size: u32) -> Result<String, String> {
    // Decode + resize + PNG-encode is CPU-bound and synchronous; run it on the
    // blocking pool so fast gallery scrolling doesn't starve the async worker
    // threads every other IPC call shares.
    tokio::task::spawn_blocking(move || {
        thumbnail::get_thumbnail_base64(&path, size).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("thumbnail task failed: {e}"))?
}

#[tauri::command]
fn get_thumbnail_cache_size() -> u64 {
    thumbnail::get_cache_size()
}

/// Drop the entire on-disk thumbnail cache. Returns the number of bytes
/// freed so the UI can show "Freed N MB" feedback.
#[tauri::command]
fn clear_thumbnail_cache() -> Result<u64, String> {
    let before = thumbnail::get_cache_size();
    thumbnail::clear_cache().map_err(|e| e.to_string())?;
    Ok(before)
}

// ============ LLM Tagging Commands ============
//
// `llm_suggest_tags` dispatches to the configured provider's real HTTP
// endpoint. `llm_estimate_cost` is pure math (no network) and the cache
// commands just read/clear a directory, so both work without a provider.

/// Cost preview for the AIAnalyzeModal. Pure function — no network and
/// no API key required.
#[tauri::command]
fn llm_estimate_cost(
    provider: String,
    model: String,
    asset_count: usize,
    has_thumbnails: bool,
) -> Result<llm::CostEstimate, String> {
    let cfg = llm::ProviderConfig {
        api_key: None,
        endpoint: None,
        model: model.clone(),
    };
    let prov = llm::make_provider(&provider, cfg).map_err(String::from)?;

    // Build a dummy request just to feed the cost estimator. The
    // estimator only reads asset count + thumbnail presence + model id;
    // the actual paths/filenames don't affect the math.
    let assets = (0..asset_count)
        .map(|i| llm::AssetInput {
            path: format!("dummy/{i}"),
            filename: format!("{i}"),
            thumbnail_base64: if has_thumbnails {
                Some(String::new())
            } else {
                None
            },
            metadata_hint: None,
        })
        .collect();

    let req = llm::TagRequest {
        assets,
        prompt_version: llm::prompts::PROMPT_VERSION,
        model,
        include_thumbnails: has_thumbnails,
        // Cost estimate doesn't depend on the actual project framing
        // (it's a function of asset count + model + thumb presence).
        // We pass empty context to keep the math simple.
        project_ctx: None,
        existing_tags: Vec::new(),
    };
    Ok(prov.estimate_cost(&req))
}

/// Convert an absolute asset path to a project-relative one for the LLM
/// prompt + cache key, so cloud providers never receive the user's machine
/// path (drive, username, directory layout). Folder structure under the
/// project root is preserved — it's useful semantic context for tagging.
/// Never returns the absolute path: with no project root (unregistered
/// project) or a path outside the root, it falls back to the bare filename.
fn project_relative_path(abs: &str, root: &str) -> String {
    let basename = || abs.rsplit(['/', '\\']).next().unwrap_or(abs).to_string();
    if root.is_empty() {
        return basename();
    }
    Path::new(abs)
        .strip_prefix(root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| basename())
}

/// Relativize each existing-tag sample path against the project root before it
/// enters an LLM prompt or the per-asset cache key. Without this, absolute
/// paths (drive letter, username, full directory layout) ship to the provider
/// and bake machine-specific data into the cache hash. Paths outside the root
/// fall back to their basename — same policy as `project_relative_path`.
fn relativize_samples(samples: Vec<String>, root: &str) -> Vec<String> {
    samples
        .into_iter()
        .map(|p| project_relative_path(&p, root))
        .collect()
}

/// Minimal HTML escaping for project-derived strings (asset names, paths, rule
/// messages) interpolated into the HTML report. Without it, a file named e.g.
/// `<img src=x onerror=...>.png` injects markup/script that runs when the user
/// opens the exported report. Escapes the five HTML-significant chars; `&` must
/// go first so we don't double-escape the entities we just inserted.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Main entry point for AI tagging. Loads thumbnails for the selected
/// assets, gathers project context (theme/goal from tidycraft.toml +
/// existing tags with up to 5 sample paths each), then dispatches to
/// the chosen provider via `make_provider`.
#[tauri::command]
async fn llm_suggest_tags(
    project_id: String,
    asset_paths: Vec<String>,
    provider: String,
    model: String,
    api_key: Option<String>,
    endpoint: Option<String>,
    upload_thumbnails: bool,
) -> Result<llm::TagResponse, String> {
    let cfg = llm::ProviderConfig {
        api_key,
        endpoint,
        model: model.clone(),
    };
    let prov = llm::make_provider(&provider, cfg).map_err(String::from)?;

    // Snapshot project context inside the project lock, then drop the
    // lock before any async work. The lock is held only briefly: we
    // clone tag names, descriptions, and a small list of sample paths.
    //
    // SAMPLES_PER_TAG: how many existing-asset paths we ship per tag.
    // 5 is a sweet spot between giving the LLM enough usage context
    // to infer the tag's intent and not blowing the prompt budget on
    // a project with hundreds of tags. Less than the tag count
    // truncates; the LLM doesn't need exhaustive samples.
    const SAMPLES_PER_TAG: usize = 5;

    let context_result = project::with_mut(&project_id, |state| {
        let root = state.root_path.clone();
        let tags_data = state.ensure_tags();
        let mut existing: Vec<llm::ExistingTagContext> =
            Vec::with_capacity(tags_data.tags.len());
        for tag in &tags_data.tags {
            let mut samples = tags_data.get_assets_with_tag(&tag.id);
            samples.truncate(SAMPLES_PER_TAG);
            existing.push(llm::ExistingTagContext {
                name: tag.name.clone(),
                description: tag.description.clone(),
                sample_paths: relativize_samples(samples, &root),
            });
        }
        Ok((root, existing))
    });

    // If the project somehow isn't registered (UI should always register
    // before calling, but be defensive), fall through with empty context
    // — the LLM still works, just without project framing.
    let (root_path, existing_tags) = context_result.unwrap_or_else(|e| {
        eprintln!("[llm_suggest_tags] context fetch failed: {e}");
        (String::new(), Vec::new())
    });

    // Read [project] from tidycraft.toml. We do this outside the project
    // lock to avoid holding it through file IO. Missing file / parse
    // failure / empty meta all collapse to None — no project block.
    let project_ctx: Option<llm::project_meta::ProjectMeta> = if root_path.is_empty() {
        None
    } else {
        let toml_path = Path::new(&root_path).join("tidycraft.toml");
        std::fs::read_to_string(&toml_path)
            .ok()
            .and_then(|content| llm::project_meta::ProjectMeta::from_toml(&content).ok())
            .filter(|m| !m.is_empty())
    };

    // Load thumbnails on the blocking pool — `get_thumbnail_base64`
    // does PNG decode + resize + base64 encode, which would otherwise
    // park the tokio runtime for tens of milliseconds per asset. The
    // thumbnail layer already has its own disk cache so repeat calls
    // for unchanged files are cheap.
    //
    // Per-asset failures (unsupported format, missing file, codec gap
    // for HDR/EXR) downgrade silently to `thumbnail_base64=None` —
    // the request still goes through, the LLM just falls back to
    // filename + path context for those entries.
    // Map the project-relative path (what we ship to the provider, cache,
    // and the LLM echoes back) to the absolute path the frontend needs to
    // bind tags. Built before `asset_paths` is moved into the builders.
    let abs_by_rel: HashMap<String, String> = asset_paths
        .iter()
        .map(|abs| (project_relative_path(abs, &root_path), abs.clone()))
        .collect();

    let assets = if upload_thumbnails {
        let paths = asset_paths;
        let root_for_thumbs = root_path.clone();
        tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|p| {
                    let filename = p
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&p)
                        .to_string();
                    // Thumbnail decode needs the real (absolute) path; the
                    // path we ship to the provider is project-relative so we
                    // never leak the user's drive / username / layout.
                    let thumb = thumbnail::get_thumbnail_base64(&p, 256).ok();
                    llm::AssetInput {
                        path: project_relative_path(&p, &root_for_thumbs),
                        filename,
                        thumbnail_base64: thumb,
                        metadata_hint: None,
                    }
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| format!("thumbnail load join failed: {e}"))?
    } else {
        asset_paths
            .into_iter()
            .map(|p| {
                let filename = p
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&p)
                    .to_string();
                llm::AssetInput {
                    path: project_relative_path(&p, &root_path),
                    filename,
                    thumbnail_base64: None,
                    metadata_hint: None,
                }
            })
            .collect()
    };

    let req = llm::TagRequest {
        assets,
        prompt_version: llm::prompts::PROMPT_VERSION,
        model,
        include_thumbnails: upload_thumbnails,
        project_ctx,
        existing_tags,
    };

    // The provider only ever saw project-relative paths, so suggestions come
    // back keyed by those. Remap each to the absolute path so the frontend
    // binds tags to the scanned (absolute-path) assets. A miss (LLM mangled
    // the path) leaves it untouched — the same graceful degradation as the
    // pre-relativization behavior.
    let mut response = prov.suggest_tags(&req).await.map_err(String::from)?;
    for s in &mut response.suggestions {
        if let Some(abs) = abs_by_rel.get(&s.asset_path) {
            s.asset_path = abs.clone();
        }
    }
    Ok(response)
}

#[tauri::command]
fn llm_clear_cache() -> Result<u64, String> {
    let before = llm::cache::size();
    llm::cache::clear().map_err(|e| e.to_string())?;
    Ok(before)
}

/// Day 6: AI Learning entry point. Samples the project, sends the
/// samples + tag system + project meta to the LLM, persists the
/// returned heuristic rules to `<project>/tidycraft.ai.toml`, and
/// returns the full `LearningResult` for the review panel.
#[tauri::command]
async fn learn_project_conventions(
    project_id: String,
    provider: String,
    model: String,
    api_key: Option<String>,
    endpoint: Option<String>,
    sampling_depth: usize,
) -> Result<llm::learning::LearningResult, String> {
    // Clamp depth to the documented 3..=30 range so a UI bug or a
    // direct command call doesn't surprise the user with a 200-file-
    // per-dir prompt that blows their token budget.
    let depth = sampling_depth.clamp(3, 30);

    let cfg = llm::ProviderConfig {
        api_key,
        endpoint,
        model: model.clone(),
    };
    let prov = llm::make_provider(&provider, cfg).map_err(String::from)?;

    // Snapshot scan + tags + root_path inside the project lock.
    // Drop the lock before any IO (toml read) or async work
    // (provider call) — same pattern as `llm_suggest_tags`.
    const SAMPLES_PER_TAG: usize = 5;
    let snapshot = project::with_mut(&project_id, |state| {
        let root = state.root_path.clone();
        let scan = state.cached_scan.clone().ok_or("Project hasn't been scanned yet")?;
        let tags_data = state.ensure_tags();
        let mut existing: Vec<llm::ExistingTagContext> =
            Vec::with_capacity(tags_data.tags.len());
        for tag in &tags_data.tags {
            let mut samples = tags_data.get_assets_with_tag(&tag.id);
            samples.truncate(SAMPLES_PER_TAG);
            existing.push(llm::ExistingTagContext {
                name: tag.name.clone(),
                description: tag.description.clone(),
                sample_paths: relativize_samples(samples, &root),
            });
        }
        Ok((root, scan, existing))
    })?;
    let (root_path, scan, existing_tags) = snapshot;

    // Read [project] meta outside the lock.
    let project_meta: Option<llm::project_meta::ProjectMeta> = {
        let toml_path = Path::new(&root_path).join("tidycraft.toml");
        std::fs::read_to_string(&toml_path)
            .ok()
            .and_then(|content| llm::project_meta::ProjectMeta::from_toml(&content).ok())
            .filter(|m| !m.is_empty())
    };

    // Deterministic-but-project-specific seed: hash the root path so
    // re-running on the same project gives the same samples, but two
    // different projects don't accidentally line up.
    let seed = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        root_path.hash(&mut h);
        h.finish()
    };
    let samples = llm::sampler::sample_directories(&scan, depth, seed);

    let request = llm::learning::LearnRequest {
        samples,
        project_meta,
        existing_tags,
        model: model.clone(),
        sampling_depth: depth,
        prompt_version: llm::learning::LEARNING_PROMPT_VERSION,
    };

    let result = prov.learn_project(&request).await.map_err(String::from)?;

    // Persist rules to <project>/tidycraft.ai.toml. Save errors are
    // non-fatal — the user already has the result in hand; worst case
    // they re-learn next time. We log via eprintln (no log crate yet).
    let doc = llm::rule_store::AiRulesDoc {
        last_learned: chrono::Utc::now().to_rfc3339(),
        prompt_version: llm::learning::LEARNING_PROMPT_VERSION,
        sampling_depth: depth,
        provider_used: provider,
        model_used: model,
        rules: result.rules.clone(),
    };
    if let Err(e) = doc.save(Path::new(&root_path)) {
        eprintln!("[learn_project_conventions] save tidycraft.ai.toml failed: {e}");
    }

    Ok(result)
}

/// Read the project's `tidycraft.ai.toml` if it exists. Frontend uses
/// this to populate the AITagPanel header status badge ("AI · 5d ago,
/// N rules") and to pre-fill LearnReviewPanel when the user clicks
/// Review without re-running the call.
#[tauri::command]
fn read_ai_rules(project_id: String) -> Result<Option<llm::rule_store::AiRulesDoc>, String> {
    project::with_ref(&project_id, |state| {
        llm::rule_store::AiRulesDoc::load(Path::new(&state.root_path))
    })
}

/// Persist a hand-edited rule list (e.g. user deleted unwanted rules in
/// LearnReviewPanel before applying). Preserves the original metadata
/// (last_learned, provider_used, etc.) loaded from disk; only `rules`
/// is replaced. If no doc exists yet (shouldn't happen — we only call
/// save after a successful learn), creates a fresh one with current
/// timestamp.
#[tauri::command]
fn save_ai_rules(
    project_id: String,
    rules: Vec<llm::learning::LearnedRule>,
) -> Result<(), String> {
    project::with_ref(&project_id, |state| {
        let root = Path::new(&state.root_path);
        let mut doc = llm::rule_store::AiRulesDoc::load(root)?.unwrap_or_else(|| {
            llm::rule_store::AiRulesDoc {
                last_learned: chrono::Utc::now().to_rfc3339(),
                prompt_version: llm::learning::LEARNING_PROMPT_VERSION,
                sampling_depth: 5,
                provider_used: "unknown".into(),
                model_used: "unknown".into(),
                rules: Vec::new(),
            }
        });
        doc.rules = rules;
        doc.save(root)
    })
}

/// Read the `[project]` block from `tidycraft.toml`. Frontend uses this
/// to pre-fill LearnSetupModal's theme/goal inputs from the project's
/// existing config. Empty / missing → returns defaults (`None` fields)
/// so the inputs render as placeholders.
#[tauri::command]
fn read_project_meta(project_id: String) -> Result<llm::project_meta::ProjectMeta, String> {
    project::with_ref(&project_id, |state| {
        let toml_path = Path::new(&state.root_path).join("tidycraft.toml");
        if !toml_path.exists() {
            return Ok(llm::project_meta::ProjectMeta::default());
        }
        let content = std::fs::read_to_string(&toml_path)
            .map_err(|e| format!("Failed to read tidycraft.toml: {e}"))?;
        llm::project_meta::ProjectMeta::from_toml(&content)
            .map_err(|e| format!("Failed to parse [project]: {e}"))
    })
}

/// Persist `theme` + `goal` from the LearnSetupModal into
/// `tidycraft.toml`'s `[project]` block. Uses `toml_edit` under the
/// hood so the user's analyzer-rule comments and other sections
/// survive the round-trip. Empty strings clear the fields
/// (template-style — keys remain but `from_toml` normalizes them
/// back to `None` so the prompt builder skips the context block).
///
/// Creates the file from `DEFAULT_CONFIG_TEMPLATE` if it doesn't
/// exist, mirroring `ensure_project_config`'s bootstrap path so
/// users hitting "Save" before ever opening the rules editor still
/// get the full annotated template.
#[tauri::command]
fn write_project_meta(
    project_id: String,
    theme: String,
    goal: String,
) -> Result<(), String> {
    project::with_ref(&project_id, |state| {
        llm::project_meta::write_back(Path::new(&state.root_path), &theme, &goal)
    })
}

#[tauri::command]
fn llm_cache_size() -> u64 {
    llm::cache::size()
}

/// Backend-sourced defaults for the AI Tagging settings panel. The
/// frontend reads these once at startup so model strings only live in
/// one place — bumping a default in `claude.rs` / `openai.rs` /
/// `ollama.rs` propagates to the UI without a parallel TS edit.
#[tauri::command]
fn llm_default_models() -> serde_json::Value {
    serde_json::json!({
        "claude": llm::claude::DEFAULT_MODEL,
        "openai": llm::openai::DEFAULT_MODEL,
        "ollama": llm::ollama::DEFAULT_MODEL,
        "ollama_endpoint": llm::ollama::DEFAULT_ENDPOINT,
    })
}

/// List the models installed on a local Ollama daemon. The endpoint
/// argument is the user's Settings-configured base URL — we strip any
/// path suffix and append `/api/tags`. Returns the raw model tag list
/// (e.g. `["qwen2.5vl:32b", "llama3.2-vision:11b-q4_K_M", "llava:7b"]`).
///
/// We do NOT filter for vision-capable models server-side — the API
/// doesn't expose the capability cleanly, and users may legitimately
/// want to pick text-only models for filename-based tagging. The UI
/// shows everything the user has installed and lets them choose.
#[tauri::command]
async fn llm_ollama_models(endpoint: String) -> Result<Vec<String>, String> {
    // Mirror the trim-and-append the provider does for /api/chat so
    // any endpoint shape the user typed in Settings still works:
    // `http://host:port` / `http://host:port/` / `http://host:port/api/tags`.
    let base = endpoint
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/api/tags")
        .trim_end_matches("/api/chat");
    let url = format!("{base}/api/tags");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_connect() {
            format!("Could not reach Ollama at {url} ({e})")
        } else if e.is_timeout() {
            format!("Ollama timed out at {url}")
        } else {
            e.to_string()
        }
    })?;

    if !resp.status().is_success() {
        return Err(format!("Ollama {} when listing models", resp.status()));
    }

    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<TagsModel>,
    }
    #[derive(serde::Deserialize)]
    struct TagsModel {
        name: String,
    }

    let parsed: TagsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Ollama /api/tags JSON: {e}"))?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

// ============ Analysis Commands ============

/// Load the project's `RuleConfig` from `<root>/tidycraft.toml` for the report
/// exporters, which (unlike `analyze_assets`) don't receive a config string from
/// the frontend. Behavior mirrors the UI path so a report can never silently
/// diverge from the Issues view:
/// - file absent → defaults (same as the frontend sending no config string)
/// - file present but unreadable or unparseable → `Err`, which the export
///   command propagates (the Issues view fails the same way via `analyze_assets`)
///
/// Previously a malformed file degraded to defaults here, so a JSON/HTML report
/// looked fine while quietly using default rules — the divergence this fixes.
fn load_rule_config(root_path: &str) -> Result<RuleConfig, String> {
    let toml_path = Path::new(root_path).join("tidycraft.toml");
    match std::fs::read_to_string(&toml_path) {
        Ok(content) => {
            RuleConfig::from_toml(&content).map_err(|e| format!("Invalid config: {}", e))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RuleConfig::default()),
        Err(e) => Err(format!("Failed to read tidycraft.toml: {}", e)),
    }
}

/// Build a `GlobSet` from `[ignore].patterns`, or `None` when the list is
/// empty. A malformed pattern surfaces as an `Err`; callers build this
/// before taking the project lock so the error short-circuits early.
fn build_ignore_set(config: &RuleConfig) -> Result<Option<globset::GlobSet>, String> {
    if config.ignore.patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &config.ignore.patterns {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| format!("Invalid ignore pattern '{}': {}", pattern, e))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| format!("Failed to build ignore set: {}", e))
}

/// The single source of truth for the analysis pipeline: apply the
/// `[ignore].patterns` filter, then run every analyzer phase — per-asset
/// rules plus the four cross-asset checks (duplicates, missing references,
/// PBR set, DCC source). `analyze_assets` (UI) and both report exporters
/// route through this so they always produce the same issue set for a given
/// project + config.
fn run_full_analysis(
    scan_result: &ScanResult,
    root_path: &str,
    config: &RuleConfig,
    ignore_set: Option<&globset::GlobSet>,
) -> AnalysisResult {
    // Only clone the scan when there are patterns to apply; most projects
    // have none and analyze the cached scan reference in place.
    let owned_filtered: Option<ScanResult> = ignore_set.map(|set| {
        let root = Path::new(root_path);
        let kept: Vec<scanner::AssetInfo> = scan_result
            .assets
            .iter()
            .filter(|a| {
                let path = Path::new(&a.path);
                let rel = path.strip_prefix(root).unwrap_or(path);
                !set.is_match(rel)
            })
            .cloned()
            .collect();
        ScanResult {
            root_path: scan_result.root_path.clone(),
            directory_tree: scan_result.directory_tree.clone(),
            assets: kept,
            total_count: scan_result.total_count,
            total_size: scan_result.total_size,
            type_counts: scan_result.type_counts.clone(),
            project_type: scan_result.project_type.clone(),
        }
    });
    let scan_to_analyze: &ScanResult = owned_filtered.as_ref().unwrap_or(scan_result);

    let analyzer = Analyzer::with_config(config);
    let mut result = analyzer.analyze(scan_to_analyze);
    let duplicates = analyzer.find_duplicates(scan_to_analyze);
    result.merge(duplicates);
    let missing = analyzer.find_missing_references(scan_to_analyze);
    result.merge(missing);
    let pbr = analyzer.find_pbr_set_issues(scan_to_analyze, &config.pbr_set);
    result.merge(pbr);
    let dcc = analyzer.find_dcc_source_issues(scan_to_analyze, &config.dcc_source);
    result.merge(dcc);
    result
}

#[tauri::command]
fn analyze_assets(project_id: String, config_toml: Option<String>) -> Result<AnalysisResult, String> {
    let config = if let Some(toml_str) = config_toml {
        RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
    } else {
        RuleConfig::default()
    };

    // Build the ignore matcher up-front so a malformed pattern surfaces as
    // an error before we touch the per-project lock.
    let ignore_set = build_ignore_set(&config)?;

    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;
        Ok(run_full_analysis(
            scan_result,
            &state.root_path,
            &config,
            ignore_set.as_ref(),
        ))
    })
}

#[tauri::command]
fn get_default_config() -> Result<String, String> {
    Ok(analyzer::rules::config_template::DEFAULT_CONFIG_TEMPLATE.to_string())
}

/// Make sure `<project_root>/tidycraft.toml` exists, writing the commented
/// default template if it doesn't, then return its absolute path. The
/// frontend hands that path to `open_with_default_app` so the user edits
/// in their preferred editor; saving and re-clicking Run Analysis is all
/// that's needed for changes to take effect.
#[tauri::command]
fn ensure_project_config(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let path = Path::new(&state.root_path).join("tidycraft.toml");
        if !path.exists() {
            std::fs::write(
                &path,
                analyzer::rules::config_template::DEFAULT_CONFIG_TEMPLATE,
            )
            .map_err(|e| format!("Failed to create tidycraft.toml: {}", e))?;
        }
        Ok(scanner::path_to_string(&path))
    })
}

#[tauri::command]
fn validate_config(config_toml: String) -> Result<bool, String> {
    match RuleConfig::from_toml(&config_toml) {
        Ok(_) => Ok(true),
        Err(e) => Err(format!("Invalid config: {}", e)),
    }
}

/// Read a project's `tidycraft.toml` from its registered root, if present.
/// Returns `Ok(None)` when the file doesn't exist (a normal state — most
/// projects use defaults), `Ok(Some(content))` on success, or `Err` for
/// IO failures. Validation/parsing happens later in `analyze_assets`.
#[tauri::command]
fn read_project_config(project_id: String) -> Result<Option<String>, String> {
    project::with_ref(&project_id, |state| {
        let path = Path::new(&state.root_path).join("tidycraft.toml");
        if !path.exists() {
            return Ok(None);
        }
        std::fs::read_to_string(&path)
            .map(Some)
            .map_err(|e| format!("Failed to read tidycraft.toml: {}", e))
    })
}

// ============ Tag Suggestions ============

#[tauri::command]
fn suggest_tags(project_id: String) -> Result<Vec<TagGroup>, String> {
    project::with_mut(&project_id, |state| {
        // Snapshot the names of tags already created (e.g. from a previous
        // suggest+apply round). We compare against `<group_name> (suggested)`
        // because applyGroup in the frontend always appends that suffix —
        // so a group whose suggested form is already in the tags list
        // would just create a duplicate-named tag if surfaced again.
        let already_suggested: std::collections::HashSet<String> = state
            .ensure_tags()
            .tags
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let scan = state.require_scan()?;
        let root = Path::new(&state.root_path);

        // Day 7: prefer AI-derived rules when present. RuleSuggester
        // produces TagGroup[] in the same shape so the frontend treats
        // both sources identically — only the `hint` string changes
        // (heuristic groups say "filename token", AI groups say
        // "ai · prefix Characters/Hero/" etc.).
        //
        // Fallback to heuristic suggester when:
        //   - no `tidycraft.ai.toml` exists yet (user hasn't run learning)
        //   - the file exists but the rule list is empty
        //   - the file is corrupt (load error) — we log + fall back
        //     rather than failing the whole call so AITagPanel still
        //     shows *something*.
        let mut groups: Vec<TagGroup> =
            match analyzer::rule_suggest::load_or_fallback(scan, root) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("[suggest_tags] AI rule load failed, falling back: {e}");
                    HeuristicSuggester.suggest(scan)
                }
            };

        groups.retain(|g| {
            !already_suggested.contains(&format!("{} (suggested)", g.name))
        });
        Ok(groups)
    })
}

// ============ Git Commands ============

#[tauri::command]
fn get_git_info(project_id: String, path: String) -> GitInfo {
    let manager = GitManager::open(Path::new(&path));
    let info = manager.get_info();

    let _ = project::with_mut(&project_id, |state| {
        state.git_manager = Some(manager);
        Ok(())
    });

    info
}

#[derive(Serialize)]
pub struct GitStatusMap {
    pub statuses: HashMap<String, String>,
}

#[tauri::command]
fn get_git_statuses(project_id: String) -> GitStatusMap {
    let statuses = project::with_mut(&project_id, |state| {
        let map = if let Some(manager) = state.git_manager.as_mut() {
            manager
                .get_all_statuses()
                .iter()
                .map(|(path, status)| {
                    // Normalize to forward slashes so keys match the scanner's
                    // asset paths on Windows. `repo.workdir().join(rel)` produces
                    // mixed `\`+`/` on Windows; without this the frontend lookup
                    // `gitStatuses[asset.path]` never hit.
                    (
                        scanner::path_to_string(path),
                        format!("{:?}", status).to_lowercase(),
                    )
                })
                .collect()
        } else {
            HashMap::new()
        };
        Ok(map)
    })
    .unwrap_or_default();

    GitStatusMap { statuses }
}

// ============ Unity Commands ============

#[tauri::command]
fn parse_unity_file(path: String) -> Result<unity::UnityFileInfo, String> {
    unity::parse_unity_file(Path::new(&path)).ok_or_else(|| "Failed to parse Unity file".to_string())
}

#[derive(Serialize)]
pub struct DependencyGraph {
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
}

/// One node in a project's dependency graph. `id` is the engine-neutral graph
/// identifier edges reference — a Unity GUID or a Godot `res://` path — while
/// `path` is the absolute filesystem path the frontend uses to locate the asset.
#[derive(Serialize)]
pub struct DependencyNode {
    pub id: String,
    pub path: String,
    pub name: String,
    pub file_type: String,
}

#[derive(Serialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
}

#[tauri::command]
fn get_unity_dependencies(project_id: String) -> Result<DependencyGraph, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        if !matches!(scan_result.project_type, Some(scanner::ProjectType::Unity)) {
            return Err("Not a Unity project".to_string());
        }

        let mut nodes: Vec<DependencyNode> = Vec::new();
        let mut edges: Vec<DependencyEdge> = Vec::new();
        let mut guid_to_path: HashMap<String, String> = HashMap::new();

        for asset in &scan_result.assets {
            if let Some(ref guid) = asset.unity_guid {
                guid_to_path.insert(guid.clone(), asset.path.clone());
                nodes.push(DependencyNode {
                    id: guid.clone(),
                    path: asset.path.clone(),
                    name: asset.name.clone(),
                    file_type: format!("{:?}", asset.asset_type).to_lowercase(),
                });
            }
        }

        for asset in &scan_result.assets {
            let ext = asset.extension.to_lowercase();
            if ext == "prefab" || ext == "unity" || ext == "mat" {
                if let Some(unity_info) = unity::parse_unity_file(Path::new(&asset.path)) {
                    if let Some(ref from_guid) = asset.unity_guid {
                        for reference in &unity_info.references {
                            if guid_to_path.contains_key(&reference.guid) {
                                edges.push(DependencyEdge {
                                    from: from_guid.clone(),
                                    to: reference.guid.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(DependencyGraph { nodes, edges })
    })
}

#[tauri::command]
fn find_unused_assets(project_id: String) -> Result<Vec<String>, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        match scan_result.project_type {
            // Godot uses res:// path refs, not GUIDs — dispatch to its own
            // parser and return early.
            Some(scanner::ProjectType::Godot) => {
                return Ok(godot::find_unused_godot_assets(
                    &state.root_path,
                    &scan_result.assets,
                ));
            }
            // Unity falls through to the GUID-based logic below.
            Some(scanner::ProjectType::Unity) => {}
            _ => {
                return Err(
                    "Unused-asset detection supports Unity and Godot projects".to_string(),
                )
            }
        }

        let mut referenced_guids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut all_guids: HashMap<String, String> = HashMap::new();

        for asset in &scan_result.assets {
            if let Some(ref guid) = asset.unity_guid {
                all_guids.insert(guid.clone(), asset.path.clone());
            }
        }

        for asset in &scan_result.assets {
            let ext = asset.extension.to_lowercase();
            if ext == "prefab" || ext == "unity" || ext == "mat" || ext == "controller" {
                if let Some(unity_info) = unity::parse_unity_file(Path::new(&asset.path)) {
                    for reference in &unity_info.references {
                        referenced_guids.insert(reference.guid.clone());
                    }
                }
            }
        }

        let unused: Vec<String> = all_guids
            .iter()
            .filter(|(guid, _path)| !referenced_guids.contains(*guid))
            .map(|(_guid, path)| path.clone())
            .collect();

        Ok(unused)
    })
}

/// Godot counterpart to `get_unity_dependencies`. Nodes are every non-metadata
/// asset keyed by its `res://` id; edges come from the `res://` references in
/// scenes / resources / scripts (target filtered to known nodes). Same parser
/// and known gaps as the unused-asset check (uid-only / dynamic `load()` missed).
#[tauri::command]
fn get_godot_dependencies(project_id: String) -> Result<DependencyGraph, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;
        if !matches!(scan_result.project_type, Some(scanner::ProjectType::Godot)) {
            return Err("Not a Godot project".to_string());
        }

        let root = Path::new(&state.root_path);
        let mut nodes: Vec<DependencyNode> = Vec::new();
        let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
        for asset in &scan_result.assets {
            if godot::is_godot_metadata(&asset.extension) {
                continue;
            }
            if let Some(id) = godot::asset_to_res_path(&asset.path, root) {
                known.insert(id.clone());
                nodes.push(DependencyNode {
                    id,
                    path: asset.path.clone(),
                    name: asset.name.clone(),
                    file_type: format!("{:?}", asset.asset_type).to_lowercase(),
                });
            }
        }

        let edges: Vec<DependencyEdge> = godot::godot_dependency_edges(root, &scan_result.assets)
            .into_iter()
            .filter(|(_from, to)| known.contains(to))
            .map(|(from, to)| DependencyEdge { from, to })
            .collect();

        Ok(DependencyGraph { nodes, edges })
    })
}

// ============ Statistics Commands ============

#[derive(Serialize)]
pub struct ProjectStats {
    pub total_assets: usize,
    pub total_size: u64,
    pub type_distribution: HashMap<String, usize>,
    pub size_distribution: HashMap<String, usize>,
    pub extension_distribution: HashMap<String, usize>,
    pub largest_files: Vec<FileInfo>,
    pub directory_sizes: HashMap<String, u64>,
}

#[derive(Serialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub asset_type: String,
}

#[tauri::command]
fn get_project_stats(project_id: String) -> Result<ProjectStats, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        let mut type_distribution: HashMap<String, usize> = HashMap::new();
        let mut size_distribution: HashMap<String, usize> = HashMap::new();
        let mut extension_distribution: HashMap<String, usize> = HashMap::new();
        let mut directory_sizes: HashMap<String, u64> = HashMap::new();
        let mut all_files: Vec<FileInfo> = Vec::new();

        for asset in &scan_result.assets {
            let type_str = format!("{:?}", asset.asset_type).to_lowercase();
            *type_distribution.entry(type_str.clone()).or_insert(0) += 1;

            *extension_distribution.entry(asset.extension.clone()).or_insert(0) += 1;

            let size_bucket = if asset.size < 1024 {
                "< 1 KB"
            } else if asset.size < 10 * 1024 {
                "1-10 KB"
            } else if asset.size < 100 * 1024 {
                "10-100 KB"
            } else if asset.size < 1024 * 1024 {
                "100 KB - 1 MB"
            } else if asset.size < 10 * 1024 * 1024 {
                "1-10 MB"
            } else {
                "> 10 MB"
            };
            *size_distribution.entry(size_bucket.to_string()).or_insert(0) += 1;

            if let Some(parent) = Path::new(&asset.path).parent() {
                let dir_str = parent.to_string_lossy().to_string();
                *directory_sizes.entry(dir_str).or_insert(0) += asset.size;
            }

            all_files.push(FileInfo {
                name: asset.name.clone(),
                path: asset.path.clone(),
                size: asset.size,
                asset_type: type_str,
            });
        }

        all_files.sort_by(|a, b| b.size.cmp(&a.size));
        let largest_files: Vec<FileInfo> = all_files.into_iter().take(10).collect();

        Ok(ProjectStats {
            total_assets: scan_result.total_count,
            total_size: scan_result.total_size,
            type_distribution,
            size_distribution,
            extension_distribution,
            largest_files,
            directory_sizes,
        })
    })
}

// ============ Export Commands ============

#[tauri::command]
fn export_to_json(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;
        serde_json::to_string_pretty(scan_result).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn export_to_csv(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        let mut csv = String::from("Name,Path,Type,Extension,Size,Width,Height\n");

        for asset in &scan_result.assets {
            let width = asset
                .metadata
                .as_ref()
                .and_then(|m| m.width)
                .map(|w| w.to_string())
                .unwrap_or_default();
            let height = asset
                .metadata
                .as_ref()
                .and_then(|m| m.height)
                .map(|h| h.to_string())
                .unwrap_or_default();

            csv.push_str(&format!(
                "\"{}\",\"{}\",{:?},\"{}\",{},{},{}\n",
                asset.name.replace('"', "\"\""),
                asset.path.replace('"', "\"\""),
                asset.asset_type,
                asset.extension.replace('"', "\"\""),
                asset.size,
                width,
                height
            ));
        }

        Ok(csv)
    })
}

#[tauri::command]
fn export_issues_to_json(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        // Mirror the UI's Run Analysis: honor the project's tidycraft.toml
        // (rule thresholds + [ignore].patterns) and run every phase,
        // including the PBR-set and DCC-source cross-asset checks. Without
        // this the exported report would silently diverge from the Issues
        // view under any custom config.
        let config = load_rule_config(&state.root_path)?;
        let ignore_set = build_ignore_set(&config)?;
        let result = run_full_analysis(scan_result, &state.root_path, &config, ignore_set.as_ref());

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn export_to_html(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        // Same analysis pipeline as Run Analysis / the JSON export, so the
        // HTML report's issue list matches the Issues view (custom config,
        // [ignore].patterns, PBR/DCC phases all applied). The asset
        // inventory cards below intentionally stay on the full scan —
        // [ignore].patterns scope analysis, not the project's file census.
        let config = load_rule_config(&state.root_path)?;
        let ignore_set = build_ignore_set(&config)?;
        let analysis_result =
            run_full_analysis(scan_result, &state.root_path, &config, ignore_set.as_ref());

        let mut type_counts: HashMap<String, usize> = HashMap::new();
        let mut size_by_type: HashMap<String, u64> = HashMap::new();

        for asset in &scan_result.assets {
            let type_str = format!("{:?}", asset.asset_type);
            *type_counts.entry(type_str.clone()).or_insert(0) += 1;
            *size_by_type.entry(type_str).or_insert(0) += asset.size;
        }

        fn format_size(bytes: u64) -> String {
            if bytes < 1024 {
                format!("{} B", bytes)
            } else if bytes < 1024 * 1024 {
                format!("{:.1} KB", bytes as f64 / 1024.0)
            } else if bytes < 1024 * 1024 * 1024 {
                format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
            } else {
                format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
            }
        }

        // "Passed" = assets with zero issues. `issue_count` counts ISSUES, not
        // assets, and one asset can raise several — so `total - issue_count`
        // under-counts and saturates to 0 on issue-heavy projects. Count the
        // DISTINCT asset paths that have an issue instead.
        let pass_count = {
            let with_issues: std::collections::HashSet<&str> = analysis_result
                .issues
                .iter()
                .map(|i| i.asset_path.as_str())
                .collect();
            scan_result.total_count.saturating_sub(with_issues.len())
        };

        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Tidycraft Report - {project_name}</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #1a1a2e; color: #e4e4e7; padding: 2rem; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #6366f1; margin-bottom: 0.5rem; }}
        h2 {{ color: #e4e4e7; margin: 2rem 0 1rem; border-bottom: 1px solid #3a3a5c; padding-bottom: 0.5rem; }}
        .meta {{ color: #9ca3af; margin-bottom: 2rem; }}
        .cards {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 2rem; }}
        .card {{ background: #252542; border-radius: 8px; padding: 1.5rem; border: 1px solid #3a3a5c; }}
        .card-value {{ font-size: 2rem; font-weight: bold; color: #6366f1; }}
        .card-label {{ color: #9ca3af; font-size: 0.875rem; margin-top: 0.25rem; }}
        table {{ width: 100%; border-collapse: collapse; background: #252542; border-radius: 8px; overflow: hidden; }}
        th, td {{ padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid #3a3a5c; }}
        th {{ background: #1a1a2e; font-weight: 600; }}
        tr:hover {{ background: #2a2a4a; }}
        .type-badge {{ display: inline-block; padding: 0.25rem 0.5rem; border-radius: 4px; font-size: 0.75rem; font-weight: 500; }}
        .texture {{ background: #4ade8020; color: #4ade80; }}
        .model {{ background: #60a5fa20; color: #60a5fa; }}
        .audio {{ background: #facc1520; color: #facc15; }}
        .video {{ background: #fb718520; color: #fb7185; }}
        .animation {{ background: #a78bfa20; color: #a78bfa; }}
        .material {{ background: #f472b620; color: #f472b6; }}
        .prefab {{ background: #22d3d120; color: #22d3d1; }}
        .scene {{ background: #fb923c20; color: #fb923c; }}
        .script {{ background: #ef444420; color: #ef4444; }}
        .data {{ background: #94a3b820; color: #94a3b8; }}
        .other {{ background: #6b728020; color: #9ca3af; }}
        .severity-error {{ color: #ef4444; }}
        .severity-warning {{ color: #f59e0b; }}
        .severity-info {{ color: #3b82f6; }}
        .chart {{ display: flex; gap: 2rem; margin-bottom: 2rem; }}
        .chart-bar {{ flex: 1; background: #252542; border-radius: 8px; padding: 1rem; }}
        .bar {{ height: 24px; background: #6366f1; border-radius: 4px; margin-bottom: 0.5rem; transition: width 0.3s; }}
        .bar-label {{ display: flex; justify-content: space-between; font-size: 0.875rem; color: #9ca3af; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Tidycraft Report</h1>
        <p class="meta">Project: {project_name} | Generated: {date}</p>

        <div class="cards">
            <div class="card">
                <div class="card-value">{total_assets}</div>
                <div class="card-label">Total Assets</div>
            </div>
            <div class="card">
                <div class="card-value">{total_size}</div>
                <div class="card-label">Total Size</div>
            </div>
            <div class="card">
                <div class="card-value">{issue_count}</div>
                <div class="card-label">Issues Found</div>
            </div>
            <div class="card">
                <div class="card-value">{pass_count}</div>
                <div class="card-label">Passed Checks</div>
            </div>
        </div>

        <h2>Asset Distribution</h2>
        <div class="chart">
            <div class="chart-bar">
                <h3 style="margin-bottom: 1rem; color: #9ca3af;">By Type</h3>
                {type_bars}
            </div>
        </div>

        <h2>Issues ({issue_count})</h2>
        <table>
            <thead>
                <tr>
                    <th>Severity</th>
                    <th>Rule</th>
                    <th>Asset</th>
                    <th>Message</th>
                </tr>
            </thead>
            <tbody>
                {issue_rows}
            </tbody>
        </table>

        <h2>Assets ({total_assets})</h2>
        <table>
            <thead>
                <tr>
                    <th>Name</th>
                    <th>Type</th>
                    <th>Size</th>
                    <th>Dimensions</th>
                </tr>
            </thead>
            <tbody>
                {asset_rows}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
            project_name = html_escape(
                scan_result
                    .root_path
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("Project")
            ),
            date = chrono::Local::now().format("%Y-%m-%d %H:%M"),
            total_assets = scan_result.total_count,
            total_size = format_size(scan_result.total_size),
            issue_count = analysis_result.issue_count,
            pass_count = pass_count,
            type_bars = {
                let max_count = type_counts.values().max().copied().unwrap_or(1) as f64;
                type_counts
                    .iter()
                    .map(|(t, c)| {
                        let pct = (*c as f64 / max_count * 100.0) as u32;
                        format!(
                            r#"<div><div class="bar" style="width: {}%"></div><div class="bar-label"><span>{}</span><span>{}</span></div></div>"#,
                            pct, t, c
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            issue_rows = {
                let total = analysis_result.issues.len();
                let mut rows: Vec<String> = analysis_result
                    .issues
                    .iter()
                    .take(100)
                    .map(|issue| {
                        let severity_class = match issue.severity {
                            analyzer::Severity::Error => "severity-error",
                            analyzer::Severity::Warning => "severity-warning",
                            analyzer::Severity::Info => "severity-info",
                        };
                        let file_name = issue
                            .asset_path
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(&issue.asset_path);
                        format!(
                            r#"<tr><td class="{}">{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                            severity_class,
                            issue.severity,
                            html_escape(&issue.rule_name),
                            html_escape(file_name),
                            html_escape(&issue.message)
                        )
                    })
                    .collect();
                if total > 100 {
                    rows.push(format!(
                        r#"<tr><td colspan="4" style="text-align:center;color:#9ca3af;font-style:italic;">Showing first 100 of {} issues — export to JSON for the complete list.</td></tr>"#,
                        total
                    ));
                }
                rows.join("\n")
            },
            asset_rows = {
                let total = scan_result.assets.len();
                let mut rows: Vec<String> = scan_result
                    .assets
                    .iter()
                    .take(500)
                    .map(|asset| {
                        let type_class = match asset.asset_type {
                            scanner::AssetType::Texture => "texture",
                            scanner::AssetType::Model => "model",
                            scanner::AssetType::Audio => "audio",
                            scanner::AssetType::Video => "video",
                            scanner::AssetType::Animation => "animation",
                            scanner::AssetType::Material => "material",
                            scanner::AssetType::Prefab => "prefab",
                            scanner::AssetType::Scene => "scene",
                            scanner::AssetType::Script => "script",
                            scanner::AssetType::Data => "data",
                            scanner::AssetType::Other => "other",
                        };
                        let dimensions = asset
                            .metadata
                            .as_ref()
                            .and_then(|m| m.width.zip(m.height))
                            .map(|(w, h)| format!("{}x{}", w, h))
                            .unwrap_or_else(|| "-".to_string());
                        format!(
                            r#"<tr><td>{}</td><td><span class="type-badge {}">{:?}</span></td><td>{}</td><td>{}</td></tr>"#,
                            html_escape(&asset.name),
                            type_class,
                            asset.asset_type,
                            format_size(asset.size),
                            dimensions
                        )
                    })
                    .collect();
                if total > 500 {
                    rows.push(format!(
                        r#"<tr><td colspan="4" style="text-align:center;color:#9ca3af;font-style:italic;">Showing first 500 of {} assets — export to CSV or JSON for the complete list.</td></tr>"#,
                        total
                    ));
                }
                rows.join("\n")
            }
        );

        Ok(html)
    })
}

// ============ Batch Operations ============

#[derive(serde::Deserialize)]
pub enum RenameOperation {
    FindReplace { find: String, replace: String },
    AddPrefix { prefix: String },
    AddSuffix { suffix: String },
    RemovePrefix { prefix: String },
    RemoveSuffix { suffix: String },
    ToLowercase,
    ToUppercase,
    ToTitleCase,
}

#[derive(Serialize)]
pub struct RenamePreview {
    pub original_path: String,
    pub original_name: String,
    pub new_name: String,
    pub will_change: bool,
}

#[derive(Serialize)]
pub struct BatchRenameResult {
    pub success_count: usize,
    pub error_count: usize,
    pub errors: Vec<String>,
}

fn apply_rename_operation(name: &str, operation: &RenameOperation) -> String {
    match operation {
        RenameOperation::FindReplace { find, replace } => name.replace(find, replace),
        RenameOperation::AddPrefix { prefix } => format!("{}{}", prefix, name),
        RenameOperation::AddSuffix { suffix } => {
            if let Some(dot_pos) = name.rfind('.') {
                format!("{}{}{}", &name[..dot_pos], suffix, &name[dot_pos..])
            } else {
                format!("{}{}", name, suffix)
            }
        }
        RenameOperation::RemovePrefix { prefix } => {
            name.strip_prefix(prefix).unwrap_or(name).to_string()
        }
        RenameOperation::RemoveSuffix { suffix } => {
            if let Some(dot_pos) = name.rfind('.') {
                let base = &name[..dot_pos];
                let ext = &name[dot_pos..];
                let new_base = base.strip_suffix(suffix).unwrap_or(base);
                format!("{}{}", new_base, ext)
            } else {
                name.strip_suffix(suffix).unwrap_or(name).to_string()
            }
        }
        RenameOperation::ToLowercase => name.to_lowercase(),
        RenameOperation::ToUppercase => name.to_uppercase(),
        RenameOperation::ToTitleCase => name
            .split(|c: char| c == '_' || c == '-' || c == ' ')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join("_"),
    }
}

#[tauri::command]
fn preview_batch_rename(paths: Vec<String>, operation: RenameOperation) -> Vec<RenamePreview> {
    paths
        .into_iter()
        .map(|path| {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let new_name = apply_rename_operation(&name, &operation);
            let will_change = name != new_name;

            RenamePreview {
                original_path: path,
                original_name: name,
                new_name,
                will_change,
            }
        })
        .collect()
}

#[tauri::command]
fn execute_batch_rename(
    project_id: String,
    paths: Vec<String>,
    operation: RenameOperation,
) -> BatchRenameResult {
    let mut success_count = 0;
    let mut error_count = 0;
    let mut errors = Vec::new();
    let mut paths_to_record: Vec<(String, String)> = Vec::new();

    for path in paths {
        let path_obj = Path::new(&path);
        let name = match path_obj.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                errors.push(format!("Invalid path: {}", path));
                error_count += 1;
                continue;
            }
        };

        let new_name = apply_rename_operation(&name, &operation);

        if name == new_name {
            continue;
        }

        let new_path = path_obj.with_file_name(&new_name);

        if new_path.exists() {
            errors.push(format!("Target already exists: {}", new_path.display()));
            error_count += 1;
            continue;
        }

        match std::fs::rename(&path, &new_path) {
            Ok(_) => {
                // Carry the Unity .meta sidecar so renamed assets keep their
                // GUID. Best-effort: no-op without a sidecar, logs on failure.
                if let Err(e) = meta_sidecar::carry_on_rename(path_obj, &new_path) {
                    eprintln!("[batch_rename] .meta sidecar not carried for {}: {}", path, e);
                }
                success_count += 1;
                // Normalize the new path to forward slashes (scanner::path_to_string)
                // so the undo record and the tag binding below key off the same
                // string the next scan will produce — a raw to_string_lossy() keeps
                // Windows backslashes and the tag key would never match.
                paths_to_record.push((path.clone(), scanner::path_to_string(&new_path)));
            }
            Err(e) => {
                errors.push(format!("Failed to rename {}: {}", name, e));
                error_count += 1;
            }
        }
    }

    if success_count > 0 && !paths_to_record.is_empty() {
        let ts = unix_timestamp();
        let file_ops: Vec<undo::FileOperation> = paths_to_record
            .iter()
            .map(|(original, new_path)| undo::FileOperation {
                operation_type: undo::OperationType::Rename,
                original_path: original.clone(),
                new_path: Some(new_path.clone()),
                timestamp: ts,
            })
            .collect();

        let _ = project::with_mut(&project_id, |state| {
            state.undo_manager.record_batch(
                format!("Batch rename: {} files", file_ops.len()),
                file_ops,
            );

            // Tags follow the file across renames — same as move_assets /
            // rename_file. Without this, the watcher's later orphan cleanup
            // reaps the old-path bindings and the tags are lost. Paths are
            // already normalized (scanner::path_to_string) so the new key
            // matches what the next scan produces for the renamed file.
            if state.tags_data.is_some() {
                let tags = state.ensure_tags();
                for (original, new_path) in &paths_to_record {
                    tags.rename_path(original, new_path);
                }
                let _ = state.save_tags();
            }
            Ok(())
        });
    }

    BatchRenameResult {
        success_count,
        error_count,
        errors,
    }
}

// ============ Unreal Engine Commands ============

#[tauri::command]
fn get_unreal_project_info(path: String) -> Result<unreal::UnrealProjectInfo, String> {
    let root_path = Path::new(&path);

    let uproject_path = unreal::find_uproject_file(root_path)
        .or_else(|| {
            if path.ends_with(".uproject") {
                Some(root_path.to_path_buf())
            } else {
                None
            }
        })
        .ok_or("No .uproject file found")?;

    unreal::parse_uproject(&uproject_path).ok_or_else(|| "Failed to parse .uproject file".to_string())
}

// ============ Godot Commands ============

#[tauri::command]
fn get_godot_project_info(path: String) -> Result<godot::GodotProjectInfo, String> {
    let root_path = Path::new(&path);

    let project_path = if path.ends_with("project.godot") {
        root_path.to_path_buf()
    } else {
        root_path.join("project.godot")
    };

    if !project_path.exists() {
        return Err("No project.godot file found".to_string());
    }

    godot::parse_project_godot(&project_path).ok_or_else(|| "Failed to parse project.godot file".to_string())
}

// ============ File System Commands ============

/// Open the OS file manager focused on `path` (Finder reveal / Explorer
/// `/select,` / xdg-open parent). We keep the per-OS dispatch here because
/// `tauri-plugin-shell::open` has no "select-this-file" mode — it can only
/// open a file/url, not highlight it inside a folder view.
#[tauri::command]
fn show_in_file_manager(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        // Two quirks of explorer's `/select,` we kept stepping on:
        //   1. The flag and path must be a SINGLE cmdline argument
        //      (`/select,C:\foo`). `Command::args(["/select,", &path])`
        //      inserts a space between them and explorer interprets that
        //      as "open the grandparent and select the parent folder",
        //      which is what users were seeing.
        //   2. `/select,` only follows backslash-separator paths.
        //      `path_to_string` normalizes to `/` for cross-platform
        //      consistency, so undo it here at the boundary.
        let win_path = path.replace('/', "\\");
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", win_path))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Launch a file with the OS-default application associated to its
/// extension. Routed through `tauri-plugin-opener` so Windows codepage,
/// path quoting, and `%` variable expansion are handled by the platform
/// shell helper — previous hand-rolled `cmd /C start` worked for ASCII
/// paths but broke on Chinese / `%`-containing paths.
#[tauri::command]
fn open_with_default_app(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Open a file with a specific external application — `editor` is the
/// absolute path to a binary or .app bundle (`Photoshop.exe`,
/// `/Applications/Blender.app`, …). Errors bubble up to the caller as a
/// string for inline UI display.
#[tauri::command]
fn open_in_editor(app: tauri::AppHandle, path: String, editor: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(&path, Some(editor.as_str()))
        .map_err(|e| e.to_string())
}

// ============ Texture resolution for 3D model loaders ============
//
// FBX/OBJ/DAE files often embed texture filenames without a directory part
// (e.g. just "colormap.png"), or with a directory that was valid on the author's
// machine but is wrong for the recipient. When Three.js's loaders ask for such a
// texture, the Tauri asset protocol returns 500. We pre-walk common sibling
// directories (`Textures/`, `Materials/`, etc.) for the model and return a
// filename → absolute-path lookup that the frontend uses in its URL modifier.

const TEXTURE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "tga", "bmp", "gif",
    "dds", "hdr", "exr", "tif", "tiff", "webp", "psd",
];

/// Subdirs to scan below the model's own directory.
const SIBLING_SUBDIRS: &[&str] = &[
    "",
    "Textures", "textures",
    "Texture", "texture",
    "Materials", "materials",
    "Material", "material",
    "Maps", "maps",
    "Tex", "tex",
    "Images", "images",
];

/// Subdirs to scan below the model's *parent* directory (for layouts where the
/// textures live as a sibling of the model folder, e.g. `Models/foo.fbx` +
/// `Textures/tex.png`).
const PARENT_SUBDIRS: &[&str] = &[
    "Textures", "textures",
    "Texture", "texture",
    "Materials", "materials",
    "Maps", "maps",
];

fn collect_texture_files(dir: &Path, out: &mut HashMap<String, String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_lowercase(),
            None => continue,
        };
        if !TEXTURE_EXTS.iter().any(|&e| e == ext) {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_lowercase(),
            None => continue,
        };
        // First hit wins — callers walk dirs in preference order so that a
        // model-local texture beats a neighboring-folder duplicate.
        out.entry(filename)
            .or_insert_with(|| scanner::path_to_string(&path));
    }
}

#[tauri::command]
fn resolve_texture_siblings(model_path: String) -> HashMap<String, String> {
    let model = Path::new(&model_path);
    let model_dir = match model.parent() {
        Some(p) => p.to_path_buf(),
        None => return HashMap::new(),
    };

    let mut result: HashMap<String, String> = HashMap::new();

    for subdir in SIBLING_SUBDIRS {
        let dir = if subdir.is_empty() {
            model_dir.clone()
        } else {
            model_dir.join(subdir)
        };
        collect_texture_files(&dir, &mut result);
    }

    if let Some(parent) = model_dir.parent() {
        for subdir in PARENT_SUBDIRS {
            collect_texture_files(&parent.join(subdir), &mut result);
        }
    }

    result
}

#[derive(Serialize)]
pub struct DeleteError {
    pub path: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct DeleteResult {
    pub success_paths: Vec<String>,
    pub errors: Vec<DeleteError>,
}

// ============ Move / Copy / Duplicate ============

#[derive(Serialize)]
pub struct FileOpError {
    pub path: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct FileOpSuccess {
    pub original_path: String,
    pub new_path: String,
}

#[derive(Serialize)]
pub struct FileOpResult {
    pub successes: Vec<FileOpSuccess>,
    pub errors: Vec<FileOpError>,
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Move each path into `target_dir`. Per-file rename; target must not already
/// exist at the destination. Successful moves are batched into the project's
/// undo manager so the user can revert.
#[tauri::command]
fn move_assets(
    project_id: String,
    paths: Vec<String>,
    target_dir: String,
) -> FileOpResult {
    let mut successes: Vec<FileOpSuccess> = Vec::new();
    let mut errors: Vec<FileOpError> = Vec::new();

    let target = Path::new(&target_dir);
    if !target.is_dir() {
        errors.push(FileOpError {
            path: target_dir.clone(),
            message: "Target is not a directory".to_string(),
        });
        return FileOpResult { successes, errors };
    }

    for path in paths {
        let src = Path::new(&path);
        let name = match src.file_name() {
            Some(n) => n.to_os_string(),
            None => {
                errors.push(FileOpError {
                    path: path.clone(),
                    message: "Invalid source path".to_string(),
                });
                continue;
            }
        };
        let dst = target.join(&name);

        if src == dst {
            // No-op: source already in target directory. Skip silently.
            continue;
        }
        if dst.exists() {
            errors.push(FileOpError {
                path: path.clone(),
                message: format!("Target already exists: {}", scanner::path_to_string(&dst)),
            });
            continue;
        }

        match std::fs::rename(src, &dst) {
            Ok(_) => {
                // Carry the Unity .meta sidecar so moved assets keep their
                // GUID. Best-effort: no-op without a sidecar, logs on failure.
                if let Err(e) = meta_sidecar::carry_on_rename(src, &dst) {
                    eprintln!("[move_assets] .meta sidecar not carried for {}: {}", path, e);
                }
                successes.push(FileOpSuccess {
                    original_path: path,
                    new_path: scanner::path_to_string(&dst),
                })
            }
            Err(e) => errors.push(FileOpError {
                path,
                message: e.to_string(),
            }),
        }
    }

    if !successes.is_empty() {
        let ts = unix_timestamp();
        let ops: Vec<undo::FileOperation> = successes
            .iter()
            .map(|s| undo::FileOperation {
                operation_type: undo::OperationType::Move,
                original_path: s.original_path.clone(),
                new_path: Some(s.new_path.clone()),
                timestamp: ts,
            })
            .collect();
        let _ = project::with_mut(&project_id, |state| {
            state.undo_manager.record_batch(
                format!("Move {} file(s)", ops.len()),
                ops,
            );

            // Tags follow the file across moves. Skip if tags haven't
            // been touched in this session (lazy load). Save errors
            // are swallowed — the move itself already succeeded.
            if state.tags_data.is_some() {
                let tags = state.ensure_tags();
                for s in &successes {
                    tags.rename_path(&s.original_path, &s.new_path);
                }
                let _ = state.save_tags();
            }
            Ok(())
        });
    }

    FileOpResult { successes, errors }
}

/// Copy each path into `target_dir`. Fails on collision (unlike duplicate).
/// No undo recording — user can just delete the copies if they're unwanted.
#[tauri::command]
fn copy_assets(paths: Vec<String>, target_dir: String) -> FileOpResult {
    let mut successes: Vec<FileOpSuccess> = Vec::new();
    let mut errors: Vec<FileOpError> = Vec::new();

    let target = Path::new(&target_dir);
    if !target.is_dir() {
        errors.push(FileOpError {
            path: target_dir.clone(),
            message: "Target is not a directory".to_string(),
        });
        return FileOpResult { successes, errors };
    }

    for path in paths {
        let src = Path::new(&path);
        let name = match src.file_name() {
            Some(n) => n.to_os_string(),
            None => {
                errors.push(FileOpError {
                    path: path.clone(),
                    message: "Invalid source path".to_string(),
                });
                continue;
            }
        };
        let dst = target.join(&name);

        if dst.exists() {
            errors.push(FileOpError {
                path: path.clone(),
                message: format!(
                    "Target already exists: {} (use Duplicate for same-name copies)",
                    scanner::path_to_string(&dst)
                ),
            });
            continue;
        }

        match std::fs::copy(src, &dst) {
            Ok(_) => successes.push(FileOpSuccess {
                original_path: path,
                new_path: scanner::path_to_string(&dst),
            }),
            Err(e) => errors.push(FileOpError {
                path,
                message: e.to_string(),
            }),
        }
    }

    FileOpResult { successes, errors }
}

/// Build a sibling path by adding " copy" (and a counter if needed) before the
/// extension. Matches macOS Finder's convention; works on all platforms.
fn unique_copy_path(src: &Path) -> Option<std::path::PathBuf> {
    let parent = src.parent()?;
    let stem = src.file_stem().and_then(|s| s.to_str())?.to_string();
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();

    let first = parent.join(format!("{} copy{}", stem, ext));
    if !first.exists() {
        return Some(first);
    }
    for i in 2..1000 {
        let candidate = parent.join(format!("{} copy {}{}", stem, i, ext));
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    // Extreme fallback — timestamp suffix guarantees uniqueness.
    Some(parent.join(format!("{} copy {}{}", stem, unix_timestamp(), ext)))
}

/// Create an in-place copy of each file with an auto-suffixed name (`foo.png`
/// → `foo copy.png`, `foo copy 2.png`, …). No undo — trash the copies if unwanted.
#[tauri::command]
fn duplicate_assets(paths: Vec<String>) -> FileOpResult {
    let mut successes: Vec<FileOpSuccess> = Vec::new();
    let mut errors: Vec<FileOpError> = Vec::new();

    for path in paths {
        let src = Path::new(&path);
        if !src.is_file() {
            errors.push(FileOpError {
                path: path.clone(),
                message: "Source is not a regular file".to_string(),
            });
            continue;
        }
        let dst = match unique_copy_path(src) {
            Some(d) => d,
            None => {
                errors.push(FileOpError {
                    path: path.clone(),
                    message: "Cannot derive duplicate name (no parent or bad stem)".to_string(),
                });
                continue;
            }
        };

        match std::fs::copy(src, &dst) {
            Ok(_) => successes.push(FileOpSuccess {
                original_path: path,
                new_path: scanner::path_to_string(&dst),
            }),
            Err(e) => errors.push(FileOpError {
                path,
                message: e.to_string(),
            }),
        }
    }

    FileOpResult { successes, errors }
}

/// Send each path to the OS recycle bin / trash. Per-path success/error is
/// reported separately so the UI can show partial results (e.g. some files on
/// a network drive that doesn't support trash).
///
/// No `project_id` parameter: the filesystem watcher will pick up the resulting
/// remove events and update `scanResult.assets` automatically.
#[tauri::command]
fn delete_assets(paths: Vec<String>) -> DeleteResult {
    let mut success_paths = Vec::new();
    let mut errors = Vec::new();

    for path in paths {
        match trash::delete(&path) {
            Ok(_) => {
                // Also trash the Unity .meta sidecar so deleting an asset
                // doesn't strand its sidecar. Best-effort: no-op without a
                // sidecar, logs on failure.
                if let Err(e) = meta_sidecar::carry_on_delete(Path::new(&path)) {
                    eprintln!("[delete_assets] .meta sidecar not carried for {}: {}", path, e);
                }
                success_paths.push(path);
            }
            Err(e) => errors.push(DeleteError {
                path,
                message: e.to_string(),
            }),
        }
    }

    DeleteResult {
        success_paths,
        errors,
    }
}

#[tauri::command]
fn rename_file(project_id: String, old_path: String, new_name: String) -> Result<String, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let old_path_ref = Path::new(&old_path);
    if !old_path_ref.exists() {
        return Err("File does not exist".to_string());
    }

    let parent = old_path_ref.parent().ok_or("Cannot get parent directory")?;
    let new_path = parent.join(&new_name);

    if new_path.exists() {
        return Err("A file with this name already exists".to_string());
    }

    let old_name = old_path_ref
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // Normalize to forward slashes so the returned path, the undo record, and
    // the tag binding all match what the scanner produces — `to_string_lossy`
    // would keep Windows backslashes (e.g. `C:/dir\new.png`).
    let new_path_str = scanner::path_to_string(&new_path);

    std::fs::rename(old_path_ref, &new_path).map_err(|e| e.to_string())?;

    // Carry the Unity .meta sidecar so the renamed asset keeps its GUID and
    // references don't break. Best-effort: a missing sidecar (non-Unity) is a
    // no-op, and a carry failure only logs — the rename already succeeded.
    if let Err(e) = meta_sidecar::carry_on_rename(old_path_ref, &new_path) {
        eprintln!("[rename_file] .meta sidecar not carried for {}: {}", old_path, e);
    }

    let _ = project::with_mut(&project_id, |state| {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let operation = undo::FileOperation {
            operation_type: undo::OperationType::Rename,
            original_path: old_path.clone(),
            new_path: Some(new_path_str.clone()),
            timestamp,
        };

        state
            .undo_manager
            .record_batch(format!("Rename {} to {}", old_name, new_name), vec![operation]);

        // Carry tags from the old path to the new one. Best-effort —
        // tag bookkeeping must never block a successful rename, so we
        // ignore save errors (the file is already renamed on disk).
        if state.tags_data.is_some() {
            // new_path_str is already normalized (scanner::path_to_string above).
            state.ensure_tags().rename_path(&old_path, &new_path_str);
            let _ = state.save_tags();
        }
        Ok(())
    });

    Ok(new_path_str)
}

// ============ Undo Commands ============

/// After an undo renames files back to their original paths, carry their tag
/// bindings the same direction (new_path → original_path), mirroring the
/// forward-direction carry in `move_assets` / `rename_file`. Only carries a
/// binding when the file actually arrived at `original` — a file whose undo
/// failed (e.g. its target already existed) stays at `new_path`, so its binding
/// must stay there too. No-op when tags were never loaded this session (the
/// same lazy-load guard the forward ops and the watcher cleanup use).
fn carry_tags_after_undo(state: &mut project::ProjectState, pairs: &[(String, String)]) {
    if pairs.is_empty() || state.tags_data.is_none() {
        return;
    }
    let tags = state.ensure_tags();
    for (original, new_path) in pairs {
        if Path::new(original).exists() {
            tags.rename_path(new_path, original);
        }
    }
    let _ = state.save_tags();
}

#[tauri::command]
fn get_undo_history(project_id: String) -> Vec<undo::HistoryEntry> {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.get_history())).unwrap_or_default()
}

#[tauri::command]
fn undo_last_operation(project_id: String) -> Result<undo::UndoResult, String> {
    project::with_mut(&project_id, |state| {
        // Snapshot the path pairs of the batch about to be reverted BEFORE the
        // undo runs, so we can carry tag bindings back to the original paths
        // afterwards (undo.rs has no access to TagsData).
        let pairs = state.undo_manager.peek_last_undoable_pairs();
        let result = state
            .undo_manager
            .undo_last()
            .ok_or_else(|| "No operation to undo".to_string())?;
        carry_tags_after_undo(state, &pairs);
        Ok(result)
    })
}

#[tauri::command]
fn undo_operation_by_id(project_id: String, id: String) -> Result<undo::UndoResult, String> {
    project::with_mut(&project_id, |state| {
        let pairs = state.undo_manager.peek_pairs_by_id(&id);
        let result = state
            .undo_manager
            .undo_by_id(&id)
            .ok_or_else(|| format!("Operation '{}' not found or already undone", id))?;
        carry_tags_after_undo(state, &pairs);
        Ok(result)
    })
}

#[tauri::command]
fn can_undo(project_id: String) -> bool {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.can_undo())).unwrap_or(false)
}

#[tauri::command]
fn clear_undo_history(project_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.undo_manager.clear_history();
        Ok(())
    })
}

#[tauri::command]
fn get_undo_count(project_id: String) -> usize {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.undoable_count())).unwrap_or(0)
}

// ============ Tags Commands ============

#[tauri::command]
fn get_all_tags(project_id: String) -> Result<Vec<tags::Tag>, String> {
    project::with_mut(&project_id, |state| Ok(state.ensure_tags().tags.clone()))
}

#[tauri::command]
fn create_tag(project_id: String, name: String, color: String) -> Result<tags::Tag, String> {
    project::with_mut(&project_id, |state| {
        let tag = state.ensure_tags().create_tag(name, color);
        state.save_tags()?;
        Ok(tag)
    })
}

#[tauri::command]
fn update_tag(
    project_id: String,
    tag_id: String,
    name: Option<String>,
    color: Option<String>,
    // `Option<Option<String>>` lets the frontend send three states:
    //   omitted        → don't touch description (Option = None outer)
    //   null           → clear description (Some(None))
    //   "some text"    → set description (Some(Some(s)))
    description: Option<Option<String>>,
) -> Result<tags::Tag, String> {
    project::with_mut(&project_id, |state| {
        let tag = state
            .ensure_tags()
            .update_tag(&tag_id, name, color, description)
            .ok_or("Tag not found")?;
        state.save_tags()?;
        Ok(tag)
    })
}

#[tauri::command]
fn delete_tag(project_id: String, tag_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.ensure_tags().delete_tag(&tag_id);
        state.save_tags()
    })
}

#[tauri::command]
fn get_asset_tags(project_id: String, asset_path: String) -> Result<Vec<tags::Tag>, String> {
    project::with_mut(&project_id, |state| {
        Ok(state.ensure_tags().get_asset_tags(&asset_path))
    })
}

#[tauri::command]
fn add_tag_to_asset(project_id: String, asset_path: String, tag_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.ensure_tags().add_tag_to_asset(&asset_path, &tag_id);
        state.save_tags()
    })
}

#[tauri::command]
fn remove_tag_from_asset(
    project_id: String,
    asset_path: String,
    tag_id: String,
) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.ensure_tags().remove_tag_from_asset(&asset_path, &tag_id);
        state.save_tags()
    })
}

#[tauri::command]
fn add_tag_to_assets(
    project_id: String,
    asset_paths: Vec<String>,
    tag_id: String,
) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        let tags = state.ensure_tags();
        for path in asset_paths {
            tags.add_tag_to_asset(&path, &tag_id);
        }
        state.save_tags()
    })
}

#[tauri::command]
fn get_all_asset_tags(project_id: String) -> Result<HashMap<String, Vec<tags::Tag>>, String> {
    project::with_mut(&project_id, |state| {
        let tags = state.ensure_tags();
        let mut result: HashMap<String, Vec<tags::Tag>> = HashMap::new();
        let paths: Vec<String> = tags.asset_tags.keys().cloned().collect();
        for path in paths {
            let asset_tags = tags.get_asset_tags(&path);
            if !asset_tags.is_empty() {
                result.insert(path, asset_tags);
            }
        }
        Ok(result)
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            #[cfg(debug_assertions)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Project lifecycle
            register_project,
            unregister_project,
            // Scan
            scan_project,
            scan_project_async,
            scan_project_incremental,
            cancel_scan,
            get_scan_progress,
            clear_scan_cache,
            start_watching,
            stop_watching,
            get_thumbnail,
            get_thumbnail_cache_size,
            clear_thumbnail_cache,
            // Analysis
            analyze_assets,
            get_default_config,
            validate_config,
            read_project_config,
            ensure_project_config,
            suggest_tags,
            // Git
            get_git_info,
            get_git_statuses,
            // Unity
            parse_unity_file,
            get_unity_dependencies,
            find_unused_assets,
            get_godot_dependencies,
            // Stats / export
            get_project_stats,
            export_to_json,
            export_to_csv,
            export_issues_to_json,
            export_to_html,
            // Batch ops
            preview_batch_rename,
            execute_batch_rename,
            // Engine info
            get_unreal_project_info,
            get_godot_project_info,
            // Undo
            get_undo_history,
            undo_last_operation,
            undo_operation_by_id,
            can_undo,
            clear_undo_history,
            get_undo_count,
            // File System
            show_in_file_manager,
            open_with_default_app,
            open_in_editor,
            rename_file,
            delete_assets,
            move_assets,
            copy_assets,
            duplicate_assets,
            resolve_texture_siblings,
            // Tags
            get_all_tags,
            create_tag,
            update_tag,
            delete_tag,
            get_asset_tags,
            add_tag_to_asset,
            remove_tag_from_asset,
            add_tag_to_assets,
            get_all_asset_tags,
            // LLM tagging
            llm_estimate_cost,
            llm_suggest_tags,
            llm_clear_cache,
            llm_cache_size,
            llm_default_models,
            llm_ollama_models,
            learn_project_conventions,
            read_ai_rules,
            save_ai_rules,
            read_project_meta,
            write_project_meta
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relativize_samples_strips_absolute_prefix() {
        // Existing-tag samples are keyed by absolute scan paths. They must be
        // relativized before they reach an LLM prompt or the cache key, or we
        // leak the user's drive/username/layout to the provider.
        let root = "C:/Users/alice/proj";
        let rel = relativize_samples(
            vec![
                "C:/Users/alice/proj/Textures/hero.png".to_string(),
                "C:/Users/alice/proj/Audio/step.wav".to_string(),
            ],
            root,
        );
        assert_eq!(rel, vec!["Textures/hero.png", "Audio/step.wav"]);
        // No absolute markers survive into the prompt context.
        for p in &rel {
            assert!(!p.contains("C:"), "leaked drive letter: {p}");
            assert!(!p.contains("alice"), "leaked username: {p}");
        }
    }

    #[test]
    fn relativize_samples_falls_back_to_basename_outside_root() {
        // A path that isn't under the project root degrades to its basename
        // rather than shipping the full absolute path.
        let rel = relativize_samples(vec!["D:/elsewhere/x.png".to_string()], "C:/proj");
        assert_eq!(rel, vec!["x.png"]);
    }

    #[test]
    fn html_escape_neutralizes_markup() {
        // An asset named to inject script must not produce live HTML.
        let escaped = html_escape(r#"<img src=x onerror="alert(1)">.png"#);
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert_eq!(
            escaped,
            "&lt;img src=x onerror=&quot;alert(1)&quot;&gt;.png"
        );
    }
}
