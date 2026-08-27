//! Engine-agnostic core: project scanning, asset analysis, engine parsers and
//! the persistence primitives they share. No Tauri types live here — the
//! desktop app and any headless consumer add their own delivery layer.

pub mod analyzer;
pub mod cache;
pub mod fs_atomic;
#[cfg(feature = "git")]
pub mod git;
pub mod godot;
#[cfg(feature = "llm")]
pub mod llm;
pub mod scanner;
pub mod sidecar;
pub mod unity;
pub mod unreal;
pub mod warning;
