pub mod pipeline;
#[cfg(feature = "llm")]
pub mod rule_suggest;
pub mod rules;
pub mod tag_suggest;

use crate::scanner::{AssetInfo, ScanResult};
use rules::{Rule, RuleConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub rule_id: String,
    pub rule_name: String,
    pub severity: Severity,
    pub message: String,
    pub asset_path: String,
    pub suggestion: Option<String>,
    pub auto_fixable: bool,
    /// Every member of the same finding, root-relative and sorted, with the kept
    /// "original" first. Only the `duplicate` rule fills this; the frontend uses
    /// it to collapse per-file duplicate issues into a single group card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_paths: Option<Vec<String>>,
    /// Placeholder values for the localized rendering of this issue's message and
    /// suggestion, keyed by the placeholder name used in the locale templates.
    /// The English prose in `message` / `suggestion` stays authoritative.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub args: HashMap<String, String>,
}

/// Build an `Issue.args` map from literal pairs, so each construction site shows
/// which placeholders the rule promises to fill rather than a `HashMap` literal
/// with `.to_string()` on both halves of every pair.
pub fn issue_args<const N: usize>(pairs: [(&str, String); N]) -> HashMap<String, String> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub issues: Vec<Issue>,
    pub issue_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub by_rule: HashMap<String, usize>,
}

impl AnalysisResult {
    pub fn new() -> Self {
        Self {
            issues: Vec::new(),
            issue_count: 0,
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            by_rule: HashMap::new(),
        }
    }

    pub fn add_issue(&mut self, issue: Issue) {
        match issue.severity {
            Severity::Error => self.error_count += 1,
            Severity::Warning => self.warning_count += 1,
            Severity::Info => self.info_count += 1,
        }

        *self.by_rule.entry(issue.rule_id.clone()).or_insert(0) += 1;
        self.issue_count += 1;
        self.issues.push(issue);
    }

    pub fn merge(&mut self, other: AnalysisResult) {
        for issue in other.issues {
            self.add_issue(issue);
        }
    }
}

impl Default for AnalysisResult {
    fn default() -> Self {
        Self::new()
    }
}

/// The main analyzer that runs all enabled rules
pub struct Analyzer {
    rules: Vec<Box<dyn Rule>>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Create analyzer with default rules based on config
    pub fn with_config(config: &RuleConfig) -> Self {
        let mut analyzer = Self::new();

        // Add naming rules
        if config.naming.enabled {
            analyzer.add_rule(Box::new(rules::naming::NamingRule::new(
                config.naming.clone(),
            )));
        }

        // `texture.color_space` is gated independently of this section's flag.
        if config.texture.enabled {
            analyzer.add_rule(Box::new(rules::texture::TextureRule::new(
                config.texture.clone(),
            )));
        }
        if config.texture.color_space.enabled {
            analyzer.add_rule(Box::new(rules::texture_colorspace::TextureColorSpaceRule));
        }

        // Add model rules
        if config.model.enabled {
            analyzer.add_rule(Box::new(rules::model::ModelRule::new(config.model.clone())));
        }

        // Add audio rules
        if config.audio.enabled {
            analyzer.add_rule(Box::new(rules::audio::AudioRule::new(config.audio.clone())));
        }

        analyzer
    }

    pub fn add_rule(&mut self, rule: Box<dyn Rule>) {
        self.rules.push(rule);
    }

    /// Analyze a single asset
    pub fn analyze_asset(&self, asset: &AssetInfo) -> Vec<Issue> {
        let mut issues = Vec::new();

        for rule in &self.rules {
            if rule.applies_to(asset) {
                if let Some(issue) = rule.check(asset) {
                    issues.push(issue);
                }
            }
        }

        issues
    }

    /// Analyze all assets in a scan result
    pub fn analyze(&self, scan_result: &ScanResult) -> AnalysisResult {
        let mut result = AnalysisResult::new();

        for asset in &scan_result.assets {
            for issue in self.analyze_asset(asset) {
                result.add_issue(issue);
            }
        }

        result
    }

