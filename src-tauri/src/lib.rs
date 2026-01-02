mod analyzer;
mod cache;
mod git;
mod godot;
mod scanner;
mod tags;
mod thumbnail;
mod undo;
mod unity;
mod unreal;

use analyzer::{AnalysisResult, Analyzer};
use analyzer::rules::RuleConfig;
use cache::ScanCache;
use git::{GitInfo, GitManager};
use parking_lot::Mutex;
use scanner::{IncrementalStats, ScanProgress, ScanResult, ScanState};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// Global scan state for cancellation
static SCAN_STATE: Mutex<Option<Arc<ScanState>>> = Mutex::new(None);

// Global cached scan result for analysis
static CACHED_SCAN: Mutex<Option<ScanResult>> = Mutex::new(None);

// Global Git manager
static GIT_MANAGER: Mutex<Option<GitManager>> = Mutex::new(None);

// Global undo manager
static UNDO_MANAGER: Mutex<undo::UndoManager> = Mutex::new(undo::UndoManager::new(50));

// Global tags data
static TAGS_DATA: Mutex<Option<(String, tags::TagsData)>> = Mutex::new(None);

#[tauri::command]
fn scan_project(path: String) -> Result<ScanResult, String> {
    // Simple synchronous scan without progress tracking
    let result = scanner::scan_directory_with_state(&path, None).map_err(|e| e.to_string())?;

    // Cache the result
    {
        let mut cache = CACHED_SCAN.lock();
        *cache = Some(result.clone());
    }

    Ok(result)
}

#[tauri::command]
async fn scan_project_async(app: AppHandle, path: String) -> Result<ScanResult, String> {
    // Create new scan state
    let state = Arc::new(ScanState::new());

    // Store state for cancellation
    {
        let mut global_state = SCAN_STATE.lock();
        *global_state = Some(state.clone());
    }

    let state_for_progress = state.clone();
    let app_for_progress = app.clone();

    // Spawn progress reporter thread
    let progress_handle = thread::spawn(move || {
        loop {
            let progress = state_for_progress.get_progress();
            let is_done = matches!(
                progress.phase,
                scanner::ScanPhase::Completed | scanner::ScanPhase::Cancelled
            );

            // Emit progress event
            let _ = app_for_progress.emit("scan-progress", &progress);

            if is_done {
                break;
            }

            thread::sleep(Duration::from_millis(100));
        }
    });

    // Run scan in blocking thread
    let state_for_scan = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        scanner::scan_directory_with_state(&path, Some(state_for_scan))
    })
    .await
    .map_err(|e| e.to_string())?;

    // Wait for progress reporter to finish
    let _ = progress_handle.join();

    // Clear global state
    {
        let mut global_state = SCAN_STATE.lock();
        *global_state = None;
    }

    let scan_result = result.map_err(|e| e.to_string())?;

    // Cache the result
    {
        let mut cache = CACHED_SCAN.lock();
        *cache = Some(scan_result.clone());
    }

    Ok(scan_result)
}

#[tauri::command]
fn cancel_scan() -> bool {
    let global_state = SCAN_STATE.lock();
    if let Some(state) = global_state.as_ref() {
        state.cancel();
        true
    } else {
        false
    }
}

#[tauri::command]
fn get_scan_progress() -> Option<ScanProgress> {
    let global_state = SCAN_STATE.lock();
    global_state.as_ref().map(|s| s.get_progress())
}

// ============ Incremental Scan Commands ============

#[derive(Serialize)]
pub struct IncrementalScanResult {
    pub result: ScanResult,
    pub stats: IncrementalStats,
}

