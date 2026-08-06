mod analyzer;
mod cache;
mod fs_atomic;
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
use analyzer::{AnalysisResult, Analyzer};
use cache::ScanCache;
use git::{GitInfo, GitManager};
use scanner::{IncrementalStats, ScanResult, ScanState};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

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
fn cancel_scan(project_id: String) -> bool {
    project::with_ref(&project_id, |s| {
        Ok(s.scan_state.as_ref().map(|st| st.cancel()).is_some())
    })
    .unwrap_or(false)
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
/// One quoted CSV cell built from a project-derived string (asset names,
/// paths, extensions — none of which the user chose).
///
/// Quoting alone is not enough: spreadsheets read a leading `=`, `+`, `-`,
/// `@`, tab or CR as the start of a *formula*, quotes and all, so a file
/// named `=cmd|'/c calc'!A1.png` — a legal name on every filesystem — runs
/// when the exported sheet is opened. A leading apostrophe is the standard
/// neutralizer: the cell then reads as literal text and displays unchanged.
/// This is the same threat the HTML report handles with [`html_escape`];
/// only the CSV side was missing it.
fn csv_cell(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    let leads_a_formula = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let prefix = if leads_a_formula { "'" } else { "" };
    format!("\"{prefix}{escaped}\"")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Substitute `{{name}}` placeholders, i18next's syntax, so one locale
/// template string works in the webview and in the HTML export alike.
/// Unknown names are left in place — same as i18next, so a broken template
/// fails identically at both ends instead of two different ways.
fn render_template(template: &str, args: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let (head, tail) = rest.split_at(start);
        out.push_str(head);
        match tail[2..].find("}}") {
            Some(end) => {
                let name = &tail[2..2 + end];
                match args.get(name.trim()) {
                    Some(value) => out.push_str(value),
                    None => out.push_str(&tail[..2 + end + 2]),
                }
                rest = &tail[2 + end + 2..];
            }
            // Unterminated `{{` — emit the rest verbatim and stop.
            None => {
                out.push_str(tail);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The title and message an issue shows in the exported report. `templates`
/// carries the UI's current locale, flattened as `<rule_id>.<field>`; absent
/// (English) means the analyzer's own prose, so the English report stays
/// byte-for-byte what it was.
fn localized_issue_cells(
    issue: &analyzer::Issue,
    templates: Option<&HashMap<String, String>>,
) -> (String, String) {
    // dcc_source's age unit is the one placeholder whose value is itself a
    // word: `args.age_unit` is the raw tag (`"d"`), not a noun. The bucket
    // choice stays in Rust's `humanize_seconds`; only the noun is looked up,
    // exactly as `localizeIssue` does in the UI — the two ends must do one
    // lookup each or the report and the panel disagree on the same issue.
    // No `duration.*` entry (a locale that skipped them) leaves the tag, which
    // is what the English prose prints anyway.
    let localized = issue
        .args
        .get("age_unit")
        .and_then(|tag| templates.and_then(|t| t.get(&format!("duration.{tag}"))))
        .map(|noun| {
            let mut args = issue.args.clone();
            args.insert("age_unit".to_string(), noun.clone());
            args
        });
    let args = localized.as_ref().unwrap_or(&issue.args);

    let pick = |field: &str, fallback: &str| match templates
        .and_then(|t| t.get(&format!("{}.{}", issue.rule_id, field)))
    {
        Some(tpl) => render_template(tpl, args),
        None => fallback.to_string(),
    };
    (pick("title", &issue.rule_name), pick("message", &issue.message))
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

    let (samples, project_meta, existing_tags) = build_learning_inputs(&project_id, depth)?;

    let request = llm::learning::LearnRequest {
        samples,
        project_meta,
        existing_tags,
        model: model.clone(),
        sampling_depth: depth,
        prompt_version: llm::learning::LEARNING_PROMPT_VERSION,
    };

    let result = prov.learn_project(&request).await.map_err(String::from)?;

    // Stage the rules in memory — NOTHING is written to disk here. The review
    // panel's Save (`save_ai_rules`) is the single commit point: it takes this
    // pending doc (true provider/model/depth metadata included) and persists
    // the user-approved rule list. Closing the panel without saving therefore
    // really discards the run, and unreviewed rules never influence
    // `suggest_tags` (which reads only the on-disk tidycraft.ai.toml).
    let doc = llm::rule_store::AiRulesDoc {
        last_learned: chrono::Utc::now().to_rfc3339(),
        prompt_version: llm::learning::LEARNING_PROMPT_VERSION,
        sampling_depth: depth,
        provider_used: provider,
        model_used: model,
        rules: result.rules.clone(),
    };
    project::with_mut(&project_id, |state| {
        state.pending_ai_rules = Some(doc);
        Ok(())
    })?;

    Ok(result)
}

/// Snapshot + sample exactly what a learning run would send: scan / tags /
/// root under the project lock, then `[project]` meta and the deterministic
/// per-project sampling outside it. Shared by the real call and its cost
/// estimator so the preview prices the ACTUAL prompt, not an approximation.
fn build_learning_inputs(
    project_id: &str,
    depth: usize,
) -> Result<
    (
        Vec<llm::learning::DirectorySample>,
        Option<llm::project_meta::ProjectMeta>,
        Vec<llm::ExistingTagContext>,
    ),
    String,
> {
    // Snapshot scan + tags + root_path inside the project lock.
    // Drop the lock before any IO (toml read) or async work
    // (provider call) — same pattern as `llm_suggest_tags`.
    const SAMPLES_PER_TAG: usize = 5;
    let snapshot = project::with_mut(project_id, |state| {
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

    Ok((samples, project_meta, existing_tags))
}

/// Cost preview for the LearnSetupModal. Pure local math — builds the SAME
/// prompt a learning run would send (same sampler, same seed, same builder)
/// and prices it, instead of shoehorning "directory count" into the per-asset
/// tagging estimator (which budgets 150 output tokens per asset and can be
/// off by orders of magnitude for a single-document learning call).
#[tauri::command]
fn estimate_learning_cost(
    project_id: String,
    provider: String,
    model: String,
    sampling_depth: usize,
) -> Result<llm::CostEstimate, String> {
    let depth = sampling_depth.clamp(3, 30);

    // Validate the provider/model pair the same way the real call would.
    let cfg = llm::ProviderConfig {
        api_key: None,
        endpoint: None,
        model: model.clone(),
    };
    let _ = llm::make_provider(&provider, cfg).map_err(String::from)?;

    let (samples, project_meta, existing_tags) = build_learning_inputs(&project_id, depth)?;
    let user_prompt =
        llm::prompts::build_learning_prompt(&samples, project_meta.as_ref(), &existing_tags);
    let prompt_chars = llm::prompts::SYSTEM_PROMPT_LEARNING.len() + user_prompt.len();
    let sample_count = samples.iter().map(|s| s.files.len()).sum();

    Ok(llm::cost::estimate_learning_cost(
        &model,
        prompt_chars,
        sample_count,
        existing_tags.len(),
    ))
}

/// Read the project's `tidycraft.ai.toml` if it exists. Frontend uses
/// this to populate the AITagPanel header status badge ("AI · 5d ago,
/// N rules"). Deliberately reads only the SAVED doc — a learning run
/// that hasn't been confirmed in the review panel (still pending in
/// `ProjectState.pending_ai_rules`) is not active and doesn't show here.
#[tauri::command]
fn read_ai_rules(project_id: String) -> Result<Option<llm::rule_store::AiRulesDoc>, String> {
    project::with_ref(&project_id, |state| {
        llm::rule_store::AiRulesDoc::load(Path::new(&state.root_path))
    })
}

/// The review panel's Save: the single point where learned rules reach disk.
/// Takes the pending doc staged by `learn_project_conventions` (carrying that
/// run's true metadata) and writes it with the user-edited rule list; with no
/// pending run (re-saving edits later), preserves the on-disk doc's metadata.
/// See `AiRulesDoc::for_save` for the exact precedence.
#[tauri::command]
fn save_ai_rules(
    project_id: String,
    rules: Vec<llm::learning::LearnedRule>,
) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        let root = Path::new(&state.root_path);
        let pending = state.pending_ai_rules.take();
        let on_disk = if pending.is_none() {
            llm::rule_store::AiRulesDoc::load(root)?
        } else {
            None
        };
        llm::rule_store::AiRulesDoc::for_save(pending, on_disk, rules).save(root)
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
    package_index: &unity::PackageGuidIndex,
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
    // Existence comes from the UNFILTERED scan: `[ignore]` limits what we
    // report on, not what the project is understood to contain. (The other
    // three cross-asset rules deliberately keep the filtered view — their
    // documented suppression path is "drop the file"; see docs/analyzer-rules.md.)
    let missing = analyzer.find_missing_references(scan_to_analyze, scan_result, package_index);
    result.merge(missing);
    let pbr = analyzer.find_pbr_set_issues(scan_to_analyze, &config.pbr_set);
    result.merge(pbr);
    let dcc = analyzer.find_dcc_source_issues(scan_to_analyze, &config.dcc_source);
    result.merge(dcc);
    result
}

// `(async)` runs this on Tauri's thread pool instead of the main thread.
// duplicate-hashing + full Unity re-parse under the project lock is heavy;
// on the main thread it froze the whole UI (window drag/resize) for the
// duration. The frontend contract is unchanged — `invoke` already awaits.
#[tauri::command(async)]
fn analyze_assets(project_id: String, config_toml: Option<String>) -> Result<AnalysisResult, String> {
    let config = if let Some(toml_str) = config_toml {
        RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
    } else {
        RuleConfig::default()
    };

    // Build the ignore matcher up-front so a malformed pattern surfaces as
    // an error before we touch the per-project lock.
    let ignore_set = build_ignore_set(&config)?;
    // Fetched before the lock below — see package_index_for.
    let package_index = package_index_for(&project_id);

    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;
        Ok(run_full_analysis(
            scan_result,
            &state.root_path,
            &config,
            ignore_set.as_ref(),
            &package_index,
        ))
    })
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
fn suggest_tags(project_id: String) -> Result<analyzer::rule_suggest::TagSuggestions, String> {
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
        //   - the file is corrupt (load error) — we fall back rather than
        //     failing the whole call so AITagPanel still shows *something*,
        //     and report it in `warnings` so the fallback isn't mistaken
        //     for a working rule set.
        let mut suggestions = analyzer::rule_suggest::load_or_fallback(scan, root);

        suggestions.groups.retain(|g| {
            !already_suggested.contains(&format!("{} (suggested)", g.name))
        });
        Ok(suggestions)
    })
}

// ============ Git Commands ============

// `(async)`: libgit2 opens the repo + runs a full-tree status (twice per
// refresh, with get_git_statuses) — off the main thread so large repos don't
// freeze the UI.
#[tauri::command(async)]
fn get_git_info(project_id: String, path: String) -> GitInfo {
    let mut manager = GitManager::open(Path::new(&path));
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

// `(async)`: full-repo libgit2 status under the project lock — off the main
// thread so a large working tree doesn't stall the event loop.
#[tauri::command(async)]
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

#[derive(Serialize)]
pub struct DependencyGraph {
    pub nodes: Vec<DependencyNode>,
    pub edges: Vec<DependencyEdge>,
}

/// One node in a project's dependency graph. `id` is the engine-neutral graph
/// identifier edges reference — a Unity GUID or a Godot `res://` path — while
/// `path` is the absolute filesystem path the frontend uses to locate the asset.
/// How firmly a graph node's identity resolves. From a disk scan this is a
/// spectrum, not a boolean — the scan set undercounts what a project can
/// legitimately reference (engine built-ins, package caches, gitignored
/// files), so each variant asserts only what the evidence supports. Same
/// doctrine as `missing_reference.rs`: "a miss is strong signal, not proof".
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DependencyNodeKind {
    /// A scanned project asset — has a real `path`, clickable in the UI.
    Asset,
    /// Unity: a referenced GUID resolved through the `Library/PackageCache`
    /// index — a package asset installed by the package manager. Known to
    /// exist; simply not part of the project's own assets.
    Package,
    /// Unity: a referenced GUID with no scanned asset behind it and no
    /// package-index hit. Ambiguous by construction — a package asset (when
    /// no local `Library/` cache exists to resolve it), an ignore-excluded
    /// file, and a genuinely broken reference are indistinguishable from a
    /// disk scan. Rendered as a warning, never asserted broken.
    Unresolved,
    /// Godot: a `res://` target that exists on disk but sits outside the scan
    /// set (gitignored / hidden directory). Not breakage.
    Unscanned,
    /// Godot: a `res://` target that does not exist on disk — confirmed broken.
    Missing,
}

#[derive(Serialize)]
pub struct DependencyNode {
    pub id: String,
    pub path: String,
    pub name: String,
    pub file_type: String,
    /// See `DependencyNodeKind`. Non-`asset` nodes carry an empty `path`
    /// (nothing to locate) and are treated as BFS terminals by the frontend,
    /// so a widely-shared unresolved GUID can't hub-connect its unrelated
    /// referrers in the 2-hop view.
    pub kind: DependencyNodeKind,
    /// Secondary identity line for the tooltip — the package id for
    /// `package` nodes ("com.unity.render-pipelines.universal"). Absent
    /// elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Cached GUID→package index for this project, built lazily and rebuilt only
/// when `Library/PackageCache`'s directory listing changes. Takes the project
/// lock briefly — callers grab the Arc BEFORE their own `with_ref` block
/// (`with_mut` inside `with_ref` would self-deadlock on the project mutex).
/// Unknown project / no cache dir both yield an empty index, which every
/// consumer treats as "resolve nothing".
fn package_index_for(project_id: &str) -> std::sync::Arc<unity::PackageGuidIndex> {
    project::with_mut(project_id, |state| {
        let root = Path::new(&state.root_path);
        let key = unity::package_cache_key(root);
        if let Some((cached_key, index)) = &state.package_index {
            if *cached_key == key {
                return Ok(index.clone());
            }
        }
        let index = std::sync::Arc::new(unity::build_package_guid_index(root));
        state.package_index = Some((key, index.clone()));
        Ok(index)
    })
    .unwrap_or_default()
}

/// The slice of project state the engine-walk commands need, cloned under a
/// brief lock so the walk itself runs with the lock released.
///
/// `get_unity_dependencies`, `find_unused_assets`, `get_godot_dependencies` and
/// `godot_asset_references` all re-open and re-parse every scene / prefab /
/// material / script in the project — seconds on a large one. Doing that inside
/// `with_ref` held the per-project mutex for the entire walk, which stalls the
/// watcher's 500ms batches, every other command for that project, and the
/// cancel path. Nothing in the walk reads state beyond these three values, so a
/// snapshot is sufficient — the same "snapshot inside the lock, IO outside"
/// discipline `llm_suggest_tags` already follows.
///
/// Worst case the snapshot is one scan stale (a watcher batch lands mid-walk).
/// That was already true of the returned graph the moment it crossed the IPC
/// boundary, so it costs no accuracy that existed before.
struct EngineScanSnapshot {
    root_path: String,
    assets: Vec<scanner::AssetInfo>,
    project_type: Option<scanner::ProjectType>,
}

fn engine_scan_snapshot(project_id: &str) -> Result<EngineScanSnapshot, String> {
    project::with_ref(project_id, |state| {
        let scan = state.require_scan()?;
        Ok(EngineScanSnapshot {
            root_path: state.root_path.clone(),
            assets: scan.assets.clone(),
            project_type: scan.project_type.clone(),
        })
    })
}

#[derive(Serialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
}

// `(async)`: re-reads + parses every prefab/scene/mat — off the main thread so
// a 10k-asset project doesn't freeze the window, and off the project lock (see
// EngineScanSnapshot) so it doesn't freeze the project's other commands either.
#[tauri::command(async)]
fn get_unity_dependencies(project_id: String) -> Result<DependencyGraph, String> {
    // Fetched before the snapshot below — see package_index_for.
    let package_index = package_index_for(&project_id);
    let scan_result = engine_scan_snapshot(&project_id)?;
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
                kind: DependencyNodeKind::Asset,
                detail: None,
            });
        }
    }

    // References the scan can't resolve. Two classes never enter the
    // graph at all — the all-zero "no reference" sentinel and the
    // editor-shipped built-in bundles (`unity default resources` /
    // `unity_builtin_extra`), the same exemptions the missing_reference
    // rule applies: they aren't project assets, and the built-ins are
    // exactly the GUIDs every material / UI element shares, so one node
    // for them would hub-connect the whole project in the 2-hop view.
    // The rest resolves through the PackageCache index when a local
    // Library/ exists — a `package` node with its file and package name
    // — and only what's left is genuinely ambiguous (no cache to check,
    // ignore-excluded, or truly deleted): one deduped `unresolved` node,
    // a warning with its edge intact, not an asserted breakage.
    let mut unresolved_guids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for asset in &scan_result.assets {
        if unity::is_reference_source(&asset.extension) {
            if let Some(unity_info) = unity::parse_unity_file(Path::new(&asset.path)) {
                if let Some(ref from_guid) = asset.unity_guid {
                    for reference in &unity_info.references {
                        if unity::is_null_guid(&reference.guid)
                            || unity::is_builtin_guid(&reference.guid)
                        {
                            continue;
                        }
                        if !guid_to_path.contains_key(&reference.guid)
                            && unresolved_guids.insert(reference.guid.clone())
                        {
                            nodes.push(match package_index.get(&reference.guid) {
                                Some(pkg) => DependencyNode {
                                    id: reference.guid.clone(),
                                    path: String::new(),
                                    name: pkg.file_name.clone(),
                                    file_type: "package".to_string(),
                                    kind: DependencyNodeKind::Package,
                                    detail: Some(pkg.package.clone()),
                                },
                                None => DependencyNode {
                                    id: reference.guid.clone(),
                                    path: String::new(),
                                    name: reference.guid.clone(),
                                    file_type: "unresolved".to_string(),
                                    kind: DependencyNodeKind::Unresolved,
                                    detail: None,
                                },
                            });
                        }
                        edges.push(DependencyEdge {
                            from: from_guid.clone(),
                            to: reference.guid.clone(),
                        });
                    }
                }
            }
        }
    }

    Ok(DependencyGraph { nodes, edges })
}

