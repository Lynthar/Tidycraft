use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::texture_colorspace::TextureColorSpaceConfig;
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

    /// Color-space mismatch detection. Lives under `[texture.color_space]`
    /// in the TOML; gated independently from this section's `enabled`
    /// flag so users can turn off PoT / size / file-size checks without
    /// also losing the sRGB-data-texture safety net.
    #[serde(default)]
    pub color_space: TextureColorSpaceConfig,
}

fn default_enabled() -> bool {
    // Out-of-box OFF: texture standards are stylistic conventions
    // (PoT, max-size, file-size). Users opt in via tidycraft.toml.
    // The independent `[texture.color_space]` rule stays on because
    // it's a real bug check, not a convention.
    false
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
            enabled: false,
            require_pot: true,
            max_size: 4096,
            min_size: 4,
            warn_non_square: false,
            max_file_size: 10 * 1024 * 1024,
            color_space: TextureColorSpaceConfig::default(),
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
        // Dimensions are optional: PSD/PSB and other exotic formats often
        // parse without them. The old `metadata.width?` early-return exempted
        // exactly those (typically the LARGEST files) from every check — the
        // file-size check below must run regardless; only the dimension-
        // dependent checks are gated.
        let dims = asset
            .metadata
            .as_ref()
            .and_then(|m| Some((m.width?, m.height?)));

        if let Some((width, height)) = dims {
            if let Some(issue) = self.check_dimensions(asset, width, height) {
                return Some(issue);
            }
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

        if let Some((width, height)) = dims {
            if let Some(issue) = self.check_mipmaps(asset, width, height) {
                return Some(issue);
            }
        }

        None
    }
}

impl TextureRule {
    /// The dimension-dependent checks (POT / max / min / square), in their
    /// historical precedence order.
    fn check_dimensions(&self, asset: &AssetInfo, width: u32, height: u32) -> Option<Issue> {
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

        None
    }

    /// DDS-only: warn when a large texture ships without a mipmap chain.
    /// Only DDS stores mipmap_count; for other formats the engine generates
    /// them on import so the file alone doesn't tell us. 512px threshold
    /// skirts UI textures that legitimately ship base-only.
    fn check_mipmaps(&self, asset: &AssetInfo, width: u32, height: u32) -> Option<Issue> {
        if let Some(mips) = asset.metadata.as_ref().and_then(|m| m.mipmap_count) {
            if mips <= 1 && (width >= 512 || height >= 512) {
                return Some(Issue {
                    rule_id: "texture.no_mipmaps".to_string(),
                    rule_name: "No Mipmap Chain".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "DDS texture {}x{} ships without mipmaps. Distant rendering will show aliasing / moiré and sample the full resolution each frame.",
                        width, height
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some(
                        "Regenerate the DDS with mipmaps, or confirm this is intentional (UI / LUT textures can skip)."
                            .to_string(),
                    ),
                    auto_fixable: false,
                });
            }
        }

        None
    }
}

fn next_power_of_two(n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    // Above 2^31 there is no next power of two representable in u32; the
    // bit-fill + 1 below would overflow (panic in debug, wrap to 0 in release).
    // Cap at 2^31 — a dimension this large only comes from a corrupt texture
    // header, and this value only feeds a human-readable "Resize to NxN"
    // suggestion, not any real computation.
    if n > (1 << 31) {
        return 1 << 31;
    }
    let mut v = n - 1;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, AssetType};

    fn psd_without_dims(size: u64) -> AssetInfo {
        AssetInfo {
            path: "/p/huge.psd".to_string(),
            name: "huge.psd".to_string(),
            extension: "psd".to_string(),
            asset_type: AssetType::Texture,
            size,
            modified: 0,
            // Parsed, but no width/height (typical for PSD/PSB).
            metadata: Some(AssetMetadata::default()),
            unity_guid: None,
        }
    }

    #[test]
    fn file_size_check_covers_textures_without_dimensions() {
        let rule = TextureRule::new(TextureConfig::default());
        // Over the default 10 MB cap: must fire even though the old
        // `width?` early-return used to exempt exactly these files.
        let issue = rule.check(&psd_without_dims(11 * 1024 * 1024));
        assert_eq!(issue.expect("expected an issue").rule_id, "texture.file_size");
        // Under the cap: silent.
        assert!(rule.check(&psd_without_dims(1024)).is_none());
    }
}