    /// Check for duplicate files across all assets
    pub fn find_duplicates(&self, scan_result: &ScanResult) -> AnalysisResult {
        rules::duplicate::find_duplicates(&scan_result.assets, &scan_result.root_path)
    }

    /// Check for Unity GUID references that resolve to no asset in the project.
    /// No-op for non-Unity projects. `scan_result` is the analysis scope
    /// (post-`[ignore]`); `full_scan` is what decides which GUIDs exist.
    pub fn find_missing_references(
        &self,
        scan_result: &ScanResult,
        full_scan: &ScanResult,
        package_index: &crate::unity::PackageGuidIndex,
    ) -> AnalysisResult {
        rules::missing_reference::find_missing_references(
            &scan_result.assets,
            &full_scan.assets,
            &scan_result.project_type,
            package_index,
        )
    }

    /// Check for incomplete PBR material sets — a directory with a BaseColor
    /// texture but missing expected siblings. Cross-asset; takes the live config.
    pub fn find_pbr_set_issues(
        &self,
        scan_result: &ScanResult,
        config: &rules::pbr_set::PbrSetConfig,
    ) -> AnalysisResult {
        rules::pbr_set::find_pbr_set_issues(&scan_result.assets, config)
    }

    /// Check for DCC source files whose runtime exports are older than the source.
    /// Cross-asset; takes the live config.
    pub fn find_dcc_source_issues(
        &self,
        scan_result: &ScanResult,
        config: &rules::dcc_source::DccSourceConfig,
    ) -> AnalysisResult {
        rules::dcc_source::find_dcc_source_issues(&scan_result.assets, config)
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, AssetType};
    use std::collections::{BTreeMap, BTreeSet};

    /// Which placeholders each rule promises to fill. Read by the locale template
    /// gates, which cannot look at the rules themselves, and pinned to observed
    /// behavior by `declared_rule_args_match_what_the_rules_actually_emit`.
    const RULE_ARGS: &[(&str, &[&str])] = &[
        ("texture.file_size", &["size", "max"]),
        (
            "texture.pot",
            &["width", "height", "pot_width", "pot_height"],
        ),
        ("texture.max_size", &["width", "height", "max"]),
        ("texture.min_size", &["width", "height", "min"]),
        ("texture.non_square", &["width", "height"]),
        ("texture.no_mipmaps", &["width", "height"]),
        ("naming.length", &["char_count", "max"]),
        ("naming.forbidden_char", &["char"]),
        ("naming.chinese", &[]),
        ("naming.prefix", &["prefix", "name"]),
        ("naming.case", &["style"]),
        ("model.vertices", &["vertex_count", "max"]),
        ("model.faces", &["face_count", "max"]),
        ("model.materials", &["material_count", "max"]),
        ("audio.sample_rate", &["rate", "allowed", "preferred"]),
        ("audio.sfx_duration", &["duration", "max"]),
        ("audio.stereo_sfx", &[]),
        ("audio.file_size", &["size", "max"]),
        ("texture.color_space", &["suffix"]),
        (
            "duplicate",
            &["file_count", "original", "original_path", "other_count"],
        ),
        ("missing_reference", &["guid"]),
        ("pbr_set.incomplete", &["set", "channels"]),
        (
            "dcc_source.outdated_export",
            &["source", "export", "dcc", "age_value", "age_unit"],
        ),
    ];

    /// Run every rule against a fixture built to trigger it, and collect the
    /// arg keys each one actually emitted. The completeness check that every
    /// rule is represented lands last.
    fn harvest() -> BTreeMap<String, BTreeSet<String>> {
        let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut record = |issue: &Issue| {
            found
                .entry(issue.rule_id.clone())
                .or_default()
                .extend(issue.args.keys().cloned());
        };

        for issue in harvest_texture() {
            record(&issue);
        }
        for issue in harvest_naming() {
            record(&issue);
        }
        for issue in harvest_model() {
            record(&issue);
        }
        for issue in harvest_audio() {
            record(&issue);
        }
        for issue in harvest_colorspace() {
            record(&issue);
        }
        for issue in harvest_duplicate() {
            record(&issue);
        }
        for issue in harvest_missing_reference() {
            record(&issue);
        }
        for issue in harvest_pbr_set() {
            record(&issue);
        }
        for issue in harvest_dcc_source() {
            record(&issue);
        }

        found
    }

