mod analyzer;
mod cache;
mod fs_atomic;
mod git;
mod godot;
mod llm;
mod project;
mod project_path;
mod scanner;
mod sidecar;
mod tags;
mod thumbnail;
mod undo;
mod unity;
mod unreal;
mod warning;
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

/// Ask whether a batch of project paths is still usable. Batched because boot
/// restore asks about every previously open project plus the recents list, and
/// async because an unresponsive network mount can block `stat` for a long time.
#[tauri::command(async)]
fn check_project_paths(paths: Vec<String>) -> Vec<project_path::ProjectPathReport> {
    project_path::check_all(&paths)
}

// ============ Scan Commands ============

/// Spawn a background thread emitting `scan-progress-{project_id}` every 100ms
/// until the scan reaches a terminal phase or the caller flips `stop`. The scan's
/// early `Err` paths never mark a terminal phase, so `stop` is what ends it.
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
    // Frontend-visible: when true (default) the scanner honors `.gitignore` /
    // `.ignore` and skips hidden dot directories. Toggled in Settings →
    // Maintenance.
    respect_gitignore: bool,
) -> Result<IncrementalScanResult, String> {
    project::register(project_id.clone(), path.clone());

    let state = Arc::new(ScanState::new());
    // In-flight guard: `scan_state` being `Some` means another scan already owns
    // this project. Reject the second rather than overwrite the first's state.
    // Check + set is atomic under the project lock held by `with_mut`.
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

    // Stop the reporter and join it before propagating any error: the scan's
    // early `Err` paths never mark a terminal phase, so `join()` would block.
    stop.store(true, Ordering::SeqCst);
    let _ = progress_handle.join();

    // Clear the in-flight guard only if it is still OURS: re-registering this id
    // against a different path rebuilds the state, and a scan of that new root owns
    // the guard by then — clearing blindly strips a live scan of its cancel handle.
    let _ = project::with_mut(&project_id, |s| {
        if s.scan_state
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &state))
        {
            s.scan_state = None;
        }
        Ok(())
    });

    let (scan_result, stats) = join_result
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    // Same reason, for the result itself: cancellation is advisory and the walk's last
    // check sits before the cache/sort/tree tail, so a cancelled scan of the old root
    // still returns a complete `Ok` — installing it would describe the wrong folder.
    project::with_mut(&project_id, |s| {
        if s.root_path == path {
            s.cached_scan = Some(scan_result.clone());
            s.respect_gitignore = respect_gitignore;
        }
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
    let (root_path, respect_gitignore) = project::with_ref(&project_id, |s| {
        Ok((s.root_path.clone(), s.respect_gitignore))
    })?;
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
// `llm_suggest_tags` calls the configured provider; `llm_estimate_cost` and the
// cache commands need no provider.

/// Cost preview for the AIAnalyzeModal. No network and no API key required.
/// Carries the same project framing the real request sends, since every
/// ≤20-asset chunk of the real dispatch re-sends that context.
#[tauri::command(async)]
fn llm_estimate_cost(
    project_id: String,
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

    let (_, existing_tags, project_ctx) = gather_llm_context(&project_id);

    // Dummy per-asset entries: the estimator prices by count and thumbnail
    // presence, so placeholders are fine — the context above has to be real.
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
        project_ctx,
        existing_tags,
    };
    Ok(prov.estimate_cost(&req))
}

/// Convert an absolute asset path to a project-relative one for the LLM prompt
/// and cache key, so providers never receive the user's machine path. Falls back
/// to the bare filename with no project root or for a path outside it.
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
/// enters an LLM prompt or the per-asset cache key. Paths outside the root fall
/// back to their basename, matching `project_relative_path`.
fn relativize_samples(samples: Vec<String>, root: &str) -> Vec<String> {
    samples
        .into_iter()
        .map(|p| project_relative_path(&p, root))
        .collect()
}

/// One quoted CSV cell built from a project-derived string. Quoting alone is not
/// enough: a leading `=`, `+`, `-`, `@`, tab or CR makes spreadsheets read the
/// cell as a formula, so those get a leading apostrophe.
fn csv_cell(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    let leads_a_formula = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let prefix = if leads_a_formula { "'" } else { "" };
    format!("\"{prefix}{escaped}\"")
}

/// Escape the five HTML-significant characters in project-derived strings
/// interpolated into the HTML report. `&` goes first so the entities just
/// inserted are not double-escaped.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Substitute `{{name}}` placeholders, i18next's syntax, so one locale template
/// string works in the webview and in the HTML export alike. Unknown names are
/// left in place, as i18next does.
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
/// carries the interface's current locale, flattened as `<rule_id>.<field>`;
/// absent (English) means the analyzer's own prose.
fn localized_issue_cells(
    issue: &analyzer::Issue,
    templates: Option<&HashMap<String, String>>,
) -> (String, String) {
    // dcc_source's age unit is the one placeholder whose value is itself a word:
    // `args.age_unit` is the raw tag (`"d"`), not a noun. Only the noun is looked
    // up, exactly as `localizeIssue` does, so report and panel agree.
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
    (
        pick("title", &issue.rule_name),
        pick("message", &issue.message),
    )
}

/// Snapshot the project framing an LLM request carries: root path, existing tags
/// (each with up to 5 relativized sample paths), and the `[project]` block from
/// tidycraft.toml. Shared by the real request and the cost preview.
fn gather_llm_context(
    project_id: &str,
) -> (
    String,
    Vec<llm::ExistingTagContext>,
    Option<llm::project_meta::ProjectMeta>,
) {
    // How many existing-asset paths ship per tag: enough usage context for the
    // model to infer a tag's intent without blowing the prompt budget.
    const SAMPLES_PER_TAG: usize = 5;

    let context_result = project::with_mut(project_id, |state| {
        let root = state.root_path.clone();
        let tags_data = state.ensure_tags();
        let mut existing: Vec<llm::ExistingTagContext> = Vec::with_capacity(tags_data.tags.len());
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

    let (root_path, existing_tags) = context_result.unwrap_or_else(|e| {
        eprintln!("[llm] context fetch failed: {e}");
        (String::new(), Vec::new())
    });

    // Read [project] from tidycraft.toml, outside the project lock to avoid
    // holding it through file IO. Missing file / parse failure / empty meta
    // all collapse to None — no project block.
    let project_ctx: Option<llm::project_meta::ProjectMeta> = if root_path.is_empty() {
        None
    } else {
        let toml_path = Path::new(&root_path).join("tidycraft.toml");
        std::fs::read_to_string(&toml_path)
            .ok()
            .and_then(|content| llm::project_meta::ProjectMeta::from_toml(&content).ok())
            .filter(|m| !m.is_empty())
    };

    (root_path, existing_tags, project_ctx)
}

/// Main entry point for AI tagging: loads thumbnails for the selected assets,
/// gathers project context, then dispatches to the chosen provider via
/// `make_provider`.
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

    let (root_path, existing_tags, project_ctx) = gather_llm_context(&project_id);

    // Map the project-relative path (what ships to the provider and comes back
    // from the model) to the absolute path the frontend needs to bind tags.
    // Built before `asset_paths` is moved into the builders.
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
                    let filename = p.rsplit(['/', '\\']).next().unwrap_or(&p).to_string();
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
                let filename = p.rsplit(['/', '\\']).next().unwrap_or(&p).to_string();
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

    // Suggestions come back keyed by project-relative paths. Remap each to the
    // absolute path so the frontend binds tags to the scanned assets; a miss
    // leaves it untouched.
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

/// AI Learning entry point. Samples the project, sends the samples, tag system
/// and project meta to the LLM, and returns the full `LearningResult` for the
/// review panel.
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

    // Staged in memory — nothing is written to disk here. The review panel's Save
    // (`save_ai_rules`) is the single commit point, so closing the panel without
    // saving discards the run and unreviewed rules never reach `suggest_tags`.
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

/// Everything a learning prompt is built from: the sampled directories, the
/// `[project]` theme/goal, and the existing tag vocabulary.
type LearningInputs = (
    Vec<llm::learning::DirectorySample>,
    Option<llm::project_meta::ProjectMeta>,
    Vec<llm::ExistingTagContext>,
);

/// Snapshot and sample exactly what a learning run would send: scan, tags and
/// root under the project lock, then `[project]` meta and the deterministic
/// sampling outside it. Shared by the real call and its cost estimator.
fn build_learning_inputs(project_id: &str, depth: usize) -> Result<LearningInputs, String> {
    // Snapshot scan + tags + root_path inside the project lock, then drop it
    // before any IO or async work.
    const SAMPLES_PER_TAG: usize = 5;
    let snapshot = project::with_mut(project_id, |state| {
        let root = state.root_path.clone();
        let scan = state
            .cached_scan
            .clone()
            .ok_or("Project hasn't been scanned yet")?;
        let tags_data = state.ensure_tags();
        let mut existing: Vec<llm::ExistingTagContext> = Vec::with_capacity(tags_data.tags.len());
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

/// Cost preview for the LearnSetupModal. Pure local math: builds the same prompt
/// a learning run would send — same sampler, same seed, same builder — and prices
/// that.
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

/// Read the project's `tidycraft.ai.toml` if it exists, for the AITagPanel's
/// status badge. Reads only the saved doc — a learning run still pending review
/// is not active and does not show here.
#[tauri::command]
fn read_ai_rules(project_id: String) -> Result<Option<llm::rule_store::AiRulesDoc>, String> {
    project::with_ref(&project_id, |state| {
        llm::rule_store::AiRulesDoc::load(Path::new(&state.root_path))
    })
}

/// The review panel's Save: the single point where learned rules reach disk.
/// Takes the doc staged by `learn_project_conventions` and writes it with the
/// user-edited rule list. See `AiRulesDoc::for_save` for the exact precedence.
#[tauri::command]
fn save_ai_rules(project_id: String, rules: Vec<llm::learning::LearnedRule>) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        let root = Path::new(&state.root_path);
        let pending = state.pending_ai_rules.clone();
        let on_disk = if pending.is_none() {
            llm::rule_store::AiRulesDoc::load(root)?
        } else {
            None
        };
        llm::rule_store::AiRulesDoc::for_save(pending, on_disk, rules).save(root)?;
        // Cleared only once the write landed: a failed save keeps the staged
        // run, so a retry still carries its provenance.
        state.pending_ai_rules = None;
        Ok(())
    })
}

/// Read the `[project]` block from `tidycraft.toml`, used to pre-fill the
/// LearnSetupModal. Empty or missing returns defaults so the inputs render as
/// placeholders.
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

/// Persist `theme` and `goal` from the LearnSetupModal into `tidycraft.toml`'s
/// `[project]` block via `toml_edit`, so the user's comments and other sections
/// survive. Creates the file from `DEFAULT_CONFIG_TEMPLATE` when absent.
#[tauri::command]
fn write_project_meta(project_id: String, theme: String, goal: String) -> Result<(), String> {
    project::with_ref(&project_id, |state| {
        llm::project_meta::write_back(Path::new(&state.root_path), &theme, &goal)
    })
}

#[tauri::command]
fn llm_cache_size() -> u64 {
    llm::cache::size()
}

/// List the models installed on a local Ollama daemon. The endpoint argument is
/// the user's Settings base URL, stripped of any path suffix. Vision capability
/// is not filtered server-side — the interface shows everything installed.
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
/// exporters. Absent file → defaults; present but unreadable or unparseable →
/// `Err`, matching how the Issues view fails via `analyze_assets`.
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
/// `[ignore].patterns` filter, then run every analyzer phase. `analyze_assets`
/// and both report exporters route through this for one issue set per config.
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
            warnings: scan_result.warnings.clone(),
        }
    });
    let scan_to_analyze: &ScanResult = owned_filtered.as_ref().unwrap_or(scan_result);

    let analyzer = Analyzer::with_config(config);
    let mut result = analyzer.analyze(scan_to_analyze);
    let duplicates = analyzer.find_duplicates(scan_to_analyze);
    result.merge(duplicates);
    // Existence comes from the UNFILTERED scan: `[ignore]` limits what is
    // reported, not what the project contains. The other three cross-asset rules
    // keep the filtered view — see docs/analyzer-rules.md.
    let missing = analyzer.find_missing_references(scan_to_analyze, scan_result, package_index);
    result.merge(missing);
    let pbr = analyzer.find_pbr_set_issues(scan_to_analyze, &config.pbr_set);
    result.merge(pbr);
    let dcc = analyzer.find_dcc_source_issues(scan_to_analyze, &config.dcc_source);
    result.merge(dcc);
    result
}