/// Result of an unused-asset scan.
///
/// `unreadable_sources` counts referenceable files whose text could not be
/// read. That is almost always a project set to Force Binary (or Mixed) asset
/// serialization: `unity::parse_unity_file` reads YAML, so a binary `.prefab`
/// or `.unity` yields NO outgoing references, and every asset only those files
/// referenced then looks unused. Silently returning that list invites the user
/// to delete assets that are very much in use — the one genuinely destructive
/// failure mode in the app — so the count travels with the result and the UI
/// says the answer can't be trusted.
#[derive(Debug, Serialize)]
struct UnusedAssetsResult {
    unused: Vec<String>,
    unreadable_sources: usize,
}

// `(async)`: same heavy Unity/Godot re-parse under the lock as the dependency
// graph — kept off the main thread.
#[tauri::command(async)]
fn find_unused_assets(project_id: String) -> Result<UnusedAssetsResult, String> {
    let scan_result = engine_scan_snapshot(&project_id)?;
    match scan_result.project_type {
        // Godot uses res:// path refs, not GUIDs — dispatch to its own
        // parser and return early.
        Some(scanner::ProjectType::Godot) => {
            return Ok(UnusedAssetsResult {
                unused: godot::find_unused_godot_assets(
                    &scan_result.root_path,
                    &scan_result.assets,
                ),
                // Godot's parser reads text too, but its sources are scenes
                // and scripts that are text by format — no binary mode to
                // silently swallow, so nothing to warn about.
                unreadable_sources: 0,
            });
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
        // Scenes are graph roots (loaded via build settings / the editor /
        // SceneManager.LoadScene by name), so having no incoming GUID
        // reference doesn't make a scene unused — drop them as candidates.
        // They're still parsed as reference *sources* below, so assets a
        // scene references aren't falsely flagged.
        if matches!(asset.asset_type, scanner::AssetType::Scene) {
            continue;
        }
        if let Some(ref guid) = asset.unity_guid {
            all_guids.insert(guid.clone(), asset.path.clone());
        }
    }

    let mut unreadable_sources = 0usize;
    for asset in &scan_result.assets {
        if unity::is_reference_source(&asset.extension) {
            match unity::parse_unity_file(Path::new(&asset.path)) {
                Some(unity_info) => {
                    for reference in &unity_info.references {
                        referenced_guids.insert(reference.guid.clone());
                    }
                }
                // Binary-serialized (or otherwise unreadable): its outgoing
                // references are invisible to us, so anything only it points
                // at is about to be reported unused. Count, don't guess.
                None => unreadable_sources += 1,
            }
        }
    }

    let unused: Vec<String> = all_guids
        .iter()
        .filter(|(guid, _path)| !referenced_guids.contains(*guid))
        .map(|(_guid, path)| path.clone())
        .collect();

    Ok(UnusedAssetsResult {
        unused,
        unreadable_sources,
    })
}

/// Godot counterpart to `get_unity_dependencies`. Nodes are every non-metadata
/// asset keyed by its `res://` id; edges come from the `res://` references in
/// scenes / resources / scripts (target filtered to known nodes). Same parser
/// and known gaps as the unused-asset check (uid-only / dynamic `load()` missed).
// `(async)`: parses every scene/resource/script under the lock — off the
// main thread (mirrors get_unity_dependencies).
#[tauri::command(async)]
fn get_godot_dependencies(project_id: String) -> Result<DependencyGraph, String> {
    let scan_result = engine_scan_snapshot(&project_id)?;
    if !matches!(scan_result.project_type, Some(scanner::ProjectType::Godot)) {
        return Err("Not a Godot project".to_string());
    }

    let root = Path::new(&scan_result.root_path);
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
                kind: DependencyNodeKind::Asset,
                detail: None,
            });
        }
    }

    // Keep every edge, but classify unknown `res://` targets honestly:
    // unlike Unity GUIDs, a res path can be checked against the disk, so
    // "outside the scan but present" (gitignored addons/, hidden dirs —
    // not breakage) and "genuinely gone" (a broken reference) get
    // different nodes instead of one scary bucket. One deduped node per
    // distinct target either way.
    let mut edges: Vec<DependencyEdge> = Vec::new();
    let mut unknown: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (from, to) in godot::godot_dependency_edges(root, &scan_result.assets) {
        if !known.contains(&to) && unknown.insert(to.clone()) {
            let on_disk = godot::res_path_to_abs(&to, root)
                .map(|p| p.exists())
                .unwrap_or(false);
            nodes.push(DependencyNode {
                id: to.clone(),
                path: String::new(),
                name: to.clone(),
                file_type: if on_disk { "unscanned" } else { "missing" }.to_string(),
                kind: if on_disk {
                    DependencyNodeKind::Unscanned
                } else {
                    DependencyNodeKind::Missing
                },
                detail: None,
            });
        }
        edges.push(DependencyEdge { from, to });
    }

    Ok(DependencyGraph { nodes, edges })
}