#[tauri::command]
async fn scan_project_incremental(app: AppHandle, path: String) -> Result<IncrementalScanResult, String> {
    // Create new scan state
    let state = Arc::new(ScanState::new());

    // Store state for cancellation
    {
        let mut global_state = SCAN_STATE.lock();
        *global_state = Some(state.clone());
    }

    let state_for_progress = state.clone();
    let app_for_progress = app.clone();

    // Spawn progress reporter thread
    let progress_handle = thread::spawn(move || {
        loop {
            let progress = state_for_progress.get_progress();
            let is_done = matches!(
                progress.phase,
                scanner::ScanPhase::Completed | scanner::ScanPhase::Cancelled
            );

            // Emit progress event
            let _ = app_for_progress.emit("scan-progress", &progress);

            if is_done {
                break;
            }

            thread::sleep(Duration::from_millis(100));
        }
    });

    // Run incremental scan in blocking thread
    let state_for_scan = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        scanner::scan_directory_incremental(&path, Some(state_for_scan))
    })
    .await
    .map_err(|e| e.to_string())?;

    // Wait for progress reporter to finish
    let _ = progress_handle.join();

    // Clear global state
    {
        let mut global_state = SCAN_STATE.lock();
        *global_state = None;
    }

    let (scan_result, stats) = result.map_err(|e| e.to_string())?;

    // Cache the result
    {
        let mut cache = CACHED_SCAN.lock();
        *cache = Some(scan_result.clone());
    }

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
fn analyze_assets(config_toml: Option<String>) -> Result<AnalysisResult, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available. Please scan a project first.")?;

    // Parse config or use default
    let config = if let Some(toml_str) = config_toml {
        RuleConfig::from_toml(&toml_str).map_err(|e| format!("Invalid config: {}", e))?
    } else {
        RuleConfig::default()
    };

    let analyzer = Analyzer::with_config(&config);
    let mut result = analyzer.analyze(scan_result);

    // Also find duplicates
    let duplicates = analyzer.find_duplicates(scan_result);
    result.merge(duplicates);

    Ok(result)
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
fn get_git_info(path: String) -> GitInfo {
    let manager = GitManager::open(Path::new(&path));
    let info = manager.get_info();

    // Store the manager for later use
    {
        let mut global_manager = GIT_MANAGER.lock();
        *global_manager = Some(manager);
    }

    info
}

#[derive(Serialize)]
pub struct GitStatusMap {
    pub statuses: HashMap<String, String>,
}

