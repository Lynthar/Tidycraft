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
#[serde(deny_unknown_fields)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A misspelled key used to be dropped on the floor: `max_sze = 512` left
    /// `max_size` at its default and the analysis ran with settings the user
    /// believed they had changed, with nothing on screen to say otherwise.
    /// Every caller of `from_toml` turns the error into "Invalid config: …",
    /// so refusing the key produces a message naming it — actionable, where
    /// the silent default was not even noticeable.
    #[test]
    fn a_misspelled_rule_key_is_reported_rather_than_silently_ignored() {
        let err = RuleConfig::from_toml("[texture]\nenabled = true\nmax_sze = 512\n")
            .expect_err("a misspelled key must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("max_sze"),
            "the error has to name the offending key: {}",
            msg
        );
    }

    /// `[project]` is not part of `RuleConfig` — it carries the AI-tagging
    /// metadata that `ProjectMeta` reads out of the same file. The top level
    /// therefore has to stay permissive; only the rule sections are strict.
    #[test]
    fn the_project_section_still_coexists_with_the_rule_sections() {
        let cfg =
            RuleConfig::from_toml("[project]\ntheme = \"cyberpunk\"\n\n[texture]\nenabled = true\n")
                .expect("[project] must not break rule parsing");
        assert!(cfg.texture.enabled);
    }

    /// The template written into every new project has to survive the same
    /// strictness. It is kept in sync with the `default_*` functions by hand,
    /// and its own doc comment concedes there is no compile-time check for
    /// that — this is the check: a key the template offers that no struct
    /// accepts now fails here instead of silently doing nothing in the field.
    #[test]
    fn the_shipped_template_parses_under_strict_sections() {
        RuleConfig::from_toml(config_template::DEFAULT_CONFIG_TEMPLATE)
            .expect("the template we write into user projects must parse");
    }
}
