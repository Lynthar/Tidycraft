mod analyzer;
mod scanner;
mod thumbnail;

use analyzer::{AnalysisResult, Analyzer};
use analyzer::rules::RuleConfig;
use parking_lot::Mutex;
use scanner::{ScanProgress, ScanResult, ScanState};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// Global scan state for cancellation
static SCAN_STATE: Mutex<Option<Arc<ScanState>>> = Mutex::new(None);

// Global cached scan result for analysis
static CACHED_SCAN: Mutex<Option<ScanResult>> = Mutex::new(None);

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
            validate_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
