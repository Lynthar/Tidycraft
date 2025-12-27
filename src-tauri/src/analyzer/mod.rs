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
