//! Session warning delivery. The warning types live in the core crate so the
//! scanner can construct them; this side owns the only Tauri-facing piece,
//! the per-project event channel.

use tauri::Emitter;

pub use tidycraft_core::warning::*;

/// Emit `w` on this project's warning channel. Failures to emit are ignored:
/// the warning channel must never fail the operation it reports on.
pub fn emit_project_warning(app: &tauri::AppHandle, project_id: &str, w: &ProjectWarning) {
    let _ = app.emit(&format!("project-warning-{project_id}"), w);
}
