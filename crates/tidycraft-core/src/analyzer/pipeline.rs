//! The one analysis pipeline every consumer shares: config loading, the
//! `[ignore]` filter, then each analyzer phase in a fixed order.

use crate::analyzer::rules::RuleConfig;
use crate::analyzer::{AnalysisResult, Analyzer};
use crate::scanner::{self, ScanResult};
use crate::unity;
use std::path::Path;

/// Load the project's `RuleConfig` from `<root>/tidycraft.toml`. Absent file →
/// defaults; present but unreadable or unparseable → `Err`, matching how the
/// Issues view fails via `analyze_assets`.
pub fn load_rule_config(root_path: &str) -> Result<RuleConfig, String> {
    let toml_path = Path::new(root_path).join("tidycraft.toml");
    match std::fs::read_to_string(&toml_path) {
        Ok(content) => {
            RuleConfig::from_toml(&content).map_err(|e| format!("Invalid config: {}", e))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RuleConfig::default()),
        Err(e) => Err(format!("Failed to read tidycraft.toml: {}", e)),
    }
}

/// Build a `GlobSet` from `[ignore].patterns`, or `None` when the list is
/// empty. A malformed pattern surfaces as an `Err`; callers build this
/// before taking the project lock so the error short-circuits early.
pub fn build_ignore_set(config: &RuleConfig) -> Result<Option<globset::GlobSet>, String> {
    if config.ignore.patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in &config.ignore.patterns {
        let glob = globset::Glob::new(pattern)
            .map_err(|e| format!("Invalid ignore pattern '{}': {}", pattern, e))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| format!("Failed to build ignore set: {}", e))
}

/// The single source of truth for the analysis pipeline: apply the
/// `[ignore].patterns` filter, then run every analyzer phase. The app's
/// `analyze_assets` and both report exporters route through this for one
/// issue set per config.
pub fn run_full_analysis(
    scan_result: &ScanResult,
    root_path: &str,
    config: &RuleConfig,
    ignore_set: Option<&globset::GlobSet>,
    package_index: &unity::PackageGuidIndex,
) -> AnalysisResult {
    // Only clone the scan when there are patterns to apply; most projects
    // have none and analyze the cached scan reference in place.
    let owned_filtered: Option<ScanResult> = ignore_set.map(|set| {
        let root = Path::new(root_path);
        let kept: Vec<scanner::AssetInfo> = scan_result
            .assets
            .iter()
            .filter(|a| {
                let path = Path::new(&a.path);
                let rel = path.strip_prefix(root).unwrap_or(path);
                !set.is_match(rel)
            })
            .cloned()
            .collect();
        ScanResult {
            root_path: scan_result.root_path.clone(),
            directory_tree: scan_result.directory_tree.clone(),
            assets: kept,
            total_count: scan_result.total_count,
            total_size: scan_result.total_size,
            type_counts: scan_result.type_counts.clone(),
            project_type: scan_result.project_type.clone(),
            warnings: scan_result.warnings.clone(),
        }
    });
    let scan_to_analyze: &ScanResult = owned_filtered.as_ref().unwrap_or(scan_result);

    let analyzer = Analyzer::with_config(config);
    let mut result = analyzer.analyze(scan_to_analyze);
    let duplicates = analyzer.find_duplicates(scan_to_analyze);
    result.merge(duplicates);
    // Existence comes from the UNFILTERED scan: `[ignore]` limits what is
    // reported, not what the project contains. The other three cross-asset rules
    // keep the filtered view — see docs/analyzer-rules.md.
    let missing = analyzer.find_missing_references(scan_to_analyze, scan_result, package_index);
    result.merge(missing);
    let pbr = analyzer.find_pbr_set_issues(scan_to_analyze, &config.pbr_set);
    result.merge(pbr);
    let dcc = analyzer.find_dcc_source_issues(scan_to_analyze, &config.dcc_source);
    result.merge(dcc);
    result
}
