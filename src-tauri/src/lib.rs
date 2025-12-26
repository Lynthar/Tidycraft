mod scanner;

use scanner::{scan_directory, ScanResult};

#[tauri::command]
fn scan_project(path: String) -> Result<ScanResult, String> {
    scan_directory(&path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![scan_project])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
