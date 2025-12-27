pub mod rules;

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

        // Add texture rules
        if config.texture.enabled {
            analyzer.add_rule(Box::new(rules::texture::TextureRule::new(
                config.texture.clone(),
            )));
        }

        // Add model rules
        if config.model.enabled {
            analyzer.add_rule(Box::new(rules::model::ModelRule::new(
                config.model.clone(),
            )));
        }

        // Add audio rules
        if config.audio.enabled {
            analyzer.add_rule(Box::new(rules::audio::AudioRule::new(
                config.audio.clone(),
            )));
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
        rules::duplicate::find_duplicates(&scan_result.assets)
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

    fn create_test_asset(name: &str, asset_type: AssetType) -> AssetInfo {
        AssetInfo {
            path: format!("/test/{}", name),
            name: name.to_string(),
            extension: name.split('.').last().unwrap_or("").to_string(),
            asset_type,
            size: 1024,
            metadata: None,
            unity_guid: None,
        }
    }

    fn create_texture_with_dimensions(name: &str, width: u32, height: u32) -> AssetInfo {
        AssetInfo {
            path: format!("/test/{}", name),
            name: name.to_string(),
            extension: "png".to_string(),
            asset_type: AssetType::Texture,
            size: 1024,
            metadata: Some(AssetMetadata {
                width: Some(width),
                height: Some(height),
                has_alpha: Some(false),
                ..Default::default()
            }),
            unity_guid: None,
        }
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
        });

        result2.add_issue(Issue {
            rule_id: "rule2".to_string(),
            rule_name: "Rule 2".to_string(),
            severity: Severity::Warning,
            message: "Warning 1".to_string(),
            asset_path: "/test/file2.png".to_string(),
            suggestion: None,
            auto_fixable: false,
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
        });

        result.add_issue(Issue {
            rule_id: "rule_a".to_string(),
            rule_name: "Rule A".to_string(),
            severity: Severity::Warning,
            message: "Warning 2".to_string(),
            asset_path: "/test/file2.png".to_string(),
            suggestion: None,
            auto_fixable: false,
        });

        result.add_issue(Issue {
            rule_id: "rule_b".to_string(),
            rule_name: "Rule B".to_string(),
            severity: Severity::Error,
            message: "Error 1".to_string(),
            asset_path: "/test/file3.png".to_string(),
            suggestion: None,
            auto_fixable: false,
        });

        assert_eq!(*result.by_rule.get("rule_a").unwrap(), 2);
        assert_eq!(*result.by_rule.get("rule_b").unwrap(), 1);
    }
}