#[tauri::command]
fn get_git_statuses() -> GitStatusMap {
    let mut global_manager = GIT_MANAGER.lock();

    let statuses = if let Some(manager) = global_manager.as_mut() {
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

    GitStatusMap { statuses }
}

#[tauri::command]
fn get_file_git_status(path: String) -> String {
    let global_manager = GIT_MANAGER.lock();

    if let Some(manager) = global_manager.as_ref() {
        let status = manager.get_file_status(Path::new(&path));
        format!("{:?}", status).to_lowercase()
    } else {
        "unknown".to_string()
    }
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
fn get_unity_dependencies() -> Result<DependencyGraph, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    // Only work with Unity projects
    if !matches!(scan_result.project_type, Some(scanner::ProjectType::Unity)) {
        return Err("Not a Unity project".to_string());
    }

    let mut nodes: Vec<DependencyNode> = Vec::new();
    let mut edges: Vec<DependencyEdge> = Vec::new();
    let mut guid_to_path: HashMap<String, String> = HashMap::new();

    // Build GUID to path mapping
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

    // Parse Unity files and extract references
    for asset in &scan_result.assets {
        let ext = asset.extension.to_lowercase();
        if ext == "prefab" || ext == "unity" || ext == "mat" {
            if let Some(unity_info) = unity::parse_unity_file(Path::new(&asset.path)) {
                if let Some(ref from_guid) = asset.unity_guid {
                    for reference in &unity_info.references {
                        // Only add edge if target exists in our project
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
}

#[tauri::command]
fn find_unused_assets() -> Result<Vec<String>, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    if !matches!(scan_result.project_type, Some(scanner::ProjectType::Unity)) {
        return Err("Not a Unity project".to_string());
    }

    let mut referenced_guids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_guids: HashMap<String, String> = HashMap::new();

    // Collect all GUIDs
    for asset in &scan_result.assets {
        if let Some(ref guid) = asset.unity_guid {
            all_guids.insert(guid.clone(), asset.path.clone());
        }
    }

    // Collect all referenced GUIDs from Unity files
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

    // Find assets that are never referenced
    let unused: Vec<String> = all_guids
        .iter()
        .filter(|(guid, _path)| !referenced_guids.contains(*guid))
        .map(|(_guid, path)| path.clone())
        .collect();

    Ok(unused)
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
fn get_project_stats() -> Result<ProjectStats, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    let mut type_distribution: HashMap<String, usize> = HashMap::new();
    let mut size_distribution: HashMap<String, usize> = HashMap::new();
    let mut extension_distribution: HashMap<String, usize> = HashMap::new();
    let mut directory_sizes: HashMap<String, u64> = HashMap::new();
    let mut all_files: Vec<FileInfo> = Vec::new();

    for asset in &scan_result.assets {
        // Type distribution
        let type_str = format!("{:?}", asset.asset_type).to_lowercase();
        *type_distribution.entry(type_str.clone()).or_insert(0) += 1;

        // Extension distribution
        *extension_distribution.entry(asset.extension.clone()).or_insert(0) += 1;

        // Size distribution (buckets)
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

        // Directory sizes
        if let Some(parent) = Path::new(&asset.path).parent() {
            let dir_str = parent.to_string_lossy().to_string();
            *directory_sizes.entry(dir_str).or_insert(0) += asset.size;
        }

        // Collect all files for sorting
        all_files.push(FileInfo {
            name: asset.name.clone(),
            path: asset.path.clone(),
            size: asset.size,
            asset_type: type_str,
        });
    }

    // Sort by size and take top 10
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
}

// ============ Export Commands ============

#[tauri::command]
fn export_to_json() -> Result<String, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    serde_json::to_string_pretty(scan_result).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_to_csv() -> Result<String, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    let mut csv = String::from("Name,Path,Type,Extension,Size,Width,Height\n");

    for asset in &scan_result.assets {
        let width = asset.metadata.as_ref().and_then(|m| m.width).map(|w| w.to_string()).unwrap_or_default();
        let height = asset.metadata.as_ref().and_then(|m| m.height).map(|h| h.to_string()).unwrap_or_default();

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
}

#[tauri::command]
fn export_issues_to_json() -> Result<String, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    let config = RuleConfig::default();
    let analyzer = Analyzer::with_config(&config);
    let mut result = analyzer.analyze(scan_result);
    let duplicates = analyzer.find_duplicates(scan_result);
    result.merge(duplicates);

    serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
}

#[tauri::command]
fn export_to_html() -> Result<String, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No scan result available")?;

    // Run analysis
    let config = RuleConfig::default();
    let analyzer = Analyzer::with_config(&config);
    let mut analysis_result = analyzer.analyze(scan_result);
    let duplicates = analyzer.find_duplicates(scan_result);
    analysis_result.merge(duplicates);

    // Calculate statistics
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut size_by_type: HashMap<String, u64> = HashMap::new();

    for asset in &scan_result.assets {
        let type_str = format!("{:?}", asset.asset_type);
        *type_counts.entry(type_str.clone()).or_insert(0) += 1;
        *size_by_type.entry(type_str).or_insert(0) += asset.size;
    }

    // Format file size helper
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

    // Generate HTML
    let html = format!(r#"<!DOCTYPE html>
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
            type_counts.iter().map(|(t, c)| {
                let pct = (*c as f64 / max_count * 100.0) as u32;
                format!(r#"<div><div class="bar" style="width: {}%"></div><div class="bar-label"><span>{}</span><span>{}</span></div></div>"#, pct, t, c)
            }).collect::<Vec<_>>().join("\n")
        },
        issue_rows = analysis_result.issues.iter().take(100).map(|issue| {
            let severity_class = match issue.severity {
                analyzer::Severity::Error => "severity-error",
                analyzer::Severity::Warning => "severity-warning",
                analyzer::Severity::Info => "severity-info",
            };
            let file_name = issue.asset_path.split('/').last().unwrap_or(&issue.asset_path);
            format!(
                r#"<tr><td class="{}">{:?}</td><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                severity_class,
                issue.severity,
                issue.rule_name,
                file_name,
                issue.message
            )
        }).collect::<Vec<_>>().join("\n"),
        asset_rows = scan_result.assets.iter().take(500).map(|asset| {
            let type_class = match asset.asset_type {
                scanner::AssetType::Texture => "texture",
                scanner::AssetType::Model => "model",
                scanner::AssetType::Audio => "audio",
                _ => "other",
            };
            let dimensions = asset.metadata.as_ref()
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
        }).collect::<Vec<_>>().join("\n")
    );

    Ok(html)
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
        RenameOperation::FindReplace { find, replace } => {
            name.replace(find, replace)
        }
        RenameOperation::AddPrefix { prefix } => {
            format!("{}{}", prefix, name)
        }
        RenameOperation::AddSuffix { suffix } => {
            // Insert suffix before extension
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
            // Remove suffix before extension
            if let Some(dot_pos) = name.rfind('.') {
                let base = &name[..dot_pos];
                let ext = &name[dot_pos..];
                let new_base = base.strip_suffix(suffix).unwrap_or(base);
                format!("{}{}", new_base, ext)
            } else {
                name.strip_suffix(suffix).unwrap_or(name).to_string()
            }
        }
        RenameOperation::ToLowercase => {
            name.to_lowercase()
        }
        RenameOperation::ToUppercase => {
            name.to_uppercase()
        }
        RenameOperation::ToTitleCase => {
            name.split(|c: char| c == '_' || c == '-' || c == ' ')
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => {
                            first.to_uppercase().collect::<String>() + chars.as_str()
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join("_")
        }
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
fn execute_batch_rename(paths: Vec<String>, operation: RenameOperation) -> BatchRenameResult {
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
            // No change needed
            continue;
        }

        let new_path = path_obj.with_file_name(&new_name);

        // Check if target already exists
        if new_path.exists() {
            errors.push(format!("Target already exists: {}", new_path.display()));
            error_count += 1;
            continue;
        }

        // Perform the rename
        match std::fs::rename(&path, &new_path) {
            Ok(_) => {
                success_count += 1;
                // Record for undo
                paths_to_record.push((path.clone(), new_path.to_string_lossy().to_string()));
            }
            Err(e) => {
                errors.push(format!("Failed to rename {}: {}", name, e));
                error_count += 1;
            }
        }
    }

    // Record the operation for undo if there were successful renames
    if success_count > 0 {
        let mut undo_manager = UNDO_MANAGER.lock();
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
            undo_manager.record_batch(
                format!("Batch rename: {} files", file_ops.len()),
                file_ops,
            );
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

    // Try to find .uproject file in the directory
    let uproject_path = unreal::find_uproject_file(root_path)
        .or_else(|| {
            // If path itself is a .uproject file
            if path.ends_with(".uproject") {
                Some(root_path.to_path_buf())
            } else {
                None
            }
        })
        .ok_or("No .uproject file found")?;

    unreal::parse_uproject(&uproject_path)
        .ok_or_else(|| "Failed to parse .uproject file".to_string())
}

// ============ Godot Commands ============

#[tauri::command]
fn get_godot_project_info(path: String) -> Result<godot::GodotProjectInfo, String> {
    let root_path = Path::new(&path);

    // Try to find project.godot file
    let project_path = if path.ends_with("project.godot") {
        root_path.to_path_buf()
    } else {
        root_path.join("project.godot")
    };

    if !project_path.exists() {
        return Err("No project.godot file found".to_string());
    }

    godot::parse_project_godot(&project_path)
        .ok_or_else(|| "Failed to parse project.godot file".to_string())
}

// ============ Undo Commands ============

#[tauri::command]
fn get_undo_history() -> Vec<undo::HistoryEntry> {
    let manager = UNDO_MANAGER.lock();
    manager.get_history()
}

#[tauri::command]
fn undo_last_operation() -> Result<undo::UndoResult, String> {
    let mut manager = UNDO_MANAGER.lock();
    manager.undo_last()
        .ok_or_else(|| "No operation to undo".to_string())
}

#[tauri::command]
fn undo_operation_by_id(id: String) -> Result<undo::UndoResult, String> {
    let mut manager = UNDO_MANAGER.lock();
    manager.undo_by_id(&id)
        .ok_or_else(|| format!("Operation '{}' not found or already undone", id))
}

#[tauri::command]
fn can_undo() -> bool {
    let manager = UNDO_MANAGER.lock();
    manager.can_undo()
}

#[tauri::command]
fn clear_undo_history() {
    let mut manager = UNDO_MANAGER.lock();
    manager.clear_history();
}

#[tauri::command]
fn get_undo_count() -> usize {
    let manager = UNDO_MANAGER.lock();
    manager.undoable_count()
}

// ============ Tags Commands ============

fn get_or_load_tags(project_path: &str) -> tags::TagsData {
    let mut global_tags = TAGS_DATA.lock();
    if let Some((path, data)) = global_tags.as_ref() {
        if path == project_path {
            return data.clone();
        }
    }
    let data = tags::TagsData::load(Path::new(project_path));
    *global_tags = Some((project_path.to_string(), data.clone()));
    data
}

fn save_tags(project_path: &str, data: &tags::TagsData) -> Result<(), String> {
    let mut global_tags = TAGS_DATA.lock();
    *global_tags = Some((project_path.to_string(), data.clone()));
    data.save(Path::new(project_path))
}

#[tauri::command]
fn get_all_tags() -> Result<Vec<tags::Tag>, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let data = get_or_load_tags(&scan_result.root_path);
    Ok(data.tags)
}

#[tauri::command]
fn create_tag(name: String, color: String) -> Result<tags::Tag, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    let tag = data.create_tag(name, color);
    save_tags(&project_path, &data)?;
    Ok(tag)
}

#[tauri::command]
fn update_tag(tag_id: String, name: Option<String>, color: Option<String>) -> Result<tags::Tag, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    let tag = data.update_tag(&tag_id, name, color).ok_or("Tag not found")?;
    save_tags(&project_path, &data)?;
    Ok(tag)
}

#[tauri::command]
fn delete_tag(tag_id: String) -> Result<(), String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    data.delete_tag(&tag_id);
    save_tags(&project_path, &data)?;
    Ok(())
}