/// Clone what an analysis or report export needs out of the project state,
/// holding the project lock only for the clone. The heavy work — duplicate
/// re-hashing, engine re-parsing, templating — then runs with the lock released.
fn scan_snapshot(project_id: &str) -> Result<(ScanResult, String), String> {
    project::with_ref(project_id, |state| {
        let scan = state.require_scan()?;
        Ok((scan.clone(), state.root_path.clone()))
    })
}

// `(async)`: duplicate hashing plus a full engine re-parse is heavy. The frontend
// contract is unchanged, since `invoke` already awaits.
#[tauri::command(async)]
fn analyze_assets(
    project_id: String,
    config_toml: Option<String>,
) -> Result<AnalysisResult, String> {
    let config = if let Some(toml_str) = config_toml {
        RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
    } else {
        RuleConfig::default()
    };

    // Build the ignore matcher up-front so a malformed pattern surfaces as
    // an error before we touch the per-project lock.
    let ignore_set = build_ignore_set(&config)?;
    // Fetched outside the lock — see package_index_for.
    let package_index = package_index_for(&project_id);

    let (scan_result, root_path) = scan_snapshot(&project_id)?;
    Ok(run_full_analysis(
        &scan_result,
        &root_path,
        &config,
        ignore_set.as_ref(),
        &package_index,
    ))
}

/// Ensure `<project_root>/tidycraft.toml` exists, writing the commented default
/// template if it does not, then return its absolute path. The frontend hands
/// that path to `open_with_default_app`.
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

/// Read a project's `tidycraft.toml` from its registered root. `Ok(None)` means
/// the file does not exist, which is a normal state. Validation and parsing
/// happen later in `analyze_assets`.
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
        // Tags already created by an earlier suggest+apply round. Compared
        // against `<group_name> (suggested)` because applyGroup always appends
        // that suffix.
        let already_suggested: std::collections::HashSet<String> = state
            .ensure_tags()
            .tags
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let scan = state.require_scan()?;
        let root = Path::new(&state.root_path);

        // Prefer AI-derived rules when present; `RuleSuggester` produces the same
        // `TagGroup[]` shape, so only the `hint` string differs. Falls back to the
        // heuristic suggester, reporting the reason in `warnings`.
        let mut suggestions = analyzer::rule_suggest::load_or_fallback(scan, root);

        suggestions
            .groups
            .retain(|g| !already_suggested.contains(&format!("{} (suggested)", g.name)));
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
                    // asset paths on Windows; `workdir().join(rel)` yields mixed
                    // separators there.
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

