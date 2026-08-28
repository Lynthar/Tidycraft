//! Shared plumbing: root resolution, config loading, path relativization and
//! the envelope blocks every verb's JSON output carries.

use crate::CliError;
use std::path::{Path, PathBuf};
use tidycraft_core::analyzer::rules::RuleConfig;
use tidycraft_core::scanner::{self, ProjectType, ScanResult};
use tidycraft_core::warning::ScanWarning;

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum Format {
    Human,
    Json,
}

#[derive(serde::Serialize)]
pub struct ToolInfo {
    pub name: &'static str,
    pub version: &'static str,
}

pub fn tool_info() -> ToolInfo {
    ToolInfo {
        name: "tidycraft",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[derive(serde::Serialize)]
pub struct ProjectBlock {
    pub root: String,
    pub engine: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_source: Option<String>,
}

pub fn engine_name(project_type: Option<&ProjectType>) -> Option<&'static str> {
    project_type.map(|t| match t {
        ProjectType::Unity => "unity",
        ProjectType::Unreal => "unreal",
        ProjectType::Godot => "godot",
        ProjectType::Generic => "generic",
    })
}

/// Resolve the ROOT argument (default: cwd) to an absolute, forward-slashed
/// project root. A missing or non-directory root is a usage error (exit 2).
pub fn resolve_root(root: Option<PathBuf>) -> Result<(PathBuf, String), CliError> {
    let raw = root.unwrap_or_else(|| PathBuf::from("."));
    let abs = std::path::absolute(&raw)
        .map_err(|e| CliError::Config(format!("cannot resolve {}: {e}", raw.display())))?;
    if !abs.is_dir() {
        return Err(CliError::Config(format!(
            "project root is not a directory: {}",
            abs.display()
        )));
    }
    let root_str = scanner::path_to_string(&abs);
    Ok((abs, root_str))
}

/// Load the effective `RuleConfig` plus the raw TOML document (the CLI-only
/// `[check]` table lives outside `RuleConfig`). Returns the config, the parsed
/// document, and a display label for the source (`None` = built-in defaults).
///
/// # Errors
/// An explicit `--config` that is missing or unparseable, and a present but
/// invalid `tidycraft.toml`, are usage errors (exit 2). An unreadable default
/// file that exists is a runtime error (exit 3).
pub fn load_config(
    root_str: &str,
    config_flag: Option<&Path>,
) -> Result<(RuleConfig, Option<toml::Value>, Option<String>), CliError> {
    let (raw, source) = match config_flag {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| {
                CliError::Config(format!("cannot read config {}: {e}", path.display()))
            })?;
            (Some(raw), Some(scanner::path_to_string(path)))
        }
        None => {
            let path = Path::new(root_str).join("tidycraft.toml");
            match std::fs::read_to_string(&path) {
                Ok(raw) => (Some(raw), Some("tidycraft.toml".to_string())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
                Err(e) => {
                    return Err(CliError::Runtime(format!(
                        "cannot read {}: {e}",
                        path.display()
                    )));
                }
            }
        }
    };
    match raw {
        Some(raw) => {
            let config = RuleConfig::from_toml(&raw)
                .map_err(|e| CliError::Config(format!("invalid config: {e}")))?;
            let value: toml::Value = raw
                .parse()
                .map_err(|e| CliError::Config(format!("invalid config: {e}")))?;
            Ok((config, Some(value), source))
        }
        None => Ok((RuleConfig::default(), None, None)),
    }
}

/// `abs` relative to `root`, forward slashes. Paths not under the root come
/// back unchanged; the root itself comes back as `.`.
pub fn rel_path(root: &str, abs: &str) -> String {
    let root = root.trim_end_matches('/');
    if let Some(rest) = abs.strip_prefix(root) {
        if rest.is_empty() {
            return ".".to_string();
        }
        if let Some(rest) = rest.strip_prefix('/') {
            return rest.to_string();
        }
    }
    abs.to_string()
}

pub fn scan_project(root_str: &str) -> Result<ScanResult, CliError> {
    scanner::scan_directory_with_state(root_str, None, true)
        .map_err(|e| CliError::Runtime(format!("scan failed: {e}")))
}

/// One human-readable line per scan warning, so an incomplete scan is always
/// admitted rather than passing as a clean read.
pub fn describe_scan_warning(w: &ScanWarning) -> String {
    match w {
        ScanWarning::TreeWalkFailed {
            skipped,
            sample,
            detail,
        } => format!(
            "tree walk failed for {skipped} entr{}: {detail}{}",
            if *skipped == 1 { "y" } else { "ies" },
            sample_suffix(sample)
        ),
        ScanWarning::AssetUnreadable {
            affected,
            sample,
            detail,
        } => format!(
            "{affected} asset{} unreadable (size/mtime missing): {detail}{}",
            if *affected == 1 { "" } else { "s" },
            sample_suffix(sample)
        ),
        ScanWarning::IgnoreRulesUnusable { detail } => {
            format!("gitignore rules unusable, nothing was filtered: {detail}")
        }
        ScanWarning::CacheNotSaved { detail } => format!("scan cache not saved: {detail}"),
    }
}

fn sample_suffix(sample: &[String]) -> String {
    if sample.is_empty() {
        String::new()
    } else {
        format!(" — e.g. {}", sample.join(", "))
    }
}

pub fn use_color() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

pub fn paint(text: &str, code: &str, colored: bool) -> String {
    if colored {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_strips_only_whole_components() {
        assert_eq!(rel_path("C:/proj", "C:/proj/Assets/a.png"), "Assets/a.png");
        assert_eq!(rel_path("C:/proj", "C:/proj"), ".");
        // A sibling directory sharing the prefix must not be cut mid-name.
        assert_eq!(rel_path("C:/proj", "C:/proj2/a.png"), "C:/proj2/a.png");
        // A root that keeps its trailing slash still strips cleanly.
        assert_eq!(rel_path("C:/", "C:/a.png"), "a.png");
    }
}
