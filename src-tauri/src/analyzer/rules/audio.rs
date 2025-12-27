use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Allowed sample rates
    #[serde(default = "default_sample_rates")]
    pub allowed_sample_rates: Vec<u32>,

    /// Maximum duration for sound effects (in seconds)
    #[serde(default = "default_max_sfx_duration")]
    pub max_sfx_duration: f64,

    /// Maximum file size in bytes
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,

    /// Warn about mono vs stereo
    #[serde(default)]
    pub prefer_mono_for_sfx: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_sample_rates() -> Vec<u32> {
    vec![44100, 48000]
}

fn default_max_sfx_duration() -> f64 {
    30.0
}

fn default_max_file_size() -> u64 {
    20 * 1024 * 1024 // 20 MB
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_sample_rates: vec![44100, 48000],
            max_sfx_duration: 30.0,
            max_file_size: 20 * 1024 * 1024,
            prefer_mono_for_sfx: false,
        }
    }
}

pub struct AudioRule {
    config: AudioConfig,
}

impl AudioRule {
    pub fn new(config: AudioConfig) -> Self {
        Self { config }
    }

    fn is_likely_sfx(&self, asset: &AssetInfo) -> bool {
        // Simple heuristic: if name contains common SFX indicators
        let name_lower = asset.name.to_lowercase();
        name_lower.contains("sfx")
            || name_lower.contains("sound")
            || name_lower.contains("effect")
            || name_lower.contains("hit")
            || name_lower.contains("click")
            || name_lower.contains("ui")
    }
}

impl Rule for AudioRule {
    fn id(&self) -> &str {
        "audio"
    }

    fn name(&self) -> &str {
        "Audio Standards"
    }

    fn applies_to(&self, asset: &AssetInfo) -> bool {
        matches!(asset.asset_type, AssetType::Audio)
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        let metadata = asset.metadata.as_ref()?;

        // Check sample rate
        if let Some(sample_rate) = metadata.sample_rate {
            if !self.config.allowed_sample_rates.contains(&sample_rate) {
                return Some(Issue {
                    rule_id: "audio.sample_rate".to_string(),
                    rule_name: "Non-Standard Sample Rate".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "Audio sample rate {} Hz is not standard (expected {:?})",
                        sample_rate, self.config.allowed_sample_rates
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some(format!(
                        "Consider resampling to {} Hz",
                        self.config.allowed_sample_rates[0]
                    )),
                    auto_fixable: false,
                });
            }
        }

        // Check SFX duration
        if let Some(duration) = metadata.duration_secs {
            if self.is_likely_sfx(asset) && duration > self.config.max_sfx_duration {
                return Some(Issue {
                    rule_id: "audio.sfx_duration".to_string(),
                    rule_name: "Long Sound Effect".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Sound effect is {:.1}s long, maximum recommended is {:.0}s",
                        duration, self.config.max_sfx_duration
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some("Long audio should be music/ambient, not SFX".to_string()),
                    auto_fixable: false,
                });
            }
        }

        // Check stereo for SFX
        if self.config.prefer_mono_for_sfx {
            if let Some(channels) = metadata.channels {
                if self.is_likely_sfx(asset) && channels > 1 {
                    return Some(Issue {
                        rule_id: "audio.stereo_sfx".to_string(),
                        rule_name: "Stereo Sound Effect".to_string(),
                        severity: Severity::Info,
                        message: "Sound effect is stereo, mono is recommended for 3D audio"
                            .to_string(),
                        asset_path: asset.path.clone(),
                        suggestion: Some("Convert to mono for better 3D spatialization".to_string()),
                        auto_fixable: false,
                    });
                }
            }
        }

        // Check file size
        if asset.size > self.config.max_file_size {
            return Some(Issue {
                rule_id: "audio.file_size".to_string(),
                rule_name: "Large Audio File".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "Audio file size {:.2} MB exceeds maximum {:.2} MB",
                    asset.size as f64 / 1024.0 / 1024.0,
                    self.config.max_file_size as f64 / 1024.0 / 1024.0
                ),
                asset_path: asset.path.clone(),
                suggestion: Some("Consider using compressed format (OGG/MP3)".to_string()),
                auto_fixable: false,
            });
        }

        None
    }
}
