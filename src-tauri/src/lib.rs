mod analyzer;
mod cache;
mod git;
mod godot;
mod project;
mod scanner;
mod tags;
mod thumbnail;
mod undo;
mod unity;
mod unreal;

use analyzer::rules::RuleConfig;
use analyzer::{AnalysisResult, Analyzer};
use cache::ScanCache;
use git::{GitInfo, GitManager};
use scanner::{IncrementalStats, ScanProgress, ScanResult, ScanState};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
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

    let result = scanner::scan_directory_with_state(&path, None).map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |state| {
        state.cached_scan = Some(result.clone());
        Ok(())
    })?;

    Ok(result)
}

/// Spawn a background thread that emits `scan-progress-{project_id}` events
/// every 100ms until the scan reaches a terminal phase.
fn spawn_progress_reporter(
    app: AppHandle,
    project_id: String,
    state: Arc<ScanState>,
) -> thread::JoinHandle<()> {
    let event_name = format!("scan-progress-{}", project_id);
    thread::spawn(move || loop {
        let progress = state.get_progress();
        let is_done = matches!(
            progress.phase,
            scanner::ScanPhase::Completed | scanner::ScanPhase::Cancelled
        );

        let _ = app.emit(&event_name, &progress);

        if is_done {
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
    project::with_mut(&project_id, |s| {
        s.scan_state = Some(state.clone());
        Ok(())
    })?;

    let progress_handle = spawn_progress_reporter(app.clone(), project_id.clone(), state.clone());

    let state_for_scan = state.clone();
    let path_for_scan = path.clone();
    let result = tokio::task::spawn_blocking(move || {
        scanner::scan_directory_with_state(&path_for_scan, Some(state_for_scan))
    })
    .await
    .map_err(|e| e.to_string())?;

    let _ = progress_handle.join();

    let _ = project::with_mut(&project_id, |s| {
        s.scan_state = None;
        Ok(())
    });

    let scan_result = result.map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |s| {
        s.cached_scan = Some(scan_result.clone());
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
) -> Result<IncrementalScanResult, String> {
    project::register(project_id.clone(), path.clone());

    let state = Arc::new(ScanState::new());
    project::with_mut(&project_id, |s| {
        s.scan_state = Some(state.clone());
        Ok(())
    })?;

    let progress_handle = spawn_progress_reporter(app.clone(), project_id.clone(), state.clone());

    let state_for_scan = state.clone();
    let path_for_scan = path.clone();
    let result = tokio::task::spawn_blocking(move || {
        scanner::scan_directory_incremental(&path_for_scan, Some(state_for_scan))
    })
    .await
    .map_err(|e| e.to_string())?;

    let _ = progress_handle.join();

    let _ = project::with_mut(&project_id, |s| {
        s.scan_state = None;
        Ok(())
    });

    let (scan_result, stats) = result.map_err(|e| e.to_string())?;

    project::with_mut(&project_id, |s| {
        s.cached_scan = Some(scan_result.clone());
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

#[tauri::command]
async fn get_thumbnail(path: String, size: u32) -> Result<String, String> {
    thumbnail::get_thumbnail_base64(&path, size).map_err(|e| e.to_string())
}

// ============ Analysis Commands ============

#[tauri::command]
fn analyze_assets(project_id: String, config_toml: Option<String>) -> Result<AnalysisResult, String> {
    let config = if let Some(toml_str) = config_toml {
        RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
    } else {
        RuleConfig::default()
    };

    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;
        let analyzer = Analyzer::with_config(&config);
        let mut result = analyzer.analyze(scan_result);
        let duplicates = analyzer.find_duplicates(scan_result);
        result.merge(duplicates);
        Ok(result)
    })
}

#[tauri::command]
fn get_default_config() -> Result<String, String> {
    let config = RuleConfig::default();
    config.to_toml().map_err(|e| e.to_string())
}

#[tauri::command]
fn validate_config(config_toml: String) -> Result<bool, String> {
    match RuleConfig::from_toml(&config_toml) {
        Ok(_) => Ok(true),
        Err(e) => Err(format!("Invalid config: {}", e)),
    }
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
                    (
                        path.to_string_lossy().to_string(),
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

#[tauri::command]
fn get_file_git_status(project_id: String, path: String) -> String {
    project::with_ref(&project_id, |state| {
        let status = if let Some(manager) = state.git_manager.as_ref() {
            format!("{:?}", manager.get_file_status(Path::new(&path))).to_lowercase()
        } else {
            "unknown".to_string()
        };
        Ok(status)
    })
    .unwrap_or_else(|_| "unknown".to_string())
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

#[derive(Serialize)]
pub struct DependencyNode {
    pub path: String,
    pub name: String,
    pub guid: Option<String>,
    pub file_type: String,
}

#[derive(Serialize)]
pub struct DependencyEdge {
    pub from_guid: String,
    pub to_guid: String,
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
                    path: asset.path.clone(),
                    name: asset.name.clone(),
                    guid: Some(guid.clone()),
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
                                    from_guid: from_guid.clone(),
                                    to_guid: reference.guid.clone(),
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

        if !matches!(scan_result.project_type, Some(scanner::ProjectType::Unity)) {
            return Err("Not a Unity project".to_string());
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
                "\"{}\",\"{}\",{:?},{},{},{},{}\n",
                asset.name.replace('"', "\"\""),
                asset.path.replace('"', "\"\""),
                asset.asset_type,
                asset.extension,
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

        let config = RuleConfig::default();
        let analyzer = Analyzer::with_config(&config);
        let mut result = analyzer.analyze(scan_result);
        let duplicates = analyzer.find_duplicates(scan_result);
        result.merge(duplicates);

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    })
}

#[tauri::command]
fn export_to_html(project_id: String) -> Result<String, String> {
    project::with_ref(&project_id, |state| {
        let scan_result = state.require_scan()?;

        let config = RuleConfig::default();
        let analyzer = Analyzer::with_config(&config);
        let mut analysis_result = analyzer.analyze(scan_result);
        let duplicates = analyzer.find_duplicates(scan_result);
        analysis_result.merge(duplicates);

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
        .texture {{ background: #22c55e20; color: #22c55e; }}
        .model {{ background: #3b82f620; color: #3b82f6; }}
        .audio {{ background: #f59e0b20; color: #f59e0b; }}
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
            project_name = scan_result.root_path.split('/').last().unwrap_or("Project"),
            date = chrono::Local::now().format("%Y-%m-%d %H:%M"),
            total_assets = scan_result.total_count,
            total_size = format_size(scan_result.total_size),
            issue_count = analysis_result.issue_count,
            pass_count = scan_result.total_count.saturating_sub(analysis_result.issue_count),
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
            issue_rows = analysis_result
                .issues
                .iter()
                .take(100)
                .map(|issue| {
                    let severity_class = match issue.severity {
                        analyzer::Severity::Error => "severity-error",
                        analyzer::Severity::Warning => "severity-warning",
                        analyzer::Severity::Info => "severity-info",
                    };
                    let file_name = issue.asset_path.split('/').last().unwrap_or(&issue.asset_path);
                    format!(
                        r#"<tr><td class="{}">{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                        severity_class, issue.severity, issue.rule_name, file_name, issue.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            asset_rows = scan_result
                .assets
                .iter()
                .take(500)
                .map(|asset| {
                    let type_class = match asset.asset_type {
                        scanner::AssetType::Texture => "texture",
                        scanner::AssetType::Model => "model",
                        scanner::AssetType::Audio => "audio",
                        _ => "other",
                    };
                    let dimensions = asset
                        .metadata
                        .as_ref()
                        .and_then(|m| m.width.zip(m.height))
                        .map(|(w, h)| format!("{}x{}", w, h))
                        .unwrap_or_else(|| "-".to_string());
                    format!(
                        r#"<tr><td>{}</td><td><span class="type-badge {}">{:?}</span></td><td>{}</td><td>{}</td></tr>"#,
                        asset.name,
                        type_class,
                        asset.asset_type,
                        format_size(asset.size),
                        dimensions
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
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
                success_count += 1;
                paths_to_record.push((path.clone(), new_path.to_string_lossy().to_string()));
            }
            Err(e) => {
                errors.push(format!("Failed to rename {}: {}", name, e));
                error_count += 1;
            }
        }
    }

    if success_count > 0 {
        let file_ops: Vec<undo::FileOperation> = paths_to_record
            .into_iter()
            .map(|(original, new_path)| undo::FileOperation {
                operation_type: undo::OperationType::Rename,
                original_path: original,
                new_path: Some(new_path),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            })
            .collect();

        if !file_ops.is_empty() {
            let _ = project::with_mut(&project_id, |state| {
                state.undo_manager.record_batch(
                    format!("Batch rename: {} files", file_ops.len()),
                    file_ops,
                );
                Ok(())
            });
        }
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

#[tauri::command]
fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .args(["/select,", &path])
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

#[tauri::command]
fn open_with_default_app(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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

    let new_path_str = new_path.to_string_lossy().to_string();

    std::fs::rename(old_path_ref, &new_path).map_err(|e| e.to_string())?;

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
        Ok(())
    });

    Ok(new_path_str)
}

// ============ Undo Commands ============

#[tauri::command]
fn get_undo_history(project_id: String) -> Vec<undo::HistoryEntry> {
    project::with_ref(&project_id, |state| Ok(state.undo_manager.get_history())).unwrap_or_default()
}

#[tauri::command]
fn undo_last_operation(project_id: String) -> Result<undo::UndoResult, String> {
    project::with_mut(&project_id, |state| {
        state
            .undo_manager
            .undo_last()
            .ok_or_else(|| "No operation to undo".to_string())
    })
}

#[tauri::command]
fn undo_operation_by_id(project_id: String, id: String) -> Result<undo::UndoResult, String> {
    project::with_mut(&project_id, |state| {
        state
            .undo_manager
            .undo_by_id(&id)
            .ok_or_else(|| format!("Operation '{}' not found or already undone", id))
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
) -> Result<tags::Tag, String> {
    project::with_mut(&project_id, |state| {
        let tag = state
            .ensure_tags()
            .update_tag(&tag_id, name, color)
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
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
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
            get_thumbnail,
            // Analysis
            analyze_assets,
            get_default_config,
            validate_config,
            // Git
            get_git_info,
            get_git_statuses,
            get_file_git_status,
            // Unity
            parse_unity_file,
            get_unity_dependencies,
            find_unused_assets,
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
            reveal_in_finder,
            open_with_default_app,
            rename_file,
            // Tags
            get_all_tags,
            create_tag,
            update_tag,
            delete_tag,
            get_asset_tags,
            add_tag_to_asset,
            remove_tag_from_asset,
            add_tag_to_assets,
            get_all_asset_tags
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