    fn texture_asset(name: &str, size: u64, meta: crate::scanner::AssetMetadata) -> AssetInfo {
        AssetInfo {
            path: format!("/p/{name}"),
            name: name.to_string(),
            extension: "png".to_string(),
            asset_type: crate::scanner::AssetType::Texture,
            size,
            modified: 0,
            metadata: Some(meta),
            unity_guid: None,
        }
    }

    fn sized(width: u32, height: u32) -> crate::scanner::AssetMetadata {
        crate::scanner::AssetMetadata {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }
    }

    fn harvest_texture() -> Vec<Issue> {
        use rules::texture::{TextureConfig, TextureRule};
        let cfg = TextureConfig {
            require_pot: true,
            warn_non_square: true,
            max_size: 2048,
            min_size: 32,
            max_file_size: 10 * 1024 * 1024,
            ..TextureConfig::default()
        };
        let rule = TextureRule::new(cfg);
        // One fixture per branch, in the rule's own precedence order: the
        // dimension checks short-circuit, so a fixture meant for a later
        // branch must pass every earlier one.
        let mut meta_no_mips = sized(512, 512);
        meta_no_mips.mipmap_count = Some(1);
        [
            texture_asset("not-pot.png", 1024, sized(100, 100)),
            texture_asset("too-big.png", 1024, sized(8192, 8192)),
            texture_asset("too-small.png", 1024, sized(8, 8)),
            texture_asset("oblong.png", 1024, sized(512, 256)),
            texture_asset("heavy.png", 11 * 1024 * 1024, sized(512, 512)),
            texture_asset("no-mips.dds", 1024, meta_no_mips),
        ]
        .iter()
        .filter_map(|a| rule.check(a))
        .collect()
    }

    fn harvest_naming() -> Vec<Issue> {
        use rules::naming::{NamingConfig, NamingRule};
        // Each fixture isolates one branch: the rule returns at the first violation
        // it finds (length → forbidden → chinese → prefix → case), so a name meant
        // for a later check must be clean for every earlier one.
        let long_name = format!("{}.png", "a".repeat(80));
        let cases: Vec<(NamingConfig, String)> = vec![
            (
                NamingConfig {
                    enabled: true,
                    max_length: 32,
                    ..NamingConfig::default()
                },
                long_name,
            ),
            (
                NamingConfig {
                    enabled: true,
                    ..NamingConfig::default()
                },
                "he<ro.png".to_string(),
            ),
            (
                NamingConfig {
                    enabled: true,
                    forbid_chinese: true,
                    ..NamingConfig::default()
                },
                "英雄.png".to_string(),
            ),
            (
                NamingConfig {
                    enabled: true,
                    texture_prefix: Some("T_".to_string()),
                    ..NamingConfig::default()
                },
                "rock.png".to_string(),
            ),
            (
                NamingConfig {
                    enabled: true,
                    case_style: "PascalCase".to_string(),
                    ..NamingConfig::default()
                },
                "rock_wall.png".to_string(),
            ),
        ];
        cases
            .into_iter()
            .filter_map(|(cfg, name)| {
                let rule = NamingRule::new(cfg);
                rule.check(&texture_asset(
                    &name,
                    1024,
                    crate::scanner::AssetMetadata::default(),
                ))
            })
            .collect()
    }

