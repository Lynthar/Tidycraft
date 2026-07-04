use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Allowed sample rates. An empty list disables the check.
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
    // Out-of-box OFF: sample-rate / duration limits are pipeline-
    // specific. Users opt in via tidycraft.toml.
    false
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
            enabled: false,
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
        // Heuristic: the filename carries a common SFX indicator as a whole
        // token. Token equality, NOT substring — `guitar.wav` contains "ui"
        // and `white_noise.wav` contains "hit", and both used to be judged
        // SFX (then warned for exceeding max_sfx_duration). Tokens split on
        // non-alphanumeric boundaries and lower/upper camelCase seams, so
        // `sword_hit_01`, `UIClick`, and `Sfx-Explosion` all still match.
        const SFX_TOKENS: [&str; 6] = ["sfx", "sound", "effect", "hit", "click", "ui"];
        sfx_name_tokens(&asset.name).any(|tok| SFX_TOKENS.contains(&tok.as_str()))
    }
}

/// Split a filename into lowercase word tokens: separators are any
/// non-alphanumeric run, plus lower→upper camelCase seams ("UIClick" →
/// ["ui", "click"] — an uppercase run followed by a lowercase letter
/// starts a new word at its last capital).
fn sfx_name_tokens(name: &str) -> impl Iterator<Item = String> + '_ {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        let prev = i.checked_sub(1).and_then(|j| chars.get(j).copied());
        let next = chars.get(i + 1).copied();
        let camel_seam = c.is_uppercase()
            && (prev.is_some_and(|p| p.is_lowercase())
                || (prev.is_some_and(|p| p.is_uppercase())
                    && next.is_some_and(|n| n.is_lowercase())));
        if camel_seam && !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.into_iter()
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

        // Check sample rate. An empty allow-list means "no constraint" —
        // skip entirely rather than flag every rate (indexing [0] below
        // used to panic on `allowed_sample_rates = []` in tidycraft.toml).
        if let (Some(sample_rate), Some(&preferred)) = (
            metadata.sample_rate,
            self.config.allowed_sample_rates.first(),
        ) {
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
                    suggestion: Some(format!("Consider resampling to {} Hz", preferred)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::AssetMetadata;

    fn audio_asset(sample_rate: u32) -> AssetInfo {
        AssetInfo {
            path: "audio/music/theme.wav".to_string(),
            name: "theme.wav".to_string(),
            extension: "wav".to_string(),
            asset_type: AssetType::Audio,
            size: 1024,
            modified: 0,
            metadata: Some(AssetMetadata {
                sample_rate: Some(sample_rate),
                ..Default::default()
            }),
            unity_guid: None,
        }
    }

    #[test]
    fn empty_allowed_sample_rates_disables_check_instead_of_panicking() {
        let rule = AudioRule::new(AudioConfig {
            allowed_sample_rates: vec![],
            ..Default::default()
        });
        // `allowed_sample_rates = []` in tidycraft.toml used to flag every
        // rate as non-standard and then panic building the suggestion
        // (indexed [0] into the empty list). Empty list = check off.
        assert!(rule.check(&audio_asset(22050)).is_none());
    }

    #[test]
    fn non_listed_sample_rate_still_reports() {
        let rule = AudioRule::new(AudioConfig::default());
        let issue = rule.check(&audio_asset(22050)).expect("22.05 kHz is non-standard");
        assert_eq!(issue.rule_id, "audio.sample_rate");
        assert!(issue.suggestion.expect("has suggestion").contains("44100"));
    }
}

#[cfg(test)]
mod sfx_token_tests {
    use super::sfx_name_tokens;

    fn toks(name: &str) -> Vec<String> {
        sfx_name_tokens(name).collect()
    }

    #[test]
    fn tokenizes_separators_and_camel_case() {
        assert_eq!(toks("sword_hit_01.wav"), ["sword", "hit", "01", "wav"]);
        assert_eq!(toks("UIClick.wav"), ["ui", "click", "wav"]);
        assert_eq!(toks("Sfx-Explosion.ogg"), ["sfx", "explosion", "ogg"]);
    }

    #[test]
    fn substrings_inside_words_do_not_leak() {
        // "guitar" must NOT produce a "ui" token, "white" no "hit" token.
        assert!(!toks("guitar_loop.wav").iter().any(|t| t == "ui"));
        assert!(!toks("white_noise.wav").iter().any(|t| t == "hit"));
    }
}
