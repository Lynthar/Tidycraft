use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Forbidden characters in file names
    #[serde(default = "default_forbidden_chars")]
    pub forbidden_chars: Vec<char>,

    /// Whether to forbid Chinese characters
    #[serde(default = "default_forbid_chinese")]
    pub forbid_chinese: bool,

    /// Maximum file name length
    #[serde(default = "default_max_length")]
    pub max_length: usize,

    /// Required prefix for textures
    #[serde(default)]
    pub texture_prefix: Option<String>,

    /// Required prefix for models
    #[serde(default)]
    pub model_prefix: Option<String>,

    /// Required prefix for audio
    #[serde(default)]
    pub audio_prefix: Option<String>,

    /// Naming case style: "PascalCase", "snake_case", "camelCase", or "any"
    #[serde(default = "default_case_style")]
    pub case_style: String,
}

fn default_enabled() -> bool {
    true
}

fn default_forbidden_chars() -> Vec<char> {
    vec![' ', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '+', '=']
}

fn default_forbid_chinese() -> bool {
    true
}

fn default_max_length() -> usize {
    64
}

fn default_case_style() -> String {
    "any".to_string()
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            forbidden_chars: default_forbidden_chars(),
            forbid_chinese: true,
            max_length: 64,
            texture_prefix: Some("T_".to_string()),
            model_prefix: None,
            audio_prefix: None,
            case_style: "any".to_string(),
        }
    }
}

pub struct NamingRule {
    config: NamingConfig,
}

impl NamingRule {
    pub fn new(config: NamingConfig) -> Self {
        Self { config }
    }

    fn check_forbidden_chars(&self, name: &str) -> Option<char> {
        for c in name.chars() {
            if self.config.forbidden_chars.contains(&c) {
                return Some(c);
            }
        }
        None
    }

    fn check_chinese(&self, name: &str) -> bool {
        if !self.config.forbid_chinese {
            return false;
        }
        name.chars().any(|c| {
            let code = c as u32;
            // CJK Unified Ideographs range
            (0x4E00..=0x9FFF).contains(&code)
                || (0x3400..=0x4DBF).contains(&code)
                || (0x20000..=0x2A6DF).contains(&code)
        })
    }

    fn check_prefix(&self, name: &str, asset_type: &AssetType) -> Option<String> {
        let required_prefix = match asset_type {
            AssetType::Texture => self.config.texture_prefix.as_ref(),
            AssetType::Model => self.config.model_prefix.as_ref(),
            AssetType::Audio => self.config.audio_prefix.as_ref(),
            _ => None,
        };

        if let Some(prefix) = required_prefix {
            if !name.starts_with(prefix) {
                return Some(prefix.clone());
            }
        }
        None
    }

    fn check_case_style(&self, name: &str) -> bool {
        match self.config.case_style.as_str() {
            "PascalCase" => is_pascal_case(name),
            "snake_case" => is_snake_case(name),
            "camelCase" => is_camel_case(name),
            _ => true, // "any" or unknown
        }
    }
}

impl Rule for NamingRule {
    fn id(&self) -> &str {
        "naming"
    }

    fn name(&self) -> &str {
        "Naming Convention"
    }

    fn applies_to(&self, _asset: &AssetInfo) -> bool {
        true // Applies to all assets
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        let name = &asset.name;
        let name_without_ext = name.rsplit_once('.').map(|(n, _)| n).unwrap_or(name);

        // Check length
        if name.len() > self.config.max_length {
            return Some(Issue {
                rule_id: "naming.length".to_string(),
                rule_name: "Name Too Long".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "File name is {} characters, max allowed is {}",
                    name.len(),
                    self.config.max_length
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Shorten the file name to {} characters", self.config.max_length)),
                auto_fixable: false,
            });
        }

        // Check forbidden characters
        if let Some(c) = self.check_forbidden_chars(name) {
            return Some(Issue {
                rule_id: "naming.forbidden_char".to_string(),
                rule_name: "Forbidden Character".to_string(),
                severity: Severity::Warning,
                message: format!("File name contains forbidden character: '{}'", c),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Remove '{}' from the file name", c)),
                auto_fixable: true,
            });
        }

        // Check Chinese characters
        if self.check_chinese(name) {
            return Some(Issue {
                rule_id: "naming.chinese".to_string(),
                rule_name: "Chinese Characters".to_string(),
                severity: Severity::Warning,
                message: "File name contains Chinese characters".to_string(),
                asset_path: asset.path.clone(),
                suggestion: Some("Use English characters for file names".to_string()),
                auto_fixable: false,
            });
        }

        // Check prefix
        if let Some(prefix) = self.check_prefix(name, &asset.asset_type) {
            return Some(Issue {
                rule_id: "naming.prefix".to_string(),
                rule_name: "Missing Prefix".to_string(),
                severity: Severity::Warning,
                message: format!("File name should start with '{}'", prefix),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Rename to {}{}", prefix, name)),
                auto_fixable: true,
            });
        }

        // Check case style
        if !self.check_case_style(name_without_ext) {
            return Some(Issue {
                rule_id: "naming.case".to_string(),
                rule_name: "Naming Case".to_string(),
                severity: Severity::Info,
                message: format!(
                    "File name does not follow {} convention",
                    self.config.case_style
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(format!("Use {} for file names", self.config.case_style)),
                auto_fixable: true,
            });
        }

        None
    }
}

fn is_pascal_case(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let first = s.chars().next().unwrap();
    first.is_uppercase() && !s.contains('_') && !s.chars().all(|c| c.is_uppercase())
}

fn is_snake_case(s: &str) -> bool {
    s.chars().all(|c| c.is_lowercase() || c.is_numeric() || c == '_')
}

fn is_camel_case(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let first = s.chars().next().unwrap();
    first.is_lowercase() && !s.contains('_')
}