    fn harvest_model() -> Vec<Issue> {
        use rules::model::{ModelConfig, ModelRule};
        // `enabled` is inert here: no `Rule::check` impl reads it, and this harvest
        // calls `check()` directly. Set true anyway so the fixture reads as a config
        // a user could actually have written.
        let cfg = ModelConfig {
            enabled: true,
            max_vertices: 10_000,
            max_faces: 10_000,
            max_materials: 4,
        };
        let rule = ModelRule::new(cfg);
        let mk = |vertices, faces, materials| {
            let meta = crate::scanner::AssetMetadata {
                vertex_count: Some(vertices),
                face_count: Some(faces),
                material_count: Some(materials),
                ..Default::default()
            };
            AssetInfo {
                path: "/p/prop.fbx".to_string(),
                name: "prop.fbx".to_string(),
                extension: "fbx".to_string(),
                asset_type: crate::scanner::AssetType::Model,
                size: 1024,
                modified: 0,
                metadata: Some(meta),
                unity_guid: None,
            }
        };
        // Same short-circuit caution as the texture fixtures.
        [mk(50_000, 10, 1), mk(10, 50_000, 1), mk(10, 10, 32)]
            .iter()
            .filter_map(|a| rule.check(a))
            .collect()
    }

    fn harvest_audio() -> Vec<Issue> {
        use rules::audio::{AudioConfig, AudioRule};
        let cfg = AudioConfig {
            enabled: true,
            allowed_sample_rates: vec![44100, 48000],
            max_sfx_duration: 5.0,
            max_file_size: 10 * 1024 * 1024,
            prefer_mono_for_sfx: true,
        };
        let rule = AudioRule::new(cfg);
        let mk = |name: &str, size: u64, meta: crate::scanner::AssetMetadata| AssetInfo {
            path: format!("/p/{name}"),
            name: name.to_string(),
            extension: "wav".to_string(),
            asset_type: crate::scanner::AssetType::Audio,
            size,
            modified: 0,
            metadata: Some(meta),
            unity_guid: None,
        };
        let meta = |rate: u32, secs: Option<f64>, channels: u32| crate::scanner::AssetMetadata {
            sample_rate: Some(rate),
            duration_secs: secs,
            channels: Some(channels),
            ..Default::default()
        };
        // Branch order is sample_rate → sfx_duration → stereo_sfx → file_size, and
        // the two SFX branches additionally require an SFX token in the file name,
        // so later fixtures carry an allowed rate and avoid SFX names.
        [
            mk("ambience.wav", 1024, meta(22050, None, 1)),
            mk("sword_hit.wav", 1024, meta(44100, Some(12.0), 1)),
            mk("ui_click.wav", 1024, meta(44100, Some(0.3), 2)),
            mk(
                "music_loop.wav",
                11 * 1024 * 1024,
                meta(44100, Some(60.0), 2),
            ),
        ]
        .iter()
        .filter_map(|a| rule.check(a))
        .collect()
    }

    fn harvest_colorspace() -> Vec<Issue> {
        use rules::texture_colorspace::TextureColorSpaceRule;
        // `_normal` is a data-texture suffix, so sRGB encoding is the
        // suspicious combination the rule exists to catch.
        let meta = AssetMetadata {
            color_space: Some("sRGB".to_string()),
            ..Default::default()
        };
        TextureColorSpaceRule
            .check(&texture_asset("rock_normal.png", 1024, meta))
            .into_iter()
            .collect()
    }

