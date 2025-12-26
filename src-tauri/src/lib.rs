mod scanner;
mod thumbnail;

use parking_lot::Mutex;
use scanner::{ScanProgress, ScanResult, ScanState};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// Global scan state for cancellation
static SCAN_STATE: Mutex<Option<Arc<ScanState>>> = Mutex::new(None);

#[tauri::command]
fn scan_project(path: String) -> Result<ScanResult, String> {
    // Simple synchronous scan without progress tracking
    scanner::scan_directory_with_state(&path, None).map_err(|e| e.to_string())
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

    result.map_err(|e| e.to_string())
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
            get_thumbnail
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
