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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetInfo, AssetType, DirectoryNode};
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn a_missing_config_means_defaults_and_a_present_one_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let defaults = load_rule_config(&root).expect("no file is fine");
        assert!(!defaults.texture.enabled);
        fs::write(
            dir.path().join("tidycraft.toml"),
            "[texture]\nenabled = true\nmax_size = 8\n",
        )
        .unwrap();
        let loaded = load_rule_config(&root).expect("a valid file loads");
        assert!(loaded.texture.enabled);
        assert_eq!(loaded.texture.max_size, 8);
    }

    /// A config that exists but cannot be read or parsed must fail the run, not
    /// silently analyze with defaults.
    #[test]
    fn an_unreadable_or_invalid_config_fails_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        fs::write(dir.path().join("tidycraft.toml"), "[texture\n").unwrap();
        let err = load_rule_config(&root).expect_err("broken toml");
        assert!(err.starts_with("Invalid config:"), "{err}");
        fs::remove_file(dir.path().join("tidycraft.toml")).unwrap();
        fs::create_dir(dir.path().join("tidycraft.toml")).unwrap();
        let err = load_rule_config(&root).expect_err("a directory cannot be read");
        assert!(err.starts_with("Failed to read tidycraft.toml:"), "{err}");
    }

    #[test]
    fn ignore_patterns_build_a_matching_set_or_fail_fast() {
        let mut config = RuleConfig::default();
        assert!(build_ignore_set(&config).unwrap().is_none());
        config.ignore.patterns = vec!["ThirdParty/**".to_string()];
        let set = build_ignore_set(&config)
            .unwrap()
            .expect("one pattern makes a set");
        assert!(set.is_match("ThirdParty/x/y.png"));
        assert!(!set.is_match("Assets/y.png"));
        config.ignore.patterns = vec!["[bad".to_string()];
        let err = build_ignore_set(&config).expect_err("a malformed glob");
        assert!(err.starts_with("Invalid ignore pattern '[bad'"), "{err}");
    }

    fn asset(path: &str) -> AssetInfo {
        AssetInfo {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            extension: "png".to_string(),
            asset_type: AssetType::Texture,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        }
    }

    /// `[ignore]` drops the matching assets from the report and nothing else.
    #[test]
    fn ignored_assets_are_dropped_and_the_rest_analyzed() {
        let scan = ScanResult {
            root_path: "/p".to_string(),
            directory_tree: DirectoryNode {
                name: "p".into(),
                path: "/p".into(),
                children: vec![],
                file_count: 2,
                total_size: 2,
            },
            assets: vec![
                asset("/p/ThirdParty/T_Bad Name.png"),
                asset("/p/Assets/T_Bad Name.png"),
            ],
            total_count: 2,
            total_size: 2,
            type_counts: HashMap::new(),
            project_type: None,
            warnings: vec![],
        };
        let mut config = RuleConfig::default();
        config.ignore.patterns = vec!["ThirdParty/**".to_string()];
        let set = build_ignore_set(&config).unwrap();
        let result = run_full_analysis(
            &scan,
            "/p",
            &config,
            set.as_ref(),
            &unity::PackageGuidIndex::default(),
        );
        let paths: Vec<&str> = result
            .issues
            .iter()
            .map(|i| i.asset_path.as_str())
            .collect();
        assert_eq!(paths, ["/p/Assets/T_Bad Name.png"]);
        assert_eq!(result.issues[0].rule_id, "naming.forbidden_char");
    }
}