    fn harvest_duplicate() -> Vec<Issue> {
        use rules::dcc_source::tests::make_asset;
        let dir = tempfile::tempdir().expect("tempdir");
        // The rule buckets by the declared `size` and only then hashes, so the two
        // files have to be byte-identical on disk.
        let paths: Vec<_> = ["a.png", "b.png"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                std::fs::write(&path, b"x").expect("write fixture");
                path
            })
            .collect();
        let assets: Vec<AssetInfo> = paths
            .iter()
            .map(|p| make_asset(&p.to_string_lossy(), AssetType::Texture))
            .collect();
        rules::duplicate::find_duplicates(&assets, &dir.path().to_string_lossy()).issues
    }

    fn harvest_missing_reference() -> Vec<Issue> {
        use rules::missing_reference::tests::{prefab_referencing, texture_with_guid};
        let dir = tempfile::tempdir().expect("tempdir");
        // The rule bails out before reporting anything when no GUID is known
        // at all, so the fixture needs one resolvable asset alongside the
        // prefab's dangling reference.
        let assets = vec![
            texture_with_guid(dir.path(), "known.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "scene.prefab",
                &["22222222222222222222222222222222"],
            ),
        ];
        rules::missing_reference::find_missing_references(
            &assets,
            &assets,
            &Some(crate::scanner::ProjectType::Unity),
            &crate::unity::PackageGuidIndex::default(),
        )
        .issues
    }

    fn harvest_pbr_set() -> Vec<Issue> {
        use rules::pbr_set::PbrSetConfig;
        // `find_pbr_set_issues` reads `config.enabled` in its own body and returns
        // an empty result when it is false, which is the default — unlike the trait
        // rules above, this fixture genuinely needs the override.
        let cfg = PbrSetConfig {
            enabled: true,
            ..PbrSetConfig::default()
        };
        // `_BaseColor` is a default alias of the `basecolor` trigger channel,
        // so a set forms; `normal` is required and has no sibling file.
        let asset = texture_asset("rock_BaseColor.png", 1024, AssetMetadata::default());
        rules::pbr_set::find_pbr_set_issues(&[asset], &cfg).issues
    }

    fn harvest_dcc_source() -> Vec<Issue> {
        use rules::dcc_source::tests::{make_asset, write_with_mtime};
        use rules::dcc_source::DccSourceConfig;
        // Same `config.enabled` caveat as pbr_set. The export is stamped first and
        // the source second, so the gap is at least the 3 days asked for and lands
        // in `humanize_seconds`'s last bucket.
        let cfg = DccSourceConfig {
            enabled: true,
            ..DccSourceConfig::default()
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let blend = dir.path().join("character.blend");
        let fbx = dir.path().join("character.fbx");
        write_with_mtime(&fbx, 86_400 * 3);
        write_with_mtime(&blend, 0);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        rules::dcc_source::find_dcc_source_issues(&assets, &cfg).issues
    }

    #[test]
    fn declared_rule_args_match_what_the_rules_actually_emit() {
        let found = harvest();
        for (rule_id, declared) in RULE_ARGS {
            let actual = found.get(*rule_id).unwrap_or_else(|| {
                panic!("no fixture in harvest() triggers {rule_id} — the declaration is unpinned")
            });
            let expected: BTreeSet<String> = declared.iter().map(|s| s.to_string()).collect();
            assert_eq!(actual, &expected, "args mismatch for {rule_id}");
        }
    }

    #[test]
    fn every_rule_the_harvest_reaches_is_declared() {
        // The other direction of `declared_rule_args_match_…`: a rule whose
        // fixture runs but whose row was never added would otherwise sit
        // unchecked, and the locale gate would silently skip it.
        let found = harvest();
        let declared: BTreeSet<&str> = RULE_ARGS.iter().map(|(id, _)| *id).collect();
        let undeclared: Vec<&String> = found
            .keys()
            .filter(|id| !declared.contains(id.as_str()))
            .collect();
        assert!(
            undeclared.is_empty(),
            "rules emitted but not declared: {undeclared:?}"
        );
        // Guards the harvest, not the analyzer: a 24th rule with no fixture in
        // `harvest()` reaches neither check above. Bump by hand when adding a rule.
        assert_eq!(
            RULE_ARGS.len(),
            23,
            "RULE_ARGS is maintained by hand — a new rule needs a row here and a fixture in harvest()"
        );
    }

    #[test]
    fn test_analysis_result_new() {
        let result = AnalysisResult::new();

        assert_eq!(result.issue_count, 0);
        assert_eq!(result.error_count, 0);
        assert_eq!(result.warning_count, 0);
        assert_eq!(result.info_count, 0);
        assert!(result.issues.is_empty());
    }

    #[test]
    fn test_analysis_result_add_error() {
        let mut result = AnalysisResult::new();

        let issue = Issue {
            rule_id: "test_rule".to_string(),
            rule_name: "Test Rule".to_string(),
            severity: Severity::Error,
            message: "Test error".to_string(),
            asset_path: "/test/file.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        };

        result.add_issue(issue);

        assert_eq!(result.issue_count, 1);
        assert_eq!(result.error_count, 1);
        assert_eq!(result.warning_count, 0);
    }

    #[test]
    fn test_analysis_result_add_warning() {
        let mut result = AnalysisResult::new();

        let issue = Issue {
            rule_id: "test_rule".to_string(),
            rule_name: "Test Rule".to_string(),
            severity: Severity::Warning,
            message: "Test warning".to_string(),
            asset_path: "/test/file.png".to_string(),
            suggestion: Some("Fix this".to_string()),
            auto_fixable: true,
            related_paths: None,
            args: HashMap::new(),
        };

        result.add_issue(issue);

        assert_eq!(result.issue_count, 1);
        assert_eq!(result.warning_count, 1);
        assert_eq!(result.error_count, 0);
    }

    #[test]
    fn test_analysis_result_merge() {
        let mut result1 = AnalysisResult::new();
        let mut result2 = AnalysisResult::new();

        result1.add_issue(Issue {
            rule_id: "rule1".to_string(),
            rule_name: "Rule 1".to_string(),
            severity: Severity::Error,
            message: "Error 1".to_string(),
            asset_path: "/test/file1.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        });

        result2.add_issue(Issue {
            rule_id: "rule2".to_string(),
            rule_name: "Rule 2".to_string(),
            severity: Severity::Warning,
            message: "Warning 1".to_string(),
            asset_path: "/test/file2.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        });

        result1.merge(result2);

        assert_eq!(result1.issue_count, 2);
        assert_eq!(result1.error_count, 1);
        assert_eq!(result1.warning_count, 1);
    }

    #[test]
    fn test_analyzer_new() {
        let analyzer = Analyzer::new();
        assert!(analyzer.rules.is_empty());
    }

    #[test]
    fn test_analyzer_with_default_config() {
        let config = RuleConfig::default();
        let analyzer = Analyzer::with_config(&config);

        // Should have rules added
        assert!(!analyzer.rules.is_empty());
    }

    #[test]
    fn test_severity_equality() {
        assert_eq!(Severity::Error, Severity::Error);
        assert_eq!(Severity::Warning, Severity::Warning);
        assert_eq!(Severity::Info, Severity::Info);
        assert_ne!(Severity::Error, Severity::Warning);
    }

    #[test]
    fn test_by_rule_tracking() {
        let mut result = AnalysisResult::new();

        result.add_issue(Issue {
            rule_id: "rule_a".to_string(),
            rule_name: "Rule A".to_string(),
            severity: Severity::Warning,
            message: "Warning 1".to_string(),
            asset_path: "/test/file1.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        });

        result.add_issue(Issue {
            rule_id: "rule_a".to_string(),
            rule_name: "Rule A".to_string(),
            severity: Severity::Warning,
            message: "Warning 2".to_string(),
            asset_path: "/test/file2.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        });

        result.add_issue(Issue {
            rule_id: "rule_b".to_string(),
            rule_name: "Rule B".to_string(),
            severity: Severity::Error,
            message: "Error 1".to_string(),
            asset_path: "/test/file3.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        });

        assert_eq!(*result.by_rule.get("rule_a").unwrap(), 2);
        assert_eq!(*result.by_rule.get("rule_b").unwrap(), 1);
    }

    #[test]
    fn an_issue_without_args_serializes_exactly_as_before() {
        // `args` mirrors `related_paths`: rules that interpolate nothing must
        // not grow the JSON export by an empty object. Exports are a contract
        // scripts read, and this keeps them byte-identical for those rules.
        let issue = Issue {
            rule_id: "naming.chinese".to_string(),
            rule_name: "Chinese Characters".to_string(),
            severity: Severity::Warning,
            message: "File name contains Chinese characters".to_string(),
            asset_path: "/p/中文.png".to_string(),
            suggestion: None,
            auto_fixable: false,
            related_paths: None,
            args: HashMap::new(),
        };
        let json = serde_json::to_string(&issue).expect("serialize");
        assert!(!json.contains("args"), "empty args must be skipped: {json}");
    }

    #[test]
    fn issue_args_builds_the_map_from_pairs() {
        let a = issue_args([("width", "1024".to_string()), ("height", "768".to_string())]);
        assert_eq!(a.get("width").map(String::as_str), Some("1024"));
        assert_eq!(a.get("height").map(String::as_str), Some("768"));
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn zh_templates_only_use_placeholders_the_rules_emit() {
        // The only place that can see both halves: which placeholders a rule fills
        // and which ones the translations reference. A typo renders as a literal
        // `{{witdh}}`, since neither end substitutes unknown names.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/i18n/locales/zh.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("zh.json parses");
        let declared: BTreeMap<&str, BTreeSet<&str>> = RULE_ARGS
            .iter()
            .map(|(id, args)| (*id, args.iter().copied().collect()))
            .collect();

        let rules = &doc["issues"]["rules"];
        if rules.is_null() {
            return; // nothing translated yet
        }
        for (rule_id, allowed) in &declared {
            // rule_id contains dots, which i18next reads as nesting.
            let mut node = rules;
            for segment in rule_id.split('.') {
                node = &node[segment];
            }
            if node.is_null() {
                continue; // this rule is not translated yet — falls back to English
            }
            for field in ["title", "message", "suggestion"] {
                let Some(tpl) = node[field].as_str() else {
                    continue;
                };
                for name in placeholders(tpl) {
                    assert!(
                        allowed.contains(name.as_str()),
                        "zh.json {rule_id}.{field} uses {{{{{name}}}}}, which {rule_id} never emits (it emits {allowed:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn zh_template_keys_all_name_a_declared_rule() {
        // The other direction of `zh_templates_only_use_placeholders_…`, which walks
        // declared rule id → JSON and skips what is missing, so a key the JSON has
        // and no rule claims is invisible to it.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/i18n/locales/zh.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let doc: serde_json::Value = serde_json::from_str(&raw).expect("zh.json parses");
        let declared: BTreeSet<&str> = RULE_ARGS.iter().map(|(id, _)| *id).collect();

        for segments in leaf_paths(&doc["issues"]["rules"]) {
            // Checked on the segments, not on the joined key: a flat
            // `"texture.max_size"` joins to the same string a properly nested one
            // does, which is why the split is visible only here.
            for segment in &segments {
                assert!(
                    !segment.contains('.'),
                    "zh.json issues.rules has the literal key `{segment}` — i18next reads `.` as \
                     nesting, so the UI misses it and falls back to English while the report, \
                     which flattens the same tree with `.`, finds it. Nest it instead."
                );
            }
            let key = segments.join(".");
            let (rule_id, field) = key
                .rsplit_once('.')
                .unwrap_or_else(|| panic!("zh.json issues.rules.{key} is not <rule_id>.<field>"));
            assert!(
                matches!(field, "title" | "message" | "suggestion"),
                "zh.json issues.rules.{key} ends in `{field}`, not title/message/suggestion"
            );
            assert!(
                declared.contains(rule_id),
                "zh.json issues.rules.{key} translates `{rule_id}`, which no rule emits"
            );
        }

        // The nouns `localized_issue_cells` (report) and `localizeIssue` (UI)
        // resolve `args.age_unit` through. The tags are `humanize_seconds`'.
        for segments in leaf_paths(&doc["issues"]["duration"]) {
            let key = segments.join(".");
            assert!(
                matches!(key.as_str(), "s" | "m" | "h" | "d"),
                "zh.json issues.duration.{key} is not a bucket humanize_seconds emits"
            );
        }
    }

    /// Every leaf under `node`, as the object keys walked to reach it. Kept as
    /// segments rather than a joined key so a key that itself contains the
    /// separator stays distinguishable from real nesting.
    fn leaf_paths(node: &serde_json::Value) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        fn walk(node: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            match node {
                serde_json::Value::Null => {}
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        path.push(k.clone());
                        walk(v, path, out);
                        path.pop();
                    }
                }
                _ => out.push(path.clone()),
            }
        }
        walk(node, &mut Vec::new(), &mut out);
        out
    }

    /// The `{{name}}` names a template references.
    fn placeholders(template: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = template;
        while let Some(start) = rest.find("{{") {
            let tail = &rest[start + 2..];
            match tail.find("}}") {
                Some(end) => {
                    names.push(tail[..end].trim().to_string());
                    rest = &tail[end + 2..];
                }
                None => break,
            }
        }
        names
    }
}
