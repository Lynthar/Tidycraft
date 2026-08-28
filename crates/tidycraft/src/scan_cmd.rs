//! `tidycraft scan` — the asset inventory as JSON, the agent-facing view of
//! what the project contains.

use crate::util;
use crate::CliError;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;
use tidycraft_core::scanner::{AssetMetadata, AssetType};
use tidycraft_core::warning::ScanWarning;

#[derive(clap::Args)]
pub struct ScanArgs {
    /// Project root (default: current directory)
    pub root: Option<PathBuf>,
    /// Only include these asset types (comma-separated), e.g. `texture,model`
    #[arg(long, value_delimiter = ',', value_name = "TYPES")]
    pub types: Vec<String>,
    /// Maximum assets to return (0 = unlimited)
    #[arg(long, default_value_t = 0, value_name = "N")]
    pub max_assets: usize,
}

/// Keep in lockstep with `asset_type_name` below — the match is exhaustive, so
/// a new `AssetType` variant fails compilation here rather than silently
/// missing from `--types` validation.
const KNOWN_TYPES: &[&str] = &[
    "texture",
    "model",
    "audio",
    "video",
    "animation",
    "material",
    "prefab",
    "scene",
    "script",
    "data",
    "other",
];

fn asset_type_name(t: &AssetType) -> &'static str {
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

#[derive(serde::Serialize)]
struct AssetOut<'a> {
    path: String,
    name: &'a str,
    #[serde(rename = "type")]
    asset_type: &'a AssetType,
    size: u64,
    modified: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<&'a AssetMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unity_guid: Option<&'a String>,
}

#[derive(serde::Serialize)]
struct Summary {
    assets_total: usize,
    matched: usize,
    returned: usize,
    truncated: bool,
    total_size: u64,
    scan_warnings: usize,
    duration_ms: u64,
}

#[derive(serde::Serialize)]
struct Report<'a> {
    schema_version: u32,
    tool: util::ToolInfo,
    project: util::ProjectBlock,
    summary: Summary,
    type_counts: &'a HashMap<String, usize>,
    scan_warnings: &'a [ScanWarning],
    assets: Vec<AssetOut<'a>>,
}

pub fn run(args: ScanArgs) -> Result<ExitCode, CliError> {
    let started = Instant::now();
    let (_root, root_str) = util::resolve_root(args.root)?;

    let requested: Option<HashSet<String>> = if args.types.is_empty() {
        None
    } else {
        let set: HashSet<String> = args
            .types
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        for t in &set {
            if !KNOWN_TYPES.contains(&t.as_str()) {
                return Err(CliError::Config(format!(
                    "unknown asset type `{t}` — known: {}",
                    KNOWN_TYPES.join(", ")
                )));
            }
        }
        Some(set)
    };

    let scan = util::scan_project(&root_str)?;
    let duration_ms = started.elapsed().as_millis() as u64;

    let mut assets: Vec<AssetOut> = scan
        .assets
        .iter()
        .filter(|a| {
            requested
                .as_ref()
                .is_none_or(|set| set.contains(asset_type_name(&a.asset_type)))
        })
        .map(|a| AssetOut {
            path: util::rel_path(&root_str, &a.path),
            name: &a.name,
            asset_type: &a.asset_type,
            size: a.size,
            modified: a.modified,
            metadata: a.metadata.as_ref(),
            unity_guid: a.unity_guid.as_ref(),
        })
        .collect();
    assets.sort_by(|a, b| a.path.cmp(&b.path));

    let matched = assets.len();
    let truncated = args.max_assets > 0 && matched > args.max_assets;
    if truncated {
        assets.truncate(args.max_assets);
    }

    let report = Report {
        schema_version: 1,
        tool: util::tool_info(),
        project: util::ProjectBlock {
            root: root_str.clone(),
            engine: util::engine_name(scan.project_type.as_ref()),
            config_source: None,
        },
        summary: Summary {
            assets_total: scan.total_count,
            matched,
            returned: assets.len(),
            truncated,
            total_size: scan.total_size,
            scan_warnings: scan.warnings.len(),
            duration_ms,
        },
        type_counts: &scan.type_counts,
        scan_warnings: &scan.warnings,
        assets,
    };
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| CliError::Runtime(format!("serialize report: {e}")))?;
    println!("{json}");
    Ok(ExitCode::SUCCESS)
}