/// How firmly a graph node's identity resolves. A disk scan undercounts what a
/// project can legitimately reference (engine built-ins, package caches,
/// gitignored files), so each variant asserts only what the evidence supports.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DependencyNodeKind {
    /// A scanned project asset — has a real `path`, clickable in the UI.
    Asset,
    /// Unity: a referenced GUID resolved through the `Library/PackageCache`
    /// index — a package asset installed by the package manager. Known to
    /// exist; simply not part of the project's own assets.
    Package,
    /// Unity: a referenced GUID with neither a scanned asset nor a package-index
    /// hit. Ambiguous by construction — a package asset, an ignore-excluded file
    /// and a broken reference look alike from a disk scan.
    Unresolved,
    /// Godot: a `res://` target that exists on disk but sits outside the scan
    /// set (gitignored / hidden directory). Not breakage.
    Unscanned,
    /// Godot: a `res://` target that does not exist on disk — confirmed broken.
    Missing,
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
    /// See `DependencyNodeKind`. Non-`asset` nodes carry an empty `path` and are
    /// BFS terminals in the frontend, so a widely-shared unresolved GUID cannot
    /// hub-connect unrelated referrers in the 2-hop view.
    pub kind: DependencyNodeKind,
    /// Secondary identity line for the tooltip — the package id for
    /// `package` nodes ("com.unity.render-pipelines.universal"). Absent
    /// elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Cached GUID→package index for this project, rebuilt only when
/// `Library/PackageCache`'s directory listing changes. Callers must grab the Arc
/// BEFORE their own `with_ref` block, or the inner `with_mut` self-deadlocks.
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
/// brief lock so the walk itself runs with the lock released. Worst case the
/// snapshot is one scan stale, which the returned graph already was.
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

    // References the scan cannot resolve. The all-zero sentinel and the built-in
    // bundles never enter the graph; the rest resolves through the PackageCache
    // index, and what remains becomes one deduped `unresolved` node.
    let mut unresolved_guids: std::collections::HashSet<String> = std::collections::HashSet::new();
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

/// Result of an unused-asset scan. `unreadable_sources` counts referenceable
/// files whose text could not be read — almost always binary asset
/// serialization, which makes every asset they referenced look unused.
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
        _ => return Err("Unused-asset detection supports Unity and Godot projects".to_string()),
    }

    let mut referenced_guids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_guids: HashMap<String, String> = HashMap::new();

    for asset in &scan_result.assets {
        // Scenes are graph roots (build settings, the editor, LoadScene by name),
        // so having no incoming GUID reference does not make one unused. They are
        // still parsed as reference sources below.
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
/// scenes, resources and scripts. Same parser and known gaps as the unused check.
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

    // Keep every edge but classify unknown `res://` targets honestly: unlike a
    // Unity GUID a res path can be checked against disk, so "outside the scan but
    // present" and "genuinely gone" get different nodes.
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
/// reference it by `res://` path, root-relative, `project.godot` included.
/// Godot-only — Unity references are GUID-based and survive renames.
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
// Path-only commands (no project_id): they re-read small marker or config files
// on every call, and return `None` rather than an error when the info is absent.

/// On-demand parse of a single Unity YAML asset for the preview panel: component
/// list (prefab and scene only, sorted) plus GUID references.
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

            *extension_distribution
                .entry(asset.extension.clone())
                .or_insert(0) += 1;

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
            *size_distribution
                .entry(size_bucket.to_string())
                .or_insert(0) += 1;

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

        all_files.sort_by_key(|f| std::cmp::Reverse(f.size));
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

// `(async)`: runs a full analysis (incl. duplicate re-hashing) — off the
// main thread, and via `scan_snapshot` off the project lock too.
#[tauri::command(async)]
fn export_issues_to_json(project_id: String) -> Result<String, String> {
    let package_index = package_index_for(&project_id);
    let (scan_result, root_path) = scan_snapshot(&project_id)?;

    // Mirror the interface's Run Analysis: honor the project's tidycraft.toml and
    // run every phase, so the exported report cannot diverge from the Issues view.
    let config = load_rule_config(&root_path)?;
    let ignore_set = build_ignore_set(&config)?;
    let result = run_full_analysis(
        &scan_result,
        &root_path,
        &config,
        ignore_set.as_ref(),
        &package_index,
    );

    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}

/// The one-line admission at the top of the HTML report when the scan under it
/// was incomplete. Empty when there is nothing to admit. CacheNotSaved is not
/// reported: cache health changes nothing about this report's numbers.
fn warning_banner_html(warnings: &[warning::ScanWarning]) -> String {
    use warning::ScanWarning;
    let mut unread = 0usize;
    let mut ignore_broken = false;
    for w in warnings {
        match w {
            ScanWarning::TreeWalkFailed { skipped, .. } => unread += skipped,
            ScanWarning::AssetUnreadable { affected, .. } => unread += affected,
            ScanWarning::IgnoreRulesUnusable { .. } => ignore_broken = true,
            ScanWarning::CacheNotSaved { .. } => {}
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if unread > 0 {
        // Deliberately not "not reflected below": what an unreadable entry leaves behind
        // differs by scan — a full one keeps a zeroed row, an incremental one keeps stale
        // cache values, and only a failed directory walk leaves nothing at all.
        parts.push(format!(
            "{unread} entries could not be read, and are missing or out of date below"
        ));
    }
    if ignore_broken {
        parts.push("the project's ignore rules were not applied".to_string());
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "        <div class=\"warn-banner\">This report is based on an incomplete scan: {}.</div>\n",
        parts.join("; ")
    )
}

/// The report's stylesheet, kept out of the `format!` template so a test can
/// read it directly (`every_asset_type_has_a_report_badge_rule`). Lifting it
/// out also drops the doubled braces the template otherwise needs.
const REPORT_STYLE: &str = r#"    <style>
        /* Palette lifted from redesign-tokens-v2.css (OKLCH converted to hex): a standalone
           report cannot read the app's stylesheet, and a palette of its own is how it drifts.
           Print forces light — `prefers-color-scheme: dark` still matches while printing. */
        :root {
            --bg: #fcf9f7; --panel: #ffffff; --line: #e5e0dc;
            --text: #201914; --text-2: #59514b;
            --primary: #c26300;
            --err: #cc3336; --warn: #b79500; --info: #008aaf;
            --c-texture: #de602f; --c-model: #009365; --c-audio: #9457ce;
            --c-video: #e14660; --c-animation: #009c7b; --c-material: #a98900;
            --c-prefab: #5671d8; --c-scene: #b950b2; --c-script: #0083c9;
            --c-data: #008a77; --c-other: #756d69;
        }
        @media (prefers-color-scheme: dark) {
            :root {
                --bg: #0b0907; --panel: #1d1a18; --line: #36322f;
                --text: #f4f1ed; --text-2: #b7b0a9;
                --primary: #e69825;
                --err: #ff5f5b; --warn: #e9c100; --info: #09b7dc;
                --c-texture: #ff8a5e; --c-model: #1fc893; --c-audio: #c189fc;
                --c-video: #ff6d80; --c-animation: #00cfac; --c-material: #d6b529;
                --c-prefab: #7e9dff; --c-scene: #e57fdd; --c-script: #39b5ff;
                --c-data: #2fbda7; --c-other: #98918d;
            }
        }
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: var(--bg); color: var(--text); padding: 2rem; }
        .container { max-width: 1200px; margin: 0 auto; }
        h1 { color: var(--primary); margin-bottom: 0.5rem; }
        h2 { color: var(--text); margin: 2rem 0 1rem; border-bottom: 1px solid var(--line); padding-bottom: 0.5rem; }
        .meta { color: var(--text-2); margin-bottom: 2rem; }
        .warn-banner { border: 1px solid var(--warn); color: var(--warn); background: var(--panel); border-radius: 6px; padding: 0.6rem 1rem; margin: -1rem 0 2rem; font-size: 0.9rem; }
        .cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 2rem; }
        .card { background: var(--panel); border-radius: 8px; padding: 1.5rem; border: 1px solid var(--line); }
        .card-value { font-size: 2rem; font-weight: bold; color: var(--primary); }
        .card-label { color: var(--text-2); font-size: 0.875rem; margin-top: 0.25rem; }
        table { width: 100%; border-collapse: collapse; background: var(--panel); border-radius: 8px; overflow: hidden; }
        th, td { padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid var(--line); }
        th { background: var(--bg); font-weight: 600; }
        tr:hover { background: var(--bg); }
        /* Outlined rather than tinted: a tint needs a translucent copy of every
           type colour, and the alpha-hex trick the tints used cannot be written
           against a custom property. `currentColor` gets it from one value. */
        .type-badge { display: inline-block; padding: 0.25rem 0.5rem; border-radius: 4px; font-size: 0.75rem; font-weight: 500; border: 1px solid currentColor; }
        .texture { color: var(--c-texture); }
        .model { color: var(--c-model); }
        .audio { color: var(--c-audio); }
        .video { color: var(--c-video); }
        .animation { color: var(--c-animation); }
        .material { color: var(--c-material); }
        .prefab { color: var(--c-prefab); }
        .scene { color: var(--c-scene); }
        .script { color: var(--c-script); }
        .data { color: var(--c-data); }
        .other { color: var(--c-other); }
        .severity-error { color: var(--err); }
        .severity-warning { color: var(--warn); }
        .severity-info { color: var(--info); }
        .chart { display: flex; gap: 2rem; margin-bottom: 2rem; }
        .chart-bar { flex: 1; background: var(--panel); border-radius: 8px; padding: 1rem; }
        .bar { height: 24px; background: var(--primary); border-radius: 4px; margin-bottom: 0.5rem; transition: width 0.3s; }
        .bar-label { display: flex; justify-content: space-between; font-size: 0.875rem; color: var(--text-2); }
        @media print {
            :root {
                --bg: #ffffff; --panel: #ffffff; --line: #cccccc;
                --text: #000000; --text-2: #444444;
                --primary: #8a4600;
                --err: #a3181c; --warn: #6f5a00; --info: #005a72;
                --c-texture: #a8420f; --c-model: #006644; --c-audio: #6a34a0;
                --c-video: #a82a42; --c-animation: #00705a; --c-material: #7a6200;
                --c-prefab: #3a52ab; --c-scene: #8c3486; --c-script: #005f96;
                --c-data: #00655a; --c-other: #55504d;
            }
            body { padding: 0; }
            .card, table, .chart-bar { break-inside: avoid; }
        }
    </style>
"#;

/// `issue_limit` / `asset_limit` cap the report's table rows (Settings →
/// Export). `None` keeps the defaults (100 / 500); `Some(0)` means unlimited.
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
    let (scan_result, root_path) = scan_snapshot(&project_id)?;
    // Block kept (not a lock): the body below predates the de-locking and
    // stays at its old indentation so blame survives.
    {
        // Same analysis pipeline as Run Analysis and the JSON export, so the
        // report's issue list matches the Issues view. The asset inventory cards
        // stay on the full scan — `[ignore]` scopes analysis, not the census.
        let config = load_rule_config(&root_path)?;
        let ignore_set = build_ignore_set(&config)?;
        let analysis_result = run_full_analysis(
            &scan_result,
            &root_path,
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

        // "Passed" = assets with zero issues. `issue_count` counts issues, not
        // assets, and one asset can raise several, so count the DISTINCT asset
        // paths that have an issue.
        let pass_count = {
            let with_issues: std::collections::HashSet<&str> = analysis_result
                .issues
                .iter()
                .map(|i| i.asset_path.as_str())
                .collect();
            scan_result.total_count.saturating_sub(with_issues.len())
        };

        let warning_banner = warning_banner_html(&scan_result.warnings);
        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Tidycraft Report - {project_name}</title>
{REPORT_STYLE}
</head>
<body>
    <div class="container">
        <h1>Tidycraft Report</h1>
        <p class="meta">Project: {project_name} | Generated: {date}</p>
{warning_banner}
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
                <h3 style="margin-bottom: 1rem; color: var(--text-2);">By Type</h3>
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
                        let (title, message) = localized_issue_cells(issue, templates.as_ref());
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
                        r#"<tr><td colspan="4" style="text-align:center;color:var(--text-2);font-style:italic;">Showing first {} of {} issues — export to JSON for the complete list, or raise the limit in Settings → Export.</td></tr>"#,
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
                        r#"<tr><td colspan="4" style="text-align:center;color:var(--text-2);font-style:italic;">Showing first {} of {} assets — export to CSV or JSON for the complete list, or raise the limit in Settings → Export.</td></tr>"#,
                        asset_cap, total
                    ));
                }
                rows.join("\n")
            }
        );

        Ok(html)
    }
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
        // An empty `find` is a no-op, not `str::replace("")`, which would insert
        // the replacement between every character. The preview shares this, so the
        // no-op also zeroes the dialog's changed count.
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
            .split(['_', '-', ' '])
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

