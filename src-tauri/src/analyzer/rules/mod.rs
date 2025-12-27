pub mod audio;
pub mod duplicate;
pub mod model;
pub mod naming;
pub mod texture;

use crate::analyzer::Issue;
use crate::scanner::AssetInfo;
use serde::{Deserialize, Serialize};

/// Trait for all analysis rules
pub trait Rule: Send + Sync {
    /// Unique identifier for the rule
    fn id(&self) -> &str;

    /// Human-readable name
    fn name(&self) -> &str;

    /// Check if this rule applies to a given asset type
    fn applies_to(&self, asset: &AssetInfo) -> bool;

    /// Run the check and return an issue if found
    fn check(&self, asset: &AssetInfo) -> Option<Issue>;
}

/// Configuration for all rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    #[serde(default)]
    pub naming: naming::NamingConfig,
    #[serde(default)]
    pub texture: texture::TextureConfig,
    #[serde(default)]
    pub model: model::ModelConfig,
    #[serde(default)]
    pub audio: audio::AudioConfig,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            naming: naming::NamingConfig::default(),
            texture: texture::TextureConfig::default(),
            model: model::ModelConfig::default(),
            audio: audio::AudioConfig::default(),
        }
    }
}

impl RuleConfig {
    /// Load config from TOML string
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Serialize config to TOML string
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}
