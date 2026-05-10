pub mod audio;
pub mod config_template;
pub mod dcc_source;
pub mod duplicate;
pub mod missing_reference;
pub mod model;
pub mod naming;
pub mod pbr_set;
pub mod texture;
pub mod texture_colorspace;

use crate::analyzer::Issue;
use crate::scanner::AssetInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IgnoreConfig {
    /// Glob patterns matched against asset paths relative to the project
    /// root. Any asset whose relative path matches at least one pattern is
    /// dropped before per-rule checks, duplicate detection, and missing-
    /// reference scanning. Empty (the default) means analyze everything.
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// Trait for all analysis rules. `id` and `name` are part of the public
/// interface for future diagnostics output (UI grouping, error messages)
/// even though no caller in lib.rs reads them yet.
#[allow(dead_code)]
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
    #[serde(default)]
    pub pbr_set: pbr_set::PbrSetConfig,
    #[serde(default)]
    pub dcc_source: dcc_source::DccSourceConfig,
    #[serde(default)]
    pub ignore: IgnoreConfig,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            naming: naming::NamingConfig::default(),
            texture: texture::TextureConfig::default(),
            model: model::ModelConfig::default(),
            audio: audio::AudioConfig::default(),
            pbr_set: pbr_set::PbrSetConfig::default(),
            dcc_source: dcc_source::DccSourceConfig::default(),
            ignore: IgnoreConfig::default(),
        }
    }
}

impl RuleConfig {
    /// Load config from TOML string
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}