/// MS-DOS device names. Win32 still resolves them as devices whatever the
/// extension carried — `CON.png` is the console, not a file — so such a file
/// cannot be created on Windows at all.
const WINDOWS_RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The last gate before a rename touches the disk. The two Windows rules below
/// are enforced on every platform: a name minted on macOS lands in the repository
/// a Windows teammate checks out.
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
    app: AppHandle,
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

    let mut warnings = Vec::new();
    let result = commit_renames(&project_id, planned, "Batch rename", &mut warnings);
    for w in &warnings {
        warning::emit_project_warning(&app, &project_id, w);
    }
    result
}

/// Turn accumulated sidecar-carry failures into one warning, if there were any.
/// Every caller collects across a whole operation first: a batch of a hundred
/// renames on a locked project would otherwise emit a hundred warnings for what
/// is one problem.
fn push_sidecar_warning(
    failures: &mut warning::SampledFailures,
    warnings: &mut Vec<warning::ProjectWarning>,
) {
    if failures.is_empty() {
        return;
    }
    warnings.push(warning::ProjectWarning::SidecarNotCarried {
        affected: failures.count,
        sample: std::mem::take(&mut failures.sample),
        detail: failures.detail.take().unwrap_or_default(),
    });
    failures.count = 0;
}

/// Rename a heterogeneous batch — each file to its own new *file name* within its
/// current directory. Returns the successes as `(old_path, normalized new path)`.
/// Free of project-state side effects; `commit_renames` layers undo and tags on.
fn rename_batch_on_disk(
    planned: Vec<(String, String)>,
    // Out-param for the same reason `commit_renames` takes one: this half stays
    // free of project state and of the AppHandle, but a sidecar that failed to
    // follow its asset has to reach the command boundary that owns the emit.
    sidecar_failures: &mut warning::SampledFailures,
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

        // The target may `exists()`-resolve to the source file itself — a case-only
        // change or an NFC/NFD variant — so only a genuinely different occupant is
        // rejected. Identity is dev+inode (undo.rs), never the name.
        if new_path.exists() && !undo::paths_are_same_file(path_obj, &new_path) {
            errors.push(format!("Target already exists: {}", new_path.display()));
            error_count += 1;
            continue;
        }

        // Same guard for the engine sidecars, run BEFORE the primary rename: a
        // stray sidecar squatting on the destination name would otherwise strand
        // the asset's identity. See sidecar::rename_conflicts.
        let sidecar_conflicts = sidecar::rename_conflicts(path_obj, &new_path);
        if !sidecar_conflicts.is_empty() {
            errors.push(format!("{}: {}", name, sidecar_conflicts.join("; ")));
            error_count += 1;
            continue;
        }

        match std::fs::rename(&path, &new_path) {
            Ok(_) => {
                // Carry engine sidecars so renamed assets keep their identity (Unity GUID, Godot
                // UID) and import settings. The rename already happened and is not rolled back, so a
                // carry failure is reported, never raised — raising says a rename on disk did not happen.
                if let Err(e) = sidecar::carry_on_rename(path_obj, &new_path) {
                    eprintln!(
                        "[batch_rename] engine sidecar not carried for {}: {}",
                        path, e
                    );
                    sidecar_failures.record(Some(&path), &e);
                }
                success_count += 1;
                // Normalize to forward slashes so the undo record and the tag
                // binding key off the same string the next scan produces.
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

/// Files renamed per lock window. A file's disk rename and its tag migration must
/// not be separated by a lock release, and holding the lock across a whole batch
/// would freeze the project's other commands — so the batch is chunked.
const RENAME_LOCK_CHUNK: usize = 100;

/// Rename a heterogeneous batch on disk, migrating tag bindings as it goes, and —
/// if anything moved — record ONE undo batch. `label` names the undo entry; the
/// rename runs inside the project lock in `RENAME_LOCK_CHUNK` slices.
fn commit_renames(
    project_id: &str,
    planned: Vec<(String, String)>,
    label: &str,
    // Out-param rather than an AppHandle: the lock-window regression tests
    // call this directly and have no Tauri app to hand it. The command
    // boundary owns the emit.
    warnings: &mut Vec<warning::ProjectWarning>,
) -> BatchRenameResult {
    let total = planned.len();
    let mut all_done: Vec<(String, String)> = Vec::new();
    let mut result = BatchRenameResult {
        success_count: 0,
        error_count: 0,
        errors: Vec::new(),
    };

    // Accumulated across chunks so a batch reports one warning, not one per
    // hundred files.
    let mut sidecar_failures = warning::SampledFailures::default();

    for chunk in planned.chunks(RENAME_LOCK_CHUNK) {
        let outcome = project::with_mut(project_id, |state| {
            let (done, part) = rename_batch_on_disk(chunk.to_vec(), &mut sidecar_failures);

            // Tags follow the file across renames — same as move_assets /
            // rename_file. Paths are already normalized (scanner::path_to_string)
            // so the new key matches what the next scan produces.
            if !done.is_empty() {
                let tags = state.ensure_tags();
                for (original, new_path) in &done {
                    tags.rename_path(original, new_path);
                }
                // Logged AND surfaced: the files are already renamed, so this
                // must not fail the command, but a silent failure here means the
                // bindings only live in memory (watcher.rs reports the same way).
                if let Err(e) = state.save_tags() {
                    eprintln!("[batch_rename] failed to save tags after rename: {}", e);
                    warnings.push(warning::ProjectWarning::TagsNotSaved {
                        detail: e.to_string(),
                    });
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
                // Project not registered: renaming with no undo record and no tag
                // migration is worse than refusing, so report it.
                let untouched = total - result.success_count - result.error_count;
                result.error_count += untouched;
                result.errors.push(format!("Renames aborted: {}", e));
                push_sidecar_warning(&mut sidecar_failures, warnings);
                return result;
            }
        }
    }

    push_sidecar_warning(&mut sidecar_failures, warnings);

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

/// Compute compliant-name suggestions for every asset with an auto-fixable naming
/// violation, using the same `tidycraft.toml` the analysis ran with. Read-only —
/// nothing is renamed until `apply_naming_fixes`.
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
/// more than one file in the batch. Keyed case-insensitively so it also catches
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

/// Apply the renames the user accepted from the Fix-it dialog through the shared
/// batch engine, so validation, clobber guards, sidecar carrying, one undo batch
/// and tag migration all match Batch Rename.
#[tauri::command(async)]
fn apply_naming_fixes(
    app: AppHandle,
    project_id: String,
    fixes: Vec<NamingFix>,
) -> BatchRenameResult {
    let planned: Vec<(String, String)> = fixes.into_iter().map(|f| (f.path, f.new_name)).collect();
    let mut warnings = Vec::new();
    let result = commit_renames(&project_id, planned, "Fix naming", &mut warnings);
    for w in &warnings {
        warning::emit_project_warning(&app, &project_id, w);
    }
    result
}

// ============ Unreal Engine Commands ============

// ============ Godot Commands ============

// ============ File System Commands ============

/// Open the OS file manager focused on `path` (Finder reveal / Explorer
/// `/select,` / xdg-open parent). The per-OS dispatch lives here because
/// `tauri-plugin-shell::open` has no "select this file" mode.
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
        // Two explorer `/select,` quirks: the flag and the path must be a SINGLE
        // command-line argument, and it only follows backslash-separated paths —
        // so `path_to_string`'s normalization is undone here at the boundary.
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

/// Launch a file with the OS-default application for its extension, routed
/// through `tauri-plugin-opener` so Windows codepage, path quoting and `%`
/// expansion are handled by the platform shell helper.
#[tauri::command]
fn open_with_default_app(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(&path, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Open a URL in the default browser. The path-opening command above goes
/// through `open_path`, which is the wrong opener half for URLs — this one
/// exists for the macOS menu's Help links (GitHub readme / issues).
#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Write an export payload to a user-chosen destination. The frontend gets `path`
/// from the native save dialog, so the command only performs the write the
/// webview itself cannot.
#[tauri::command]
fn save_text_file(path: String, contents: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Empty destination path".to_string());
    }
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/// Open a file with a specific external application — `editor` is the absolute
/// path to a binary or .app bundle. Errors bubble up to the caller as a string
/// for inline display.
#[tauri::command]
fn open_in_editor(app: tauri::AppHandle, path: String, editor: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(&path, Some(editor.as_str()))
        .map_err(|e| e.to_string())
}

// ============ Texture resolution for 3D model loaders ============
// FBX/OBJ/DAE files often embed texture filenames with no directory part, or one
// valid only on the author's machine, which the asset protocol cannot resolve.

const TEXTURE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "tga", "bmp", "gif", "dds", "hdr", "exr", "tif", "tiff", "webp", "psd",
];

/// Subdirs to scan below the model's own directory.
const SIBLING_SUBDIRS: &[&str] = &[
    "",
    "Textures",
    "textures",
    "Texture",
    "texture",
    "Materials",
    "materials",
    "Material",
    "material",
    "Maps",
    "maps",
    "Tex",
    "tex",
    "Images",
    "images",
];

/// Subdirs to scan below the model's *parent* directory (for layouts where the
/// textures live as a sibling of the model folder, e.g. `Models/foo.fbx` +
/// `Textures/tex.png`).
const PARENT_SUBDIRS: &[&str] = &[
    "Textures",
    "textures",
    "Texture",
    "texture",
    "Materials",
    "materials",
    "Maps",
    "maps",
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
    app: AppHandle,
    project_id: String,
    paths: Vec<String>,
    target_dir: String,
) -> FileOpResult {
    let mut warnings = Vec::new();
    let result = commit_moves(&project_id, paths, target_dir, &mut warnings);
    for w in &warnings {
        warning::emit_project_warning(&app, &project_id, w);
    }
    result
}

/// The body of `move_assets`, minus the emit — same split as
/// `commit_renames`: the lock-window regression tests call this directly
/// and have no Tauri app to hand an `AppHandle`.
fn commit_moves(
    project_id: &str,
    paths: Vec<String>,
    target_dir: String,
    warnings: &mut Vec<warning::ProjectWarning>,
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

    // Accumulated across chunks, like commit_renames: one warning per operation.
    let mut sidecar_failures = warning::SampledFailures::default();

    // Chunked, and the moves happen INSIDE the project lock: a file that has left
    // its old path while its tag binding has not is exactly what the watcher's
    // orphan cleanup reaps. See `RENAME_LOCK_CHUNK`.
    for chunk in paths.chunks(RENAME_LOCK_CHUNK) {
        let outcome = project::with_mut(project_id, |state| {
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
                    // No-op: the source is already in the target directory. Checked
                    // by identity, not by string — a case variant, a symlinked
                    // folder or a `..` all name the same directory.
                    continue;
                }
                if dst.exists() {
                    failed.push(FileOpError {
                        path: path.clone(),
                        message: format!(
                            "Target already exists: {}",
                            scanner::path_to_string(&dst)
                        ),
                    });
                    continue;
                }

                // Sidecar pre-flight, same reasoning as rename_batch_on_disk:
                // refuse the move outright rather than move the asset away
                // from a .meta/.uid that can't follow it.
                let sidecar_conflicts = sidecar::rename_conflicts(src, &dst);
                if !sidecar_conflicts.is_empty() {
                    failed.push(FileOpError {
                        path: path.clone(),
                        message: sidecar_conflicts.join("; "),
                    });
                    continue;
                }

                match std::fs::rename(src, &dst) {
                    Ok(_) => {
                        // Carry engine sidecars so moved assets keep their identity
                        // (Unity GUID, Godot UID) and their import settings.
                        // Best-effort: no-op without a sidecar, logs on failure.
                        if let Err(e) = sidecar::carry_on_rename(src, &dst) {
                            eprintln!(
                                "[move_assets] engine sidecar not carried for {}: {}",
                                path, e
                            );
                            sidecar_failures.record(Some(path), &e);
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
            if !moved.is_empty() {
                let tags = state.ensure_tags();
                for s in &moved {
                    tags.rename_path(&s.original_path, &s.new_path);
                }
                // Logged AND surfaced: the move already succeeded so this
                // can't fail the command, but a silent failure leaves the
                // bindings in memory only (watcher.rs reports the same way).
                if let Err(e) = state.save_tags() {
                    eprintln!("[move_assets] failed to save tags after move: {}", e);
                    warnings.push(warning::ProjectWarning::TagsNotSaved {
                        detail: e.to_string(),
                    });
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
                push_sidecar_warning(&mut sidecar_failures, warnings);
                return FileOpResult { successes, errors };
            }
        }
    }

    push_sidecar_warning(&mut sidecar_failures, warnings);

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
        let _ = project::with_mut(project_id, |state| {
            state
                .undo_manager
                .record_batch(format!("Move {} file(s)", ops.len()), ops);
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

/// Send each path to the OS recycle bin. Per-path success and error are reported
/// separately so the interface can show partial results. No `project_id`: the
/// watcher picks up the remove events and updates the scan.
#[tauri::command(async)]
fn delete_assets(paths: Vec<String>) -> DeleteResult {
    let mut success_paths = Vec::new();
    let mut errors = Vec::new();

    for path in paths {
        match trash::delete(&path) {
            Ok(_) => {
                // Trash the engine sidecars too, so a delete doesn't strand them.
                // **Deliberately not reported as `SidecarNotCarried`** (the rename and move sites
                // do): a sidecar whose asset is gone breaks no reference, and it is the half worth keeping.
                if let Err(e) = sidecar::carry_on_delete(Path::new(&path)) {
                    eprintln!(
                        "[delete_assets] engine sidecar not carried for {}: {}",
                        path, e
                    );
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
fn rename_file(
    app: AppHandle,
    project_id: String,
    old_path: String,
    new_name: String,
) -> Result<String, String> {
    let mut warnings = Vec::new();
    let result = commit_single_rename(&project_id, old_path, new_name, &mut warnings);
    for w in &warnings {
        warning::emit_project_warning(&app, &project_id, w);
    }
    result
}

/// The body of `rename_file`, minus the emit — same split as
/// `commit_renames`: tests call this directly and have no Tauri app to hand
/// an `AppHandle`.
fn commit_single_rename(
    project_id: &str,
    old_path: String,
    new_name: String,
    warnings: &mut Vec<warning::ProjectWarning>,
) -> Result<String, String> {
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

    // The target may `exists()`-resolve to the source itself (case-only rename,
    // NFC/NFD variant); only a genuinely different occupant is a conflict, and
    // identity is by dev+inode, not by name.
    if new_path.exists() && !undo::paths_are_same_file(old_path_ref, &new_path) {
        return Err("A file with this name already exists".to_string());
    }

    // Engine sidecars get the same pre-flight as the file itself: refuse the
    // rename outright rather than move the asset away from a .meta/.uid a
    // stray destination sidecar would block (see sidecar::rename_conflicts).
    let sidecar_conflicts = sidecar::rename_conflicts(old_path_ref, &new_path);
    if !sidecar_conflicts.is_empty() {
        return Err(sidecar_conflicts.join("; "));
    }

    // Normalize to forward slashes so the returned path, the undo record, and
    // the tag binding all match what the scanner produces — `to_string_lossy`
    // would keep Windows backslashes (e.g. `C:/dir\new.png`).
    let new_path_str = scanner::path_to_string(&new_path);

    std::fs::rename(old_path_ref, &new_path).map_err(|e| e.to_string())?;

    // Carry engine sidecars so the renamed asset keeps its identity and its
    // references. Best-effort: a missing sidecar is a no-op. The file is already
    // renamed and stays that way, so a failure is reported rather than raised.
    if let Err(e) = sidecar::carry_on_rename(old_path_ref, &new_path) {
        eprintln!(
            "[rename_file] engine sidecar not carried for {}: {}",
            old_path, e
        );
        let mut failures = warning::SampledFailures::default();
        failures.record(Some(&old_path), &e);
        push_sidecar_warning(&mut failures, warnings);
    }

    let _ = project::with_mut(project_id, |state| {
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

        state.undo_manager.record_batch(
            format!("Rename {} to {}", old_name, new_name),
            vec![operation],
        );

        // Carry tags from the old path to the new one. Best-effort — this must
        // never fail a rename that already landed on disk — but logged, so a
        // persistently unwritable tags file stays diagnosable.
        state.ensure_tags().rename_path(&old_path, &new_path_str);
        if let Err(e) = state.save_tags() {
            eprintln!("[rename_file] failed to save tags after rename: {}", e);
            warnings.push(warning::ProjectWarning::TagsNotSaved {
                detail: e.to_string(),
            });
        }
        Ok(())
    });

    Ok(new_path_str)
}

// ============ Undo Commands ============

/// After an undo reverts renames or moves, carry each reverted file's tag binding
/// back (new_path → original_path). The pairs are exactly the ones the undo
/// actually reverted, so a failed file keeps its binding at `new_path`.
fn carry_tags_after_undo(
    state: &mut project::ProjectState,
    reverted_pairs: &[(String, String)],
) -> Option<warning::ProjectWarning> {
    if reverted_pairs.is_empty() {
        return None;
    }
    let tags = state.ensure_tags();
    for (original, new_path) in reverted_pairs {
        tags.rename_path(new_path, original);
    }
    // Worth surfacing even though the undo itself succeeded: memory now says
    // the bindings sit at the restored paths while disk still says the new
    // ones, and on the next launch the watcher reaps those as orphans.
    match state.save_tags() {
        Ok(()) => None,
        Err(e) => {
            eprintln!("[undo] failed to save tags after carrying them back: {}", e);
            Some(warning::ProjectWarning::TagsNotSaved {
                detail: e.to_string(),
            })
        }
    }
}

#[tauri::command]
fn get_undo_history(project_id: String) -> Vec<undo::HistoryEntry> {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.get_history())).unwrap_or_default()
}

#[tauri::command]
fn undo_last_operation(app: AppHandle, project_id: String) -> Result<undo::UndoResult, String> {
    let mut tags_warning: Option<warning::ProjectWarning> = None;
    let mut warnings: Vec<warning::ProjectWarning> = Vec::new();
    let mut sidecar_failures = warning::SampledFailures::default();
    let result = project::with_mut(&project_id, |state| {
        let result = state
            .undo_manager
            .undo_last(&mut sidecar_failures)
            .ok_or_else(|| "No operation to undo".to_string())?;
        // Carry tag bindings back for the files the undo actually reverted
        // (undo.rs has no access to TagsData). `reverted_pairs` excludes any
        // file whose undo failed, so their tags stay put at new_path.
        tags_warning = carry_tags_after_undo(state, &result.reverted_pairs);
        Ok(result)
    });

    // An undo that put files back but left their sidecars behind breaks the same
    // references a forward rename would have. Emitted even when the undo itself
    // returned `Err`: the failures recorded before that point already happened.
    push_sidecar_warning(&mut sidecar_failures, &mut warnings);
    for w in &warnings {
        warning::emit_project_warning(&app, &project_id, w);
    }
    if let Some(w) = &tags_warning {
        warning::emit_project_warning(&app, &project_id, w);
    }
    result
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
        state.mutate_tags_persisted(|tags| tags.create_tag(name, color))
    })
}

#[tauri::command]
fn update_tag(
    project_id: String,
    tag_id: String,
    name: Option<String>,
    color: Option<String>,
    // `Option<Option<String>>` carries three states: omitted leaves the
    // description alone, null clears it, a string sets it.
    description: Option<Option<String>>,
) -> Result<tags::Tag, String> {
    project::with_mut(&project_id, |state| {
        state
            .mutate_tags_persisted(|tags| tags.update_tag(&tag_id, name, color, description))?
            .ok_or_else(|| "Tag not found".to_string())
    })
}

#[tauri::command]
fn delete_tag(project_id: String, tag_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.mutate_tags_persisted(|tags| tags.delete_tag(&tag_id))
    })
}

#[tauri::command]
fn add_tag_to_asset(project_id: String, asset_path: String, tag_id: String) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.mutate_tags_persisted(|tags| tags.add_tag_to_asset(&asset_path, &tag_id))
    })
}

#[tauri::command]
fn remove_tag_from_asset(
    project_id: String,
    asset_path: String,
    tag_id: String,
) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.mutate_tags_persisted(|tags| tags.remove_tag_from_asset(&asset_path, &tag_id))
    })
}

#[tauri::command]
fn add_tag_to_assets(
    project_id: String,
    asset_paths: Vec<String>,
    tag_id: String,
) -> Result<(), String> {
    project::with_mut(&project_id, |state| {
        state.mutate_tags_persisted(|tags| {
            for path in asset_paths {
                tags.add_tag_to_asset(&path, &tag_id);
            }
        })
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

/// Toggle the webview inspector. Debug builds open it at startup, so this covers
/// getting it back after closing it. It compiles to a no-op in release, where the
/// `devtools` feature is off, but stays registered so the keybinding is uniform.
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

            // Debug builds auto-open the inspector; `open_devtools` only exists
            // under `debug_assertions` now that the `devtools` cargo feature is
            // off. `_app` keeps release builds free of unused warnings.
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
            check_project_paths,
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
            open_url,
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

    use crate::scanner::AssetType;

    /// A tag mutation whose save fails must not stay in memory: the frontend
    /// mirror shows nothing, so a later unrelated save would persist a phantom,
    /// and a retry of create_tag would mint a same-named duplicate.
    #[test]
    fn tag_crud_save_failure_does_not_leave_memory_ahead_of_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("appears_later");
        let id = "tag_rollback_test".to_string();
        project::register(id.clone(), root.to_string_lossy().replace('\\', "/"));

        // Root missing → the atomic write fails.
        assert!(create_tag(id.clone(), "Hero".into(), "#ff0000".into()).is_err());
        project::with_mut(&id, |st| {
            assert!(
                st.ensure_tags().tags.is_empty(),
                "failed save must not leave a phantom tag in memory"
            );
            Ok(())
        })
        .unwrap();

        // The folder comes back; the retry must produce exactly one tag.
        std::fs::create_dir_all(&root).unwrap();
        create_tag(id.clone(), "Hero".into(), "#ff0000".into()).unwrap();
        project::with_mut(&id, |st| {
            assert_eq!(
                st.ensure_tags().tags.len(),
                1,
                "a retry after a failed save must not duplicate the tag"
            );
            Ok(())
        })
        .unwrap();
        project::unregister(&id);
    }

    /// The staged learning run must survive a failed save: it carries the run's
    /// provenance (provider/model/depth), which a retry would otherwise lose.
    #[test]
    fn save_ai_rules_keeps_the_staged_run_until_the_write_lands() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("appears_later");
        let id = "ai_rules_pending_test".to_string();
        project::register(id.clone(), root.to_string_lossy().replace('\\', "/"));

        let staged = llm::rule_store::AiRulesDoc {
            last_learned: "2026-08-23T00:00:00Z".into(),
            prompt_version: 1,
            sampling_depth: 5,
            provider_used: "claude".into(),
            model_used: "claude-sonnet-5".into(),
            rules: Vec::new(),
        };
        project::with_mut(&id, |st| {
            st.pending_ai_rules = Some(staged);
            Ok(())
        })
        .unwrap();

        assert!(save_ai_rules(id.clone(), Vec::new()).is_err());
        project::with_ref(&id, |st| {
            assert!(
                st.pending_ai_rules.is_some(),
                "a failed save must keep the staged run for the retry"
            );
            Ok(())
        })
        .unwrap();

        std::fs::create_dir_all(&root).unwrap();
        save_ai_rules(id.clone(), Vec::new()).unwrap();
        let saved = llm::rule_store::AiRulesDoc::load(&root).unwrap().unwrap();
        assert_eq!(
            saved.provider_used, "claude",
            "the retried save still carries the staged run's provenance"
        );
        project::with_ref(&id, |st| {
            assert!(st.pending_ai_rules.is_none(), "consumed only on success");
            Ok(())
        })
        .unwrap();
        project::unregister(&id);
    }

    /// One warning per operation, not per file — and the accumulator has to come
    /// back empty, or the next chunk of the same batch reports the same failures
    /// again on top of its own.
    #[test]
    fn sidecar_failures_roll_up_into_one_warning_and_reset() {
        let mut failures = warning::SampledFailures::default();
        let mut warnings: Vec<warning::ProjectWarning> = Vec::new();

        // Nothing recorded: nothing said.
        push_sidecar_warning(&mut failures, &mut warnings);
        assert!(warnings.is_empty(), "a clean run must not warn");

        for i in 0..7 {
            failures.record(Some(&format!("Assets/tex_{i}.png")), "sidecar is locked");
        }
        push_sidecar_warning(&mut failures, &mut warnings);

        assert_eq!(warnings.len(), 1, "seven failures, one warning");
        match &warnings[0] {
            warning::ProjectWarning::SidecarNotCarried {
                affected,
                sample,
                detail,
            } => {
                assert_eq!(*affected, 7, "the count is the whole batch");
                assert_eq!(
                    sample.len(),
                    warning::SAMPLE_CAP,
                    "the sample is capped, the count is not"
                );
                assert_eq!(sample[0], "Assets/tex_0.png");
                assert_eq!(detail, "sidecar is locked");
            }
            other => panic!("wrong warning: {other:?}"),
        }

        // Drained: a second flush of the same accumulator says nothing.
        push_sidecar_warning(&mut failures, &mut warnings);
        assert_eq!(warnings.len(), 1, "the accumulator must not report twice");
    }

    /// A report that says "1,234 assets" over a scan that skipped a subtree is
    /// lying. A healthy scan gets no banner element at all, and cache trouble
    /// alone stays silent.
    #[test]
    fn report_banner_admits_an_incomplete_scan() {
        use warning::ScanWarning;
        assert_eq!(warning_banner_html(&[]), "");
        assert_eq!(
            warning_banner_html(&[ScanWarning::CacheNotSaved { detail: "x".into() }]),
            ""
        );

        let banner = warning_banner_html(&[
            ScanWarning::TreeWalkFailed {
                skipped: 3,
                sample: vec![],
                detail: "denied".into(),
            },
            ScanWarning::AssetUnreadable {
                affected: 2,
                sample: vec![],
                detail: "gone".into(),
            },
            ScanWarning::IgnoreRulesUnusable {
                detail: "bad".into(),
            },
        ]);
        assert!(banner.contains(r#"class="warn-banner""#));
        assert!(banner.contains("5 "), "3 skipped + 2 affected roll up");
        assert!(banner.contains("ignore rules"));

        assert!(
            REPORT_STYLE.contains(".warn-banner {"),
            "the stylesheet must style the banner it emits"
        );
        // Theme discipline: --warn is declared in light, dark and print.
        assert_eq!(REPORT_STYLE.matches("--warn:").count(), 3);
    }

    #[test]
    fn every_asset_type_has_a_report_badge_rule() {
        // The report derives each badge's class from the variant name at runtime,
        // so a variant with no matching CSS rule ships as an unstyled badge. The
        // exhaustive match makes a new variant a compile error here.
        fn badge_class(t: &AssetType) -> &'static str {
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
        }

        for t in [
            AssetType::Texture,
            AssetType::Model,
            AssetType::Audio,
            AssetType::Video,
            AssetType::Animation,
            AssetType::Material,
            AssetType::Prefab,
            AssetType::Scene,
            AssetType::Script,
            AssetType::Data,
            AssetType::Other,
        ] {
            let class = badge_class(&t);
            assert_eq!(
                format!("{t:?}").to_lowercase(),
                class,
                "the class the report emits for {t:?} is not the one declared here"
            );
            assert!(
                REPORT_STYLE.contains(&format!(".{class} {{")),
                "the report stylesheet has no rule for the {class} badge"
            );
            // Every badge colour must be theme-aware, i.e. declared in both the
            // default block and the dark override. Counting occurrences of the
            // custom property is enough: three (declare, dark, print).
            let var = format!("--c-{class}:");
            assert_eq!(
                REPORT_STYLE.matches(&var).count(),
                3,
                "{var} should be declared in the light, dark and print palettes"
            );
        }

        for severity in ["error", "warning", "info"] {
            assert!(
                REPORT_STYLE.contains(&format!(".severity-{severity} {{")),
                "the report stylesheet has no rule for severity {severity}"
            );
        }
    }

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

    /// Two Windows landmines, rejected on every platform: a name minted on macOS
    /// lands in the repository a Windows teammate has to check out.
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
            warnings: Vec::new(),
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

    /// A sprite atlas is a pure reference holder, so leaving `.spriteatlas` out of
    /// the reference-source set made every sprite only an atlas points at look
    /// unused. `unreadable_sources` is asserted too: the two gates run in series.
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

    /// Any spelling of the target directory that is not byte-identical to the
    /// source's parent must not report "Target already exists" about the very file
    /// being moved. The fixture uses `..` because case behaviour is not portable.
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
        let result = commit_moves(
            project_id,
            vec![scanner::path_to_string(&hero)],
            scanner::path_to_string(&alias),
            &mut Vec::new(),
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
        let (done, result) = rename_batch_on_disk(planned, &mut Default::default());

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
            (
                bad.to_string_lossy().to_string(),
                "sub/evil.png".to_string(),
            ),
        ];
        let (done, result) = rename_batch_on_disk(planned, &mut Default::default());

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
        let (done, result) = rename_batch_on_disk(planned, &mut Default::default());

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

    /// The tag-migration race, proved deterministically rather than by sleeping:
    /// hold the project lock, start the rename on another thread, and assert it
    /// cannot touch the disk while the lock is held.
    #[test]
    fn commit_renames_does_not_expose_renamed_files_before_tags_follow() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = scanner::path_to_string(dir.path());
        let project_id = "test_commit_renames_tag_race";
        project::register(project_id.to_string(), root);

        // Enough files that an unlocked batch would be plainly observable.
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
            std::thread::spawn(move || commit_renames(project_id, planned, "Race", &mut Vec::new()))
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

    /// The tags file lives at the project root, so a read-only root makes
    /// save_tags fail while renames inside a writable subdirectory still succeed.
    /// chmod is a no-op on Windows.
    #[cfg(unix)]
    #[test]
    fn commit_renames_reports_a_tags_save_failure_instead_of_swallowing_it() {
        use std::os::unix::fs::PermissionsExt;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let sub = dir.path().join("assets");
        std::fs::create_dir(&sub).unwrap();
        let src = sub.join("a.png");
        std::fs::write(&src, "x").unwrap();
        let src_key = scanner::path_to_string(&src);

        let project_id = "test_commit_renames_tags_save_failure";
        project::register(project_id.to_string(), scanner::path_to_string(dir.path()));
        project::with_mut(project_id, |state| {
            let tags = state.ensure_tags();
            let tag = tags.create_tag("warn".to_string(), "#fff".to_string());
            tags.add_tag_to_asset(&src_key, &tag.id);
            Ok(())
        })
        .unwrap();

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let mut warnings = Vec::new();
        let result = commit_renames(
            project_id,
            vec![(src_key, "b.png".to_string())],
            "Race",
            &mut warnings,
        );
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        project::unregister(project_id);

        assert_eq!(result.success_count, 1, "the rename itself must land");
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, warning::ProjectWarning::TagsNotSaved { .. })),
            "a failed tags save must come back as a warning, got {warnings:?}"
        );
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
            std::thread::spawn(move || commit_moves(project_id, paths, target_dir, &mut Vec::new()))
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
                    tags.get_asset_tags(&s.new_path)
                        .iter()
                        .any(|t| t.id == tag_id),
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

    /// A rename must carry the tag binding even when nothing has loaded tags in
    /// this session. The tag is written through a detached `TagsData` and read back
    /// from disk, since memory alone would pass on a migration never saved.
    #[test]
    fn rename_carries_tags_that_were_never_loaded_this_session() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = scanner::path_to_string(dir.path());
        let src = dir.path().join("old.png");
        std::fs::write(&src, "x").unwrap();
        let old_key = scanner::path_to_string(&src);

        let mut on_disk = tags::TagsData::default();
        let tag = on_disk.create_tag("never-loaded".to_string(), "#fff".to_string());
        on_disk.add_tag_to_asset(&old_key, &tag.id);
        on_disk.save(dir.path()).unwrap();

        let project_id = "test_rename_tags_never_loaded";
        project::register(project_id.to_string(), root);
        project::with_ref(project_id, |state| {
            assert!(
                state.tags_data.is_none(),
                "precondition: the rename must arrive with tags unloaded"
            );
            Ok(())
        })
        .unwrap();

        let new_key = commit_single_rename(
            project_id,
            old_key.clone(),
            "new.png".to_string(),
            &mut Vec::new(),
        )
        .unwrap();

        let reloaded = tags::TagsData::load(dir.path());
        assert!(
            reloaded
                .get_asset_tags(&new_key)
                .iter()
                .any(|t| t.id == tag.id),
            "tag did not follow {} → {}",
            old_key,
            new_key
        );
        assert!(
            reloaded.get_asset_tags(&old_key).is_empty(),
            "binding was left on the old path"
        );

        project::unregister(project_id);
    }

    /// A CSV cell whose text starts with `=`, `+`, `-` or `@` is a formula to
    /// Excel, LibreOffice and Sheets, and `=cmd|'/c calc'!A1` is the classic proof
    /// that it reaches the shell. Such file names are legal on disk.
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
            (
                "texture.max_size.title".to_string(),
                "贴图尺寸过大".to_string(),
            ),
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
        // The template is ours and carries no markup; the arg values come from file
        // names and can. Escaping the composed string once covers both.
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

    /// The end of the wire the panel actually reads: a real corrupt rules file on
    /// disk, through the real command, serialized the way Tauri serializes it.
    /// `rule_suggest`'s own tests cover the branch; this covers the plumbing.
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
        project::register(project_id.to_string(), scanner::path_to_string(dir.path()));
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
