mod analyzer;
mod git;
mod scanner;
mod thumbnail;

use analyzer::{AnalysisResult, Analyzer};
use analyzer::rules::RuleConfig;
use git::{GitInfo, GitManager};
use parking_lot::Mutex;
use scanner::{ScanProgress, ScanResult, ScanState};
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            scan_project,
            scan_project_async,
            cancel_scan,
            get_scan_progress,
            get_thumbnail,
            analyze_assets,
            get_default_config,
            validate_config,
            get_git_info,
            get_git_statuses,
            get_file_git_status,
            get_project_stats,
            export_to_json,
            export_to_csv,
            export_issues_to_json
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
