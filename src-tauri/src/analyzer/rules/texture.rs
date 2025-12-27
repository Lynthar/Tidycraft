use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Require power-of-two dimensions
    #[serde(default = "default_require_pot")]
    pub require_pot: bool,

    /// Maximum texture size (width or height)
    #[serde(default = "default_max_size")]
    pub max_size: u32,

    /// Minimum texture size
    #[serde(default = "default_min_size")]
    pub min_size: u32,

    /// Warn if texture is not square
    #[serde(default)]
    pub warn_non_square: bool,

    /// Maximum file size in bytes
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_require_pot() -> bool {
    true
}

fn default_max_size() -> u32 {
    4096
}

fn default_min_size() -> u32 {
    4
}

fn default_max_file_size() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

impl Default for TextureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            require_pot: true,
            max_size: 4096,
            min_size: 4,
            warn_non_square: false,
            max_file_size: 10 * 1024 * 1024,
        }
    }
}

pub struct TextureRule {
    config: TextureConfig,
}

impl TextureRule {
    pub fn new(config: TextureConfig) -> Self {
        Self { config }
    }

    fn is_power_of_two(n: u32) -> bool {
        n > 0 && (n & (n - 1)) == 0
    }
}

impl Rule for TextureRule {
    fn id(&self) -> &str {
        "texture"
    }

    fn name(&self) -> &str {
        "Texture Standards"
    }

    fn applies_to(&self, asset: &AssetInfo) -> bool {
        matches!(asset.asset_type, AssetType::Texture)
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        let metadata = asset.metadata.as_ref()?;
        let width = metadata.width?;
        let height = metadata.height?;

        // Check POT
        if self.config.require_pot {
            if !Self::is_power_of_two(width) || !Self::is_power_of_two(height) {
                return Some(Issue {
                    rule_id: "texture.pot".to_string(),
                    rule_name: "Non-POT Texture".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Texture dimensions {}x{} are not power of two",
                        width, height
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some(format!(
                        "Resize to {}x{}",
                        next_power_of_two(width),
                        next_power_of_two(height)
                    )),
                    auto_fixable: false,
                });
            }
        }

        // Check max size
        if width > self.config.max_size || height > self.config.max_size {
            return Some(Issue {
                rule_id: "texture.max_size".to_string(),
                rule_name: "Texture Too Large".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "Texture {}x{} exceeds maximum size {}",
                    width, height, self.config.max_size
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(format!(
                    "Resize to {}x{} or smaller",
                    self.config.max_size, self.config.max_size
                )),
                auto_fixable: false,
            });
        }

        // Check min size
        if width < self.config.min_size || height < self.config.min_size {
            return Some(Issue {
                rule_id: "texture.min_size".to_string(),
                rule_name: "Texture Too Small".to_string(),
                severity: Severity::Info,
                message: format!(
                    "Texture {}x{} is smaller than minimum size {}",
                    width, height, self.config.min_size
                ),
                asset_path: asset.path.clone(),
                suggestion: None,
                auto_fixable: false,
            });
        }

        // Check square
        if self.config.warn_non_square && width != height {
            return Some(Issue {
                rule_id: "texture.non_square".to_string(),
                rule_name: "Non-Square Texture".to_string(),
                severity: Severity::Info,
                message: format!("Texture {}x{} is not square", width, height),
                asset_path: asset.path.clone(),
                suggestion: None,
                auto_fixable: false,
            });
        }

        // Check file size
        if asset.size > self.config.max_file_size {
            return Some(Issue {
                rule_id: "texture.file_size".to_string(),
                rule_name: "Large File Size".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "Texture file size {:.2} MB exceeds maximum {:.2} MB",
                    asset.size as f64 / 1024.0 / 1024.0,
                    self.config.max_file_size as f64 / 1024.0 / 1024.0
                ),
                asset_path: asset.path.clone(),
                suggestion: Some("Consider compressing or reducing resolution".to_string()),
                auto_fixable: false,
            });
        }

        None
    }
}

fn next_power_of_two(n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    let mut v = n - 1;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v + 1
}