/// Rename guardrail: for each of `paths` (absolute), the project files that
/// reference it by `res://` path — root-relative names, `project.godot`
/// included. Godot-only: Unity references are GUID-based and survive renames
/// (the `.meta` sidecar moves with the file), so the frontend never calls
/// this for other project types.
// `(async)`: re-reads every scene/resource/script under the lock — off the
// main thread (same shape as get_godot_dependencies).
#[tauri::command(async)]
fn godot_asset_references(
    project_id: String,
    paths: Vec<String>,
) -> Result<HashMap<String, Vec<String>>, String> {
    let scan_result = engine_scan_snapshot(&project_id)?;
    if !matches!(scan_result.project_type, Some(scanner::ProjectType::Godot)) {
        return Err("Not a Godot project".to_string());
    }
    Ok(godot::referencing_files(
        Path::new(&scan_result.root_path),
        &scan_result.assets,
        &paths,
    ))
}

// ============ Engine Info Commands ============
//
// Path-only commands (no project_id): they re-read small marker/config files
// fresh on every call, so there's no per-project state to consult. Each
// returns `None` instead of an error when the info isn't there — an absent
// card is the correct UI for a project without the marker file.

/// On-demand parse of a single Unity YAML asset for the preview panel:
/// component list (prefab/scene only, sorted) + GUID references.
// `(async)`: reads + line-scans a potentially multi-MB scene file — off the
// main thread.
#[tauri::command(async)]
fn get_unity_file_info(path: String) -> Option<unity::UnityFileInfo> {
    unity::parse_unity_file(Path::new(&path))
}

/// Unity engine card: editor version from `ProjectSettings/ProjectVersion.txt`.
#[tauri::command(async)]
fn get_unity_project_info(root_path: String) -> Option<unity::UnityProjectInfo> {
    unity::parse_project_version(Path::new(&root_path))
}

/// Godot engine card: name / version / main scene / renderer / autoloads
/// parsed from `<root>/project.godot`.
#[tauri::command(async)]
fn get_godot_project_info(root_path: String) -> Option<godot::GodotProjectInfo> {
    godot::parse_project_godot(&Path::new(&root_path).join("project.godot"))
}

/// Unreal engine card: engine association / modules / plugins / target
/// platforms parsed from the root `.uproject` (JSON).
#[tauri::command(async)]
fn get_unreal_project_info(root_path: String) -> Option<unreal::UnrealProjectInfo> {
    let uproject = unreal::find_uproject_file(Path::new(&root_path))?;
    unreal::parse_uproject(&uproject)
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
                "{},{},{:?},{},{},{},{}\n",
                csv_cell(&asset.name),
                csv_cell(&asset.path),
                asset.asset_type,
                csv_cell(&asset.extension),
                asset.size,
                width,
                height
            ));
        }

        Ok(csv)
    })
}

// `(async)`: runs a full analysis (incl. duplicate re-hashing) under the lock.
#[tauri::command(async)]
fn export_issues_to_json(project_id: String) -> Result<String, String> {
    // Fetched before the lock below — see package_index_for.
    let package_index = package_index_for(&project_id);
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        // Mirror the UI's Run Analysis: honor the project's tidycraft.toml
        // (rule thresholds + [ignore].patterns) and run every phase,
        // including the PBR-set and DCC-source cross-asset checks. Without
        // this the exported report would silently diverge from the Issues
        // view under any custom config.
        let config = load_rule_config(&state.root_path)?;
        let ignore_set = build_ignore_set(&config)?;
        let result = run_full_analysis(
            scan_result,
            &state.root_path,
            &config,
            ignore_set.as_ref(),
            &package_index,
        );

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    })
}