#[tauri::command]
fn get_asset_tags(asset_path: String) -> Result<Vec<tags::Tag>, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let data = get_or_load_tags(&scan_result.root_path);
    Ok(data.get_asset_tags(&asset_path))
}

#[tauri::command]
fn add_tag_to_asset(asset_path: String, tag_id: String) -> Result<(), String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    data.add_tag_to_asset(&asset_path, &tag_id);
    save_tags(&project_path, &data)?;
    Ok(())
}

#[tauri::command]
fn remove_tag_from_asset(asset_path: String, tag_id: String) -> Result<(), String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    data.remove_tag_from_asset(&asset_path, &tag_id);
    save_tags(&project_path, &data)?;
    Ok(())
}

#[tauri::command]
fn add_tag_to_assets(asset_paths: Vec<String>, tag_id: String) -> Result<(), String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let project_path = scan_result.root_path.clone();
    drop(cache);

    let mut data = get_or_load_tags(&project_path);
    for path in asset_paths {
        data.add_tag_to_asset(&path, &tag_id);
    }
    save_tags(&project_path, &data)?;
    Ok(())
}

#[tauri::command]
fn get_all_asset_tags() -> Result<HashMap<String, Vec<tags::Tag>>, String> {
    let cache = CACHED_SCAN.lock();
    let scan_result = cache.as_ref().ok_or("No project loaded")?;
    let data = get_or_load_tags(&scan_result.root_path);

    let mut result: HashMap<String, Vec<tags::Tag>> = HashMap::new();
    for (path, _) in &data.asset_tags {
        let tags = data.get_asset_tags(path);
        if !tags.is_empty() {
            result.insert(path.clone(), tags);
        }
    }
    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            scan_project,
            scan_project_async,
            scan_project_incremental,
            cancel_scan,
            get_scan_progress,
            clear_scan_cache,
            get_thumbnail,
            analyze_assets,
            get_default_config,
            validate_config,
            get_git_info,
            get_git_statuses,
            get_file_git_status,
            parse_unity_file,
            get_unity_dependencies,
            find_unused_assets,
            get_project_stats,
            export_to_json,
            export_to_csv,
            export_issues_to_json,
            export_to_html,
            preview_batch_rename,
            execute_batch_rename,
            get_unreal_project_info,
            get_godot_project_info,
            get_undo_history,
            undo_last_operation,
            undo_operation_by_id,
            can_undo,
            clear_undo_history,
            get_undo_count,
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