/// `issue_limit` / `asset_limit` cap the report's table rows (Settings →
/// Export). `None` keeps the historical defaults (100 / 500); `Some(0)`
/// means unlimited — a 100k-file project then produces a very large file,
/// which is the user's explicit choice.
// `(async)`: runs a full analysis (incl. duplicate re-hashing) under the lock.
#[tauri::command(async)]
fn export_to_html(
    project_id: String,
    issue_limit: Option<usize>,
    asset_limit: Option<usize>,
    // Flattened `issues.rules.*` / `issues.duration.*` for the UI's current
    // locale, keyed `"<rule_id>.message"` / `"duration.d"`. Absent for
    // English — the report then uses the analyzer's own prose.
    templates: Option<HashMap<String, String>>,
) -> Result<String, String> {
    let cap = |limit: Option<usize>, default: usize| match limit {
        Some(0) => usize::MAX,
        Some(n) => n,
        None => default,
    };
    let issue_cap = cap(issue_limit, 100);
    let asset_cap = cap(asset_limit, 500);

    // Fetched before the lock below — see package_index_for.
    let package_index = package_index_for(&project_id);
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        // Same analysis pipeline as Run Analysis / the JSON export, so the
        // HTML report's issue list matches the Issues view (custom config,
        // [ignore].patterns, PBR/DCC phases all applied). The asset
        // inventory cards below intentionally stay on the full scan —
        // [ignore].patterns scope analysis, not the project's file census.
        let config = load_rule_config(&state.root_path)?;
        let ignore_set = build_ignore_set(&config)?;
        let analysis_result = run_full_analysis(
            scan_result,
            &state.root_path,
            &config,
            ignore_set.as_ref(),
            &package_index,
        );

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
                    .take(issue_cap)
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
                        let (title, message) =
                            localized_issue_cells(issue, templates.as_ref());
                        format!(
                            r#"<tr><td class="{}">{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                            severity_class,
                            issue.severity,
                            html_escape(&title),
                            html_escape(file_name),
                            html_escape(&message)
                        )
                    })
                    .collect();
                if total > issue_cap {
                    rows.push(format!(
                        r#"<tr><td colspan="4" style="text-align:center;color:#9ca3af;font-style:italic;">Showing first {} of {} issues — export to JSON for the complete list, or raise the limit in Settings → Export.</td></tr>"#,
                        issue_cap, total
                    ));
                }
                rows.join("\n")
            },
            asset_rows = {
                let total = scan_result.assets.len();
                let mut rows: Vec<String> = scan_result
                    .assets
                    .iter()
                    .take(asset_cap)
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
                if total > asset_cap {
                    rows.push(format!(
                        r#"<tr><td colspan="4" style="text-align:center;color:#9ca3af;font-style:italic;">Showing first {} of {} assets — export to CSV or JSON for the complete list, or raise the limit in Settings → Export.</td></tr>"#,
                        asset_cap, total
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
        // An empty `find` is a no-op, NOT `str::replace("")` — that inserts
        // the replacement between every character ("abc" → "XaXbXcX"). The
        // preview shares this function, so the no-op also zeroes the
        // dialog's changed-count and disables Apply.
        RenameOperation::FindReplace { find, replace } => {
            if find.is_empty() {
                name.to_string()
            } else {
                name.replace(find, replace)
            }
        }
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

/// Reject rename targets that would escape the file's own directory. The
/// dialogs validate too, but the IPC boundary must not rely on frontend
/// checks — a separator in `new_name` turns `parent.join(new_name)` into a
/// directory traversal, and a find→replace text can inject one just as
/// easily as a direct call.
/// MS-DOS device names. Win32 still resolves them as devices whatever the
/// extension carried — `CON.png` is the console, not a file — so such a file
/// cannot be created on Windows at all.
const WINDOWS_RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The last gate before a rename touches the disk. The two Windows rules below
/// are enforced on every platform on purpose: a name minted on macOS lands in
/// the repository a Windows teammate checks out, and a guard behind
/// `cfg(windows)` is a guard that never runs where the work is being done —
/// the same trap as a policy that only applies in production.
fn validate_new_name(new_name: &str) -> Result<(), String> {
    if new_name.is_empty() || new_name == "." || new_name == ".." {
        return Err("Invalid file name".to_string());
    }
    if new_name.contains('/') || new_name.contains('\\') {
        return Err("File name cannot contain path separators".to_string());
    }
    // Win32 strips these on create, so the file would land under a name that
    // is not the one we record in the undo stack and the tag bindings — three
    // views of one file that stop agreeing, silently.
    if new_name.ends_with(' ') || new_name.ends_with('.') {
        return Err("File name cannot end with a space or a period".to_string());
    }
    let stem = new_name.split('.').next().unwrap_or(new_name);
    if WINDOWS_RESERVED_STEMS
        .iter()
        .any(|r| stem.eq_ignore_ascii_case(r))
    {
        return Err(format!(
            "\"{}\" is a reserved device name on Windows and cannot be used as a file name",
            stem
        ));
    }
    Ok(())
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
    // Every path gets the SAME operation applied to derive its new file name;
    // the shared heterogeneous engine below does validation, the rename, .meta
    // carry, undo, and tag migration.
    let planned: Vec<(String, String)> = paths
        .into_iter()
        .map(|path| {
            let name = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let new_name = apply_rename_operation(&name, &operation);
            (path, new_name)
        })
        .collect();

    commit_renames(&project_id, planned, "Batch rename")
}

/// Rename a heterogeneous batch — each file to its own new *file name* within
/// its current directory: validate → same-file guard → fs::rename → carry the
/// Unity .meta sidecar. Returns the successes as `(old_path, normalized new
/// path)` alongside the tallied result. Deliberately free of project-state
/// side effects (no undo, no tags) so it's unit-testable with a tempdir and
/// shared by both batch-rename entry points; `commit_renames` layers undo +
/// tag migration on top.
fn rename_batch_on_disk(
    planned: Vec<(String, String)>,
) -> (Vec<(String, String)>, BatchRenameResult) {
    let mut success_count = 0;
    let mut error_count = 0;
    let mut errors = Vec::new();
    let mut done: Vec<(String, String)> = Vec::new();

    for (path, new_name) in planned {
        let path_obj = Path::new(&path);
        let name = match path_obj.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                errors.push(format!("Invalid path: {}", path));
                error_count += 1;
                continue;
            }
        };

        if name == new_name {
            continue; // no-op — nothing to rename
        }

        if let Err(e) = validate_new_name(&new_name) {
            errors.push(format!("{}: {}", name, e));
            error_count += 1;
            continue;
        }

        let new_path = path_obj.with_file_name(&new_name);

        // The target may `exists()`-resolve to the source file itself — a pure
        // case change (foo.PNG → foo.png) on case-insensitive filesystems
        // (NTFS/APFS), or an NFC/NFD Unicode variant on macOS. `fs::rename`
        // handles those fine, so only reject when the occupant is genuinely a
        // *different* file. Identity is checked by dev+inode (undo.rs), not by
        // name: on case-sensitive filesystems `foo.png` and `FOO.PNG` can
        // coexist, and a name-based "case-only ⇒ allow" guess would let the
        // rename silently clobber the other file.
        if new_path.exists() && !undo::paths_are_same_file(path_obj, &new_path) {
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
                // so the undo record and the tag binding key off the same string
                // the next scan will produce — a raw to_string_lossy() keeps
                // Windows backslashes and the tag key would never match.
                done.push((path.clone(), scanner::path_to_string(&new_path)));
            }
            Err(e) => {
                errors.push(format!("Failed to rename {}: {}", name, e));
                error_count += 1;
            }
        }
    }

    (
        done,
        BatchRenameResult {
            success_count,
            error_count,
            errors,
        },
    )
}

/// Files renamed per lock window.
///
/// A file's disk rename and its tag migration must not be separated by a lock
/// release: the watcher reaps tag bindings for paths that have vanished, so a
/// gap there loses the tags of every file renamed before the watcher fired.
/// Holding the project lock across a five-thousand-file batch would instead
/// freeze every other command for that project, so the batch is chunked —
/// bounded hold, and between chunks the watcher can only see files whose tags
/// have already moved or files not yet touched.
const RENAME_LOCK_CHUNK: usize = 100;

/// Rename a heterogeneous batch on disk, migrating tag bindings as it goes, and
/// — if anything moved — record ONE undo batch (so the whole set reverts with a
/// single Ctrl+Z). `label` names the undo entry ("Batch rename" / "Fix
/// naming"); the recorded description is `"{label}: {N} files"` with N = the
/// number of files actually renamed. Shared by execute_batch_rename and
/// apply_naming_fixes.
///
/// The rename itself runs *inside* the project lock, in `RENAME_LOCK_CHUNK`
/// slices — see that constant for why, and
/// `commit_renames_does_not_expose_renamed_files_before_tags_follow` for the
/// regression this shape exists to prevent.
fn commit_renames(project_id: &str, planned: Vec<(String, String)>, label: &str) -> BatchRenameResult {
    let total = planned.len();
    let mut all_done: Vec<(String, String)> = Vec::new();
    let mut result = BatchRenameResult {
        success_count: 0,
        error_count: 0,
        errors: Vec::new(),
    };

    for chunk in planned.chunks(RENAME_LOCK_CHUNK) {
        let outcome = project::with_mut(project_id, |state| {
            let (done, part) = rename_batch_on_disk(chunk.to_vec());

            // Tags follow the file across renames — same as move_assets /
            // rename_file. Paths are already normalized (scanner::path_to_string)
            // so the new key matches what the next scan produces.
            if state.tags_data.is_some() && !done.is_empty() {
                let tags = state.ensure_tags();
                for (original, new_path) in &done {
                    tags.rename_path(original, new_path);
                }
                // Logged, not swallowed: the files are already renamed, so this
                // must not fail the command, but a silent failure here means the
                // bindings only live in memory (watcher.rs logs the same way).
                if let Err(e) = state.save_tags() {
                    eprintln!("[batch_rename] failed to save tags after rename: {}", e);
                }
            }
            Ok((done, part))
        });

        match outcome {
            Ok((done, part)) => {
                result.success_count += part.success_count;
                result.error_count += part.error_count;
                result.errors.extend(part.errors);
                all_done.extend(done);
            }
            Err(e) => {
                // Project not registered. We could still rename on disk, but
                // with no undo record and no tag migration — renaming files
                // with no way back is worse than refusing, so report it. (The
                // previous shape swallowed this with `let _ =` and renamed
                // anyway.)
                let untouched = total - result.success_count - result.error_count;
                result.error_count += untouched;
                result.errors.push(format!("Renames aborted: {}", e));
                return result;
            }
        }
    }

    if !all_done.is_empty() {
        let ts = unix_timestamp();
        let file_ops: Vec<undo::FileOperation> = all_done
            .iter()
            .map(|(original, new_path)| undo::FileOperation {
                operation_type: undo::OperationType::Rename,
                original_path: original.clone(),
                new_path: Some(new_path.clone()),
                timestamp: ts,
            })
            .collect();

        let _ = project::with_mut(project_id, |state| {
            state
                .undo_manager
                .record_batch(format!("{}: {} files", label, file_ops.len()), file_ops);
            Ok(())
        });
    }

    result
}

// ============ Fix-it (auto-fixable naming) Commands ============

/// One proposed naming fix surfaced to the Fix-it review dialog. Only assets
/// that actually carry an auto-fixable naming violation are emitted, so
/// `suggested_name` always differs from `original_name`.
#[derive(Serialize)]
pub struct NamingFixPreview {
    /// Absolute, forward-slash-normalized path of the asset to rename.
    pub path: String,
    pub original_name: String,
    pub suggested_name: String,
    /// True when another proposed fix in the same directory targets the same
    /// name — applying both would collide. Advisory for the UI; the fs guard in
    /// `rename_batch_on_disk` is the real backstop.
    pub collides: bool,
}

/// A single rename the user accepted from the Fix-it dialog. `new_name` may have
/// been hand-edited, so it runs through the same validation + same-file guards
/// as every other rename entry point (see `rename_file`).
#[derive(serde::Deserialize)]
pub struct NamingFix {
    pub path: String,
    pub new_name: String,
}

/// Compute compliant-name suggestions for every asset with an auto-fixable
/// naming violation, using the same `tidycraft.toml` the analysis ran with.
/// Read-only — nothing is renamed until `apply_naming_fixes`.
// `(async)`: iterates the whole scan under the project lock — and that lock
// may be held by an in-flight analysis for seconds, which a main-thread
// command would turn into a whole-window freeze.
#[tauri::command(async)]
fn preview_naming_fixes(
    project_id: String,
    config_toml: Option<String>,
) -> Result<Vec<NamingFixPreview>, String> {
    let config = match config_toml {
        Some(toml_str) => {
            RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
        }
        None => RuleConfig::default(),
    };
    let rule = analyzer::rules::naming::NamingRule::new(config.naming);

    project::with_ref(&project_id, |state| {
        let scan = state.require_scan()?;
        let mut previews: Vec<NamingFixPreview> = scan
            .assets
            .iter()
            .filter_map(|asset| {
                rule.suggest_compliant_name(asset)
                    .map(|suggested| NamingFixPreview {
                        path: asset.path.clone(),
                        original_name: asset.name.clone(),
                        suggested_name: suggested,
                        collides: false,
                    })
            })
            .collect();
        mark_naming_fix_collisions(&mut previews);
        Ok(previews)
    })
}

/// Flag proposals whose target (parent directory + suggested name) is shared by
/// more than one file in the batch — only the first would land, the rest would
/// hit "target already exists". Keyed case-insensitively so it also catches
/// collisions that only surface on case-insensitive filesystems.
fn mark_naming_fix_collisions(previews: &mut [NamingFixPreview]) {
    use std::collections::HashMap;
    let key = |p: &NamingFixPreview| -> String {
        let parent = Path::new(&p.path)
            .parent()
            .map(|d| d.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        format!("{}\u{0}{}", parent, p.suggested_name.to_lowercase())
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for p in previews.iter() {
        *counts.entry(key(p)).or_insert(0) += 1;
    }
    for p in previews.iter_mut() {
        if counts.get(&key(p)).copied().unwrap_or(0) > 1 {
            p.collides = true;
        }
    }
}

/// Apply the renames the user accepted from the Fix-it dialog. Routes through
/// the shared batch engine, so it validates each target, guards against
/// clobbering a different file, carries Unity .meta sidecars, records ONE undo
/// batch, and migrates tags — identical guarantees to Batch Rename.
// `(async)`: "Fix all naming" can submit thousands of renames (plus .meta
// probes and the undo/tags write-back) in one batch — off the main thread,
// same rationale as delete_assets.
#[tauri::command(async)]
fn apply_naming_fixes(project_id: String, fixes: Vec<NamingFix>) -> BatchRenameResult {
    let planned: Vec<(String, String)> = fixes.into_iter().map(|f| (f.path, f.new_name)).collect();
    commit_renames(&project_id, planned, "Fix naming")
}

// ============ Unreal Engine Commands ============

// ============ Godot Commands ============

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

/// Write an export payload to a user-chosen destination. The frontend gets
/// `path` from the native save dialog (plugin-dialog), so the user has
/// already pointed at this exact location — the command only performs the
/// write the webview itself cannot. Replaces the old blob-`<a download>`
/// trick, which saved silently to Downloads on Windows and is unreliable
/// in WKWebView.
#[tauri::command]
fn save_text_file(path: String, contents: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Empty destination path".to_string());
    }
    std::fs::write(&path, contents).map_err(|e| e.to_string())
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

#[derive(Serialize, Debug)]
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

    // Chunked, and the moves happen INSIDE the project lock: a file that has
    // left its old path but whose tag binding hasn't moved yet is exactly what
    // the watcher's orphan cleanup reaps. Same shape and same reasoning as
    // `commit_renames` — see `RENAME_LOCK_CHUNK`.
    for chunk in paths.chunks(RENAME_LOCK_CHUNK) {
        let outcome = project::with_mut(&project_id, |state| {
            let mut moved: Vec<FileOpSuccess> = Vec::new();
            let mut failed: Vec<FileOpError> = Vec::new();

            for path in chunk {
                let src = Path::new(path);
                let name = match src.file_name() {
                    Some(n) => n.to_os_string(),
                    None => {
                        failed.push(FileOpError {
                            path: path.clone(),
                            message: "Invalid source path".to_string(),
                        });
                        continue;
                    }
                };
                let dst = target.join(&name);

                if src == dst || undo::paths_are_same_file(src, &dst) {
                    // No-op: source already in target directory. Skip silently.
                    // The literal comparison alone only catches the spelling
                    // the caller happened to send; a case variant on a
                    // case-insensitive filesystem, a symlinked folder, or a
                    // `..` names the same directory and used to fall through
                    // to the guard below, which reported "Target already
                    // exists" about the very file being moved. Same identity
                    // check the rename guard uses, same reason.
                    continue;
                }
                if dst.exists() {
                    failed.push(FileOpError {
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
                        moved.push(FileOpSuccess {
                            original_path: path.clone(),
                            new_path: scanner::path_to_string(&dst),
                        })
                    }
                    Err(e) => failed.push(FileOpError {
                        path: path.clone(),
                        message: e.to_string(),
                    }),
                }
            }

            // Tags follow the file across moves, before the lock is released.
            // Skip if tags haven't been touched in this session (lazy load).
            if state.tags_data.is_some() && !moved.is_empty() {
                let tags = state.ensure_tags();
                for s in &moved {
                    tags.rename_path(&s.original_path, &s.new_path);
                }
                // Logged, not swallowed: the move already succeeded so this
                // can't fail the command, but a silent failure leaves the
                // bindings in memory only (watcher.rs logs the same way).
                if let Err(e) = state.save_tags() {
                    eprintln!("[move_assets] failed to save tags after move: {}", e);
                }
            }
            Ok((moved, failed))
        });

        match outcome {
            Ok((moved, failed)) => {
                successes.extend(moved);
                errors.extend(failed);
            }
            Err(e) => {
                // Project not registered: moving with no undo record and no tag
                // migration is worse than refusing. Report the untouched files.
                for path in chunk {
                    errors.push(FileOpError {
                        path: path.clone(),
                        message: format!("Move aborted: {}", e),
                    });
                }
                return FileOpResult { successes, errors };
            }
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
// `(async)`: each trash operation is an OS call; the duplicate-group cleanup
// can submit thousands of paths at once (Kenney-scale groups), which would
// freeze the window if run on the main thread.
#[tauri::command(async)]
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

    validate_new_name(&new_name)?;

    let old_path_ref = Path::new(&old_path);
    if !old_path_ref.exists() {
        return Err("File does not exist".to_string());
    }

    let parent = old_path_ref.parent().ok_or("Cannot get parent directory")?;
    let new_path = parent.join(&new_name);

    let old_name = old_path_ref
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    // The target may `exists()`-resolve to the source itself (case-only rename
    // on a case-insensitive filesystem, NFC/NFD variant on macOS) — allowed,
    // `fs::rename` handles it. Only a genuinely different occupant is a
    // conflict; identity is by dev+inode, not name (see execute_batch_rename).
    if new_path.exists() && !undo::paths_are_same_file(old_path_ref, &new_path) {
        return Err("A file with this name already exists".to_string());
    }

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

        // Carry tags from the old path to the new one. Best-effort — tag
        // bookkeeping must never fail a rename that already landed on disk —
        // but logged, not silent, so a persistently unwritable tags file is
        // diagnosable (same treatment as watcher.rs / the batch paths).
        if state.tags_data.is_some() {
            // new_path_str is already normalized (scanner::path_to_string above).
            state.ensure_tags().rename_path(&old_path, &new_path_str);
            if let Err(e) = state.save_tags() {
                eprintln!("[rename_file] failed to save tags after rename: {}", e);
            }
        }
        Ok(())
    });

    Ok(new_path_str)
}

// ============ Undo Commands ============

/// After an undo reverts renames/moves, carry each reverted file's tag binding
/// back the same direction (new_path → original_path), mirroring the forward
/// carry in `move_assets` / `rename_file`. The pairs passed in are exactly the
/// ones the undo ACTUALLY reverted (`UndoResult.reverted_pairs`), so a file
/// whose undo failed (source lost, or target occupied by an unrelated
/// placeholder) keeps its binding at `new_path` instead of having it stripped.
/// Using the real per-file result — rather than an `original.exists()` guess —
/// also correctly handles case-only rename undos, where `new_path` still
/// `exists()`-resolves to the restored file on case-insensitive filesystems.
/// No-op when tags were never loaded this session (the same lazy-load guard the
/// forward ops and the watcher cleanup use).
fn carry_tags_after_undo(state: &mut project::ProjectState, reverted_pairs: &[(String, String)]) {
    if reverted_pairs.is_empty() || state.tags_data.is_none() {
        return;
    }
    let tags = state.ensure_tags();
    for (original, new_path) in reverted_pairs {
        tags.rename_path(new_path, original);
    }
    // Worth a log line even though the undo itself succeeded: memory now says
    // the bindings sit at the restored paths while disk still says the new
    // ones, and on the next launch the watcher reaps those as orphans.
    if let Err(e) = state.save_tags() {
        eprintln!("[undo] failed to save tags after carrying them back: {}", e);
    }
}

#[tauri::command]
fn get_undo_history(project_id: String) -> Vec<undo::HistoryEntry> {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.get_history())).unwrap_or_default()
}

#[tauri::command]
fn undo_last_operation(project_id: String) -> Result<undo::UndoResult, String> {
    project::with_mut(&project_id, |state| {
        let result = state
            .undo_manager
            .undo_last()
            .ok_or_else(|| "No operation to undo".to_string())?;
        // Carry tag bindings back for the files the undo actually reverted
        // (undo.rs has no access to TagsData). `reverted_pairs` excludes any
        // file whose undo failed, so their tags stay put at new_path.
        carry_tags_after_undo(state, &result.reverted_pairs);
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

/// Toggle the webview inspector. Debug builds open it automatically at
/// startup (see the setup hook below), so this exists for the one case that
/// hook doesn't cover: getting it back after closing it. The frontend
/// suppresses the native context menu app-wide, which takes the "Inspect
/// Element" entry with it, and neither WKWebView nor WebView2 gives us a
/// cross-platform key for this — hence a command plus a frontend binding.
///
/// Compiles to a no-op in release: the `devtools` cargo feature is off, so
/// these methods only exist under `debug_assertions` and there is no
/// inspector to toggle. The command stays registered either way so the
/// keybinding needs no build-mode branch of its own.
#[tauri::command]
fn toggle_devtools(window: tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    {
        if window.is_devtools_open() {
            window.close_devtools();
        } else {
            window.open_devtools();
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = window;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|_app| {
            // Thumbnail keys carry the source file's mtime, so an edited image
            // strands its old thumbnail permanently. Sweep once per launch,
            // off the main thread: it stats every file in the cache directory.
            std::thread::spawn(thumbnail::prune_cache);

            // Debug builds auto-open the inspector. `open_devtools` (and the
            // inspector itself) only exists under `debug_assertions` now that
            // the `devtools` cargo feature is off — release builds ship
            // without it (see the tauri dependency note in Cargo.toml).
            // `_app` + the scoped Manager import keep release builds free of
            // unused warnings once this block compiles away.
            #[cfg(debug_assertions)]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
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
            scan_project_incremental,
            cancel_scan,
            clear_scan_cache,
            start_watching,
            stop_watching,
            get_thumbnail,
            get_thumbnail_cache_size,
            clear_thumbnail_cache,
            // Analysis
            analyze_assets,
            read_project_config,
            ensure_project_config,
            suggest_tags,
            // Git
            get_git_info,
            get_git_statuses,
            // Unity
            get_unity_dependencies,
            find_unused_assets,
            get_godot_dependencies,
            godot_asset_references,
            // Stats / export
            get_project_stats,
            export_to_json,
            export_to_csv,
            export_issues_to_json,
            export_to_html,
            save_text_file,
            // Batch ops
            preview_batch_rename,
            execute_batch_rename,
            // Fix-it (auto-fixable naming)
            preview_naming_fixes,
            apply_naming_fixes,
            // Engine info
            get_unity_file_info,
            get_unity_project_info,
            get_godot_project_info,
            get_unreal_project_info,
            // Undo
            get_undo_history,
            undo_last_operation,
            can_undo,
            clear_undo_history,
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
            add_tag_to_asset,
            remove_tag_from_asset,
            add_tag_to_assets,
            get_all_asset_tags,
            // LLM tagging
            llm_estimate_cost,
            estimate_learning_cost,
            llm_suggest_tags,
            llm_clear_cache,
            llm_cache_size,
            llm_ollama_models,
            learn_project_conventions,
            read_ai_rules,
            save_ai_rules,
            read_project_meta,
            write_project_meta,
            // Developer tools
            toggle_devtools
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_targets_reject_separators_and_degenerates() {
        // A separator in new_name turns `parent.join(new_name)` into a
        // directory traversal — the backend must reject it even though the
        // dialogs validate too (defense in depth at the IPC boundary).
        assert!(validate_new_name("../evil.png").is_err());
        assert!(validate_new_name("sub/inner.png").is_err());
        assert!(validate_new_name("sub\\inner.png").is_err());
        assert!(validate_new_name("").is_err());
        assert!(validate_new_name(".").is_err());
        assert!(validate_new_name("..").is_err());
        assert!(validate_new_name("normal_name.png").is_ok());
        // Dotfiles are odd but legal targets.
        assert!(validate_new_name(".hidden").is_ok());
    }

    /// Two Windows landmines, rejected on every platform deliberately: a name
    /// minted on macOS lands in the repository a Windows teammate has to check
    /// out, and a guard compiled only on one platform is a guard nobody
    /// exercises where the work gets done.
    #[test]
    fn rename_targets_reject_windows_reserved_names_and_trailing_punctuation() {
        // Win32 resolves these to devices whatever the extension, so the file
        // cannot be created at all — the rename fails with an OS error nobody
        // can act on.
        for name in ["CON", "con.png", "NUL.txt", "aux.fbx", "COM1.wav", "lpt9"] {
            assert!(
                validate_new_name(name).is_err(),
                "{} is a reserved device name and must be rejected",
                name
            );
        }
        // Win32 strips a trailing space or period on create, so the file lands
        // under a name that is not the one recorded in the undo stack or the
        // tag bindings — three views of one file that no longer agree.
        for name in ["rock.png.", "rock.png ", "trailing "] {
            assert!(
                validate_new_name(name).is_err(),
                "{:?} ends in stripped punctuation and must be rejected",
                name
            );
        }
        // Merely containing a reserved word is fine — the device name has to
        // be the whole stem.
        for name in ["console.png", "connect.fbx", "my_con.png", "AUXILIARY.wav"] {
            assert!(
                validate_new_name(name).is_ok(),
                "{} is an ordinary name and must be allowed",
                name
            );
        }
    }

    /// Build a one-directory scan result so engine commands can run against a
    /// hand-made asset set without going through a real scan.
    #[cfg(test)]
    fn scan_of(root: &std::path::Path, assets: Vec<scanner::AssetInfo>) -> scanner::ScanResult {
        let root_path = scanner::path_to_string(root);
        scanner::ScanResult {
            directory_tree: scanner::DirectoryNode {
                name: root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: root_path.clone(),
                children: Vec::new(),
                file_count: assets.len(),
                total_size: 0,
            },
            total_count: assets.len(),
            total_size: 0,
            type_counts: HashMap::new(),
            project_type: Some(scanner::ProjectType::Unity),
            root_path,
            assets,
        }
    }

    #[cfg(test)]
    fn unity_asset(
        path: &std::path::Path,
        asset_type: scanner::AssetType,
        guid: Option<&str>,
    ) -> scanner::AssetInfo {
        scanner::AssetInfo {
            path: scanner::path_to_string(path),
            name: path.file_name().unwrap().to_string_lossy().to_string(),
            extension: path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default(),
            asset_type,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: guid.map(|g| g.to_string()),
        }
    }

    /// A sprite atlas exists to list the sprites it packs — it is a pure
    /// reference holder. Leaving `.spriteatlas` out of the reference-source
    /// set made every sprite that only an atlas points at look unused, which
    /// is the one failure mode that talks a user into deleting a live asset.
    ///
    /// `unreadable_sources` is asserted too, and that is the sharp end: the
    /// extension list in this file and `UnityFileType::from_extension` are two
    /// gates in series. Widening only the first one gets the atlas handed to a
    /// parser that still refuses it, which counts as an unreadable source and
    /// raises the "don't trust this list" banner on every atlas project —
    /// swapping a silent false positive for a loud one.
    #[test]
    fn sprite_atlas_keeps_the_sprites_it_packs_out_of_the_unused_list() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let hero_guid = "1234567890abcdef1234567890abcdef";

        let hero = dir.path().join("hero.png");
        std::fs::write(&hero, "png").unwrap();
        let atlas = dir.path().join("Heroes.spriteatlas");
        std::fs::write(
            &atlas,
            format!(
                "%YAML 1.1\n--- !u!687078895 &1\nSpriteAtlas:\n  m_PackedSprites:\n  - {{fileID: 21300000, guid: {}, type: 3}}\n",
                hero_guid
            ),
        )
        .unwrap();

        let project_id = "test_spriteatlas_unused";
        project::register(project_id.to_string(), scanner::path_to_string(dir.path()));
        let scan = scan_of(
            dir.path(),
            vec![
                unity_asset(&hero, scanner::AssetType::Texture, Some(hero_guid)),
                unity_asset(&atlas, scanner::AssetType::Other, None),
            ],
        );
        project::with_mut(project_id, |s| {
            s.cached_scan = Some(scan);
            Ok(())
        })
        .unwrap();

        let result = find_unused_assets(project_id.to_string()).unwrap();
        project::unregister(project_id);

        assert_eq!(
            result.unreadable_sources, 0,
            "the atlas must parse, not be counted as unreadable"
        );
        assert!(
            !result.unused.contains(&scanner::path_to_string(&hero)),
            "a sprite the atlas packs is in use, got unused = {:?}",
            result.unused
        );
    }

    /// `move_assets` guarded its destination with a bare `dst.exists()` while
    /// the rename path beside it asks `paths_are_same_file`. So any spelling
    /// of the target directory that isn't byte-identical to the source's
    /// parent — a case variant on a case-insensitive filesystem, a symlinked
    /// folder, a `..` in the middle — reported "Target already exists" about
    /// the very file being moved.
    ///
    /// The fixture uses `..` rather than a case variant because case is not
    /// portable: the same fixture is a no-op on macOS and two distinct files
    /// on Linux CI. `..` names the same directory on every filesystem, and it
    /// reaches the identical guard.
    #[test]
    fn moving_a_file_into_its_own_directory_under_another_spelling_is_a_no_op() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let hero = sub.join("hero.png");
        std::fs::write(&hero, "x").unwrap();

        let project_id = "test_move_same_dir_alias";
        project::register(project_id.to_string(), scanner::path_to_string(dir.path()));
        let alias = sub.join("..").join("sub");
        let result = move_assets(
            project_id.to_string(),
            vec![scanner::path_to_string(&hero)],
            scanner::path_to_string(&alias),
        );
        project::unregister(project_id);

        assert!(
            result.errors.is_empty(),
            "moving a file into the directory it is already in must not error: {:?}",
            result.errors
        );
        assert!(hero.exists(), "the file must still be on disk");
    }

    #[test]
    fn rename_batch_on_disk_renames_heterogeneous_targets() {
        // The Fix-it engine's differentiator vs. execute_batch_rename: each
        // file gets its OWN target name in one batch.
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let a = dir.path().join("my file.png");
        let b = dir.path().join("rock.fbx");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();

        let planned = vec![
            (a.to_string_lossy().to_string(), "my_file.png".to_string()),
            (b.to_string_lossy().to_string(), "SM_rock.fbx".to_string()),
        ];
        let (done, result) = rename_batch_on_disk(planned);

        assert_eq!(result.success_count, 2);
        assert_eq!(result.error_count, 0);
        assert!(result.errors.is_empty());
        assert_eq!(done.len(), 2);
        assert!(dir.path().join("my_file.png").exists());
        assert!(dir.path().join("SM_rock.fbx").exists());
        assert!(!a.exists() && !b.exists());
        // Successes report forward-slash-normalized new paths so the undo /
        // tag keys match what the next scan produces.
        assert!(done.iter().all(|(_, np)| !np.contains('\\')));
    }

    #[test]
    fn rename_batch_on_disk_skips_noops_and_rejects_bad_names() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let same = dir.path().join("keep.png");
        let bad = dir.path().join("bad.png");
        std::fs::write(&same, "x").unwrap();
        std::fs::write(&bad, "y").unwrap();

        let planned = vec![
            // no-op: target equals current name → neither success nor error
            (same.to_string_lossy().to_string(), "keep.png".to_string()),
            // path separator in the target → rejected at the IPC-safety guard
            (bad.to_string_lossy().to_string(), "sub/evil.png".to_string()),
        ];
        let (done, result) = rename_batch_on_disk(planned);

        assert_eq!(result.success_count, 0);
        assert_eq!(result.error_count, 1); // only the bad name counts
        assert!(done.is_empty());
        assert!(bad.exists() && same.exists()); // both untouched on disk
    }

    #[test]
    fn rename_batch_on_disk_reports_intra_batch_collision() {
        // Two proposals resolving to the same name in the same directory:
        // the first lands, the second must fail with "target already exists"
        // (the fs guard is the backstop behind the preview's `collides` flag).
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let a = dir.path().join("a b.png");
        let b = dir.path().join("a+b.png");
        std::fs::write(&a, "1").unwrap();
        std::fs::write(&b, "2").unwrap();

        let planned = vec![
            (a.to_string_lossy().to_string(), "a_b.png".to_string()),
            (b.to_string_lossy().to_string(), "a_b.png".to_string()),
        ];
        let (done, result) = rename_batch_on_disk(planned);

        assert_eq!(result.success_count, 1);
        assert_eq!(result.error_count, 1);
        assert_eq!(done.len(), 1);
        assert!(dir.path().join("a_b.png").exists());
        // Exactly one original survives (the one that lost the race).
        assert_eq!(a.exists() as u8 + b.exists() as u8, 1);
    }

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

    /// The tag-migration race. `commit_renames` used to run the ENTIRE disk
    /// batch with the project lock free and only then take the lock to migrate
    /// tag bindings. The watcher releases events by individual age (500ms
    /// timeout, 125ms tick — see notify-debouncer-full's `debounced_events`),
    /// not after a quiet period, so on a batch longer than the window it fires
    /// mid-flight, sees the early files' old paths gone, and reaps their tag
    /// bindings as orphans (watcher.rs). The later migration then looked up an
    /// old key that no longer existed and silently did nothing: the tags of
    /// every early file were lost from memory AND disk, with no log line. Small
    /// batches finished inside the window and looked fine.
    ///
    /// Deterministic proof of the fix — no sleep-and-hope for the race itself:
    /// hold the project lock, start the rename on another thread, and assert it
    /// cannot touch the disk while the lock is held. That mutual exclusion is
    /// precisely what leaves the watcher no window to observe a renamed-away
    /// file whose tag hasn't moved yet.
    #[test]
    fn commit_renames_does_not_expose_renamed_files_before_tags_follow() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = scanner::path_to_string(dir.path());
        let project_id = "test_commit_renames_tag_race";
        project::register(project_id.to_string(), root);

        // Enough files that the old code's unlocked batch is plainly observable.
        let planned: Vec<(String, String)> = (0..12)
            .map(|i| {
                let src = dir.path().join(format!("old_{}.png", i));
                std::fs::write(&src, "x").unwrap();
                (scanner::path_to_string(&src), format!("new_{}.png", i))
            })
            .collect();

        // Bind a tag to every source path so the migration has work to do.
        let tag_id = project::with_mut(project_id, |state| {
            let tags = state.ensure_tags();
            let tag = tags.create_tag("race".to_string(), "#fff".to_string());
            for (path, _) in &planned {
                tags.add_tag_to_asset(path, &tag.id);
            }
            Ok(tag.id)
        })
        .unwrap();

        let guard_holder = project::get(project_id).unwrap();
        let held = guard_holder.lock();

        let worker = {
            let planned = planned.clone();
            std::thread::spawn(move || commit_renames(project_id, planned, "Race"))
        };

        // While the lock is held, no rename may land on disk. Without the fix
        // the worker renames all twelve immediately and this fails.
        std::thread::sleep(std::time::Duration::from_millis(300));
        for (path, _) in &planned {
            assert!(
                Path::new(path).exists(),
                "{} was renamed while the project lock was held — the watcher \
                 could observe it before its tag binding moved",
                path
            );
        }

        drop(held);
        let result = worker.join().unwrap();
        assert_eq!(result.success_count, planned.len());
        assert_eq!(result.error_count, 0);

        // Every binding followed its file; none were left on an old path.
        project::with_ref(project_id, |state| {
            let tags = state.tags_data.as_ref().expect("tags were created above");
            for (old_path, new_name) in &planned {
                let new_path = Path::new(old_path).with_file_name(new_name);
                let new_key = scanner::path_to_string(&new_path);
                assert!(
                    tags.get_asset_tags(&new_key).iter().any(|t| t.id == tag_id),
                    "tag did not follow {} → {}",
                    old_path,
                    new_key
                );
                assert!(tags.get_asset_tags(old_path).is_empty());
            }
            Ok(())
        })
        .unwrap();

        project::unregister(project_id);
    }

    /// `move_assets` carried the identical defect as `commit_renames`: the whole
    /// disk loop ran with the lock free, tags migrated afterwards. Right-click →
    /// "Move to…" is a common bulk action, so the exposure was the same.
    #[test]
    fn move_assets_does_not_expose_moved_files_before_tags_follow() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = scanner::path_to_string(dir.path());
        let project_id = "test_move_assets_tag_race";
        project::register(project_id.to_string(), root);

        let target = dir.path().join("dest");
        std::fs::create_dir(&target).unwrap();

        let sources: Vec<String> = (0..12)
            .map(|i| {
                let src = dir.path().join(format!("m_{}.png", i));
                std::fs::write(&src, "x").unwrap();
                scanner::path_to_string(&src)
            })
            .collect();

        let tag_id = project::with_mut(project_id, |state| {
            let tags = state.ensure_tags();
            let tag = tags.create_tag("move".to_string(), "#fff".to_string());
            for path in &sources {
                tags.add_tag_to_asset(path, &tag.id);
            }
            Ok(tag.id)
        })
        .unwrap();

        let guard_holder = project::get(project_id).unwrap();
        let held = guard_holder.lock();

        let worker = {
            let paths = sources.clone();
            let target_dir = scanner::path_to_string(&target);
            std::thread::spawn(move || {
                move_assets(project_id.to_string(), paths, target_dir)
            })
        };

        std::thread::sleep(std::time::Duration::from_millis(300));
        for path in &sources {
            assert!(
                Path::new(path).exists(),
                "{} moved while the project lock was held",
                path
            );
        }

        drop(held);
        let result = worker.join().unwrap();
        assert_eq!(result.successes.len(), sources.len());
        assert!(
            result.errors.is_empty(),
            "unexpected errors: {:?}",
            result.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
        );

        project::with_ref(project_id, |state| {
            let tags = state.tags_data.as_ref().expect("tags were created above");
            for s in &result.successes {
                assert!(
                    tags.get_asset_tags(&s.new_path).iter().any(|t| t.id == tag_id),
                    "tag did not follow {} → {}",
                    s.original_path,
                    s.new_path
                );
                assert!(tags.get_asset_tags(&s.original_path).is_empty());
            }
            Ok(())
        })
        .unwrap();

        project::unregister(project_id);
    }

    /// The CSV export faces the same threat the HTML export already defends
    /// against, from the same source (file names the user did not choose):
    /// a cell whose text starts with `=`, `+`, `-` or `@` is a *formula* to
    /// Excel / LibreOffice / Sheets, and `=cmd|'/c calc'!A1` is the classic
    /// proof that it reaches the shell. Such names are perfectly legal on
    /// disk, so an asset library shared as CSV carries the payload along.
    #[test]
    fn csv_cells_cannot_smuggle_a_formula_into_a_spreadsheet() {
        for dangerous in ["=cmd|'/c calc'!A1", "+1+1", "-1+1", "@SUM(A1)"] {
            let cell = csv_cell(dangerous);
            assert!(
                cell.starts_with("\"'"),
                "{dangerous} must be neutralized, got {cell}"
            );
        }

        // Tab and carriage return lead a formula just as well.
        assert!(csv_cell("\t=1+1").starts_with("\"'"));
        assert!(csv_cell("\r=1+1").starts_with("\"'"));

        // Ordinary values are untouched apart from the quoting that was
        // always there, and embedded quotes still double.
        assert_eq!(csv_cell("hero.png"), "\"hero.png\"");
        assert_eq!(csv_cell(r#"a"b.png"#), "\"a\"\"b.png\"");
        assert_eq!(csv_cell(""), "\"\"");
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

    #[test]
    fn render_template_substitutes_named_placeholders() {
        let args = HashMap::from([
            ("width".to_string(), "1024".to_string()),
            ("height".to_string(), "768".to_string()),
        ]);
        assert_eq!(
            render_template("贴图 {{width}}×{{height}} 超过上限", &args),
            "贴图 1024×768 超过上限"
        );
    }

    #[test]
    fn render_template_leaves_unknown_placeholders_alone() {
        // Matches i18next's default, so a translator's typo looks the same in
        // the exported report as it does in the UI. The template gate is what
        // stops it from shipping; this is only about the two ends agreeing.
        let args = HashMap::from([("width".to_string(), "1024".to_string())]);
        assert_eq!(
            render_template("{{width}} / {{witdh}}", &args),
            "1024 / {{witdh}}"
        );
    }

    #[test]
    fn render_template_passes_through_a_template_with_no_placeholders() {
        assert_eq!(render_template("纯文本", &HashMap::new()), "纯文本");
    }

    #[test]
    fn html_issue_rows_use_the_supplied_templates_and_escape_after_interpolation() {
        let issue = analyzer::Issue {
            rule_id: "texture.max_size".to_string(),
            rule_name: "Texture Too Large".to_string(),
            severity: analyzer::Severity::Warning,
            message: "Texture 4096x4096 exceeds maximum size 2048".to_string(),
            asset_path: "/p/a<b>.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::from([
                ("width".to_string(), "4096".to_string()),
                ("height".to_string(), "4096".to_string()),
                ("max".to_string(), "2048".to_string()),
            ]),
        };
        let templates = HashMap::from([
            ("texture.max_size.title".to_string(), "贴图尺寸过大".to_string()),
            (
                "texture.max_size.message".to_string(),
                "贴图 {{width}}×{{height}} 超过上限 {{max}}".to_string(),
            ),
        ]);

        let (title, message) = localized_issue_cells(&issue, Some(&templates));
        assert_eq!(title, "贴图尺寸过大");
        assert_eq!(message, "贴图 4096×4096 超过上限 2048");

        // No templates: the analyzer's own prose, unchanged.
        let (title_en, message_en) = localized_issue_cells(&issue, None);
        assert_eq!(title_en, "Texture Too Large");
        assert_eq!(message_en, "Texture 4096x4096 exceeds maximum size 2048");
    }

    /// A `dcc_source.outdated_export` issue as the rule emits it: `age_unit`
    /// is the bucket tag `humanize_seconds` chose, not a word.
    fn outdated_export_issue() -> analyzer::Issue {
        analyzer::Issue {
            rule_id: "dcc_source.outdated_export".to_string(),
            rule_name: "Outdated DCC export".to_string(),
            severity: analyzer::Severity::Warning,
            message: "Source `character.blend` is 3d newer than its export `character.fbx` — possibly missing a re-export.".to_string(),
            asset_path: "/p/character.blend".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::from([
                ("source".to_string(), "character.blend".to_string()),
                ("export".to_string(), "character.fbx".to_string()),
                ("dcc".to_string(), "Blender".to_string()),
                ("age_value".to_string(), "3".to_string()),
                ("age_unit".to_string(), "d".to_string()),
            ]),
        }
    }

    const OUTDATED_EXPORT_ZH: &str =
        "源文件 {{source}} 比它的导出 {{export}} 新 {{age_value}} {{age_unit}}，可能漏了一次重新导出。";

    #[test]
    fn age_unit_resolves_through_the_duration_templates() {
        // The UI looks the tag up in `issues.duration.*` before interpolating;
        // the report has to do the same or the two disagree on one issue.
        let templates = HashMap::from([
            (
                "dcc_source.outdated_export.message".to_string(),
                OUTDATED_EXPORT_ZH.to_string(),
            ),
            ("duration.d".to_string(), "天".to_string()),
            ("duration.h".to_string(), "小时".to_string()),
        ]);
        let (_, message) = localized_issue_cells(&outdated_export_issue(), Some(&templates));
        assert_eq!(
            message,
            "源文件 character.blend 比它的导出 character.fbx 新 3 天，可能漏了一次重新导出。"
        );
    }

    #[test]
    fn age_unit_falls_back_to_the_raw_tag_when_the_duration_templates_are_absent() {
        // A locale that translated the rule but not the units. The tag is what
        // the English prose prints, so the sentence still reads.
        let templates = HashMap::from([(
            "dcc_source.outdated_export.message".to_string(),
            OUTDATED_EXPORT_ZH.to_string(),
        )]);
        let (_, message) = localized_issue_cells(&outdated_export_issue(), Some(&templates));
        assert_eq!(
            message,
            "源文件 character.blend 比它的导出 character.fbx 新 3 d，可能漏了一次重新导出。"
        );
    }

    #[test]
    fn html_escaping_happens_after_interpolation_not_before() {
        // The template is ours and carries no markup; the arg values come from
        // file names and can. Escaping the composed string once covers both —
        // escaping the template first would double-escape nothing and escaping
        // only the template would let `<` through from the args.
        let issue = analyzer::Issue {
            rule_id: "naming.forbidden_char".to_string(),
            rule_name: "Forbidden Character".to_string(),
            severity: analyzer::Severity::Warning,
            message: "File name contains forbidden character: '<'".to_string(),
            asset_path: "/p/a<b.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::from([("char".to_string(), "<".to_string())]),
        };
        let templates = HashMap::from([(
            "naming.forbidden_char.message".to_string(),
            "文件名含禁用字符 {{char}}".to_string(),
        )]);
        let (_, message) = localized_issue_cells(&issue, Some(&templates));
        assert_eq!(html_escape(&message), "文件名含禁用字符 &lt;");
    }

    /// End of the wire the panel actually reads: a real corrupt rules file on
    /// disk, through the real command (registry, tags snapshot, the
    /// already-suggested filter), serialized the way Tauri serializes it.
    /// `rule_suggest`'s own tests cover the decision; this covers the plumbing
    /// between it and the frontend, which is the part with no type checking
    /// across it.
    #[test]
    fn suggest_tags_reports_a_corrupt_rules_file_over_the_wire() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("tidycraft.ai.toml"),
            "rules = this is not toml",
        )
        .unwrap();

        // Enough shared-token files for the heuristic fallback to produce a
        // group, so an empty `groups` can't mask a missing fallback.
        let assets: Vec<scanner::AssetInfo> = ["Hero", "Villain", "Rock", "Tree"]
            .iter()
            .map(|n| {
                let p = dir.path().join(format!("T_{n}_BaseColor.png"));
                std::fs::write(&p, "x").unwrap();
                unity_asset(&p, scanner::AssetType::Texture, None)
            })
            .collect();

        let project_id = "test_suggest_tags_corrupt_rules";
        project::register(
            project_id.to_string(),
            scanner::path_to_string(dir.path()),
        );
        project::with_mut(project_id, |s| {
            s.cached_scan = Some(scan_of(dir.path(), assets));
            Ok(())
        })
        .unwrap();

        let out = suggest_tags(project_id.to_string()).unwrap();
        project::unregister(project_id);

        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(
            json["warnings"][0]["kind"], "rules_unreadable",
            "the panel has nothing else to tell the user by"
        );
        assert!(
            json["warnings"][0]["detail"]
                .as_str()
                .is_some_and(|d| !d.is_empty()),
            "the parse error is what makes the file fixable"
        );
        assert!(
            json["groups"].as_array().is_some_and(|g| !g.is_empty()),
            "fallback still has to fill the panel"
        );
    }
}

