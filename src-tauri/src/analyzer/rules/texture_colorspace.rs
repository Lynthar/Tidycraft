//! Flag likely color-space / data-channel mismatches on textures.
//!
//! Common shipping bug: artist exports a normal map / roughness mask as PNG
//! with an `sRGB` chunk. At import the engine de-gammas the pixels, silently
//! corrupting the data channel. We catch this by combining two cheap signals:
//!
//! 1. The file's declared color space (from PNG's `sRGB` / `iCCP` chunks).
//! 2. A filename-suffix heuristic for "this should be linear data, not color".
//!
//! Both signals must fire for a warning, so pure color textures and old PNGs
//! without a color profile don't produce false positives.

use std::path::Path;

use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};

use super::Rule;

/// Stem suffixes (case-insensitive, `ends_with` match after lowercasing)
/// that imply the texture is data, not sRGB color.
const DATA_HINTS: &[&str] = &[
    "_n", "_normal", "_norm", "_nrm",
    "_r", "_rough", "_roughness",
    "_m", "_metal", "_metallic",
    "_ao", "_mask",
    "_data", "_lin", "_linear",
    "_height", "_disp", "_displacement",
    "_orm", "_mra", "_rma",
];

pub struct TextureColorSpaceRule;

impl Rule for TextureColorSpaceRule {
    fn id(&self) -> &str {
        "texture.color_space"
    }

    fn name(&self) -> &str {
        "Texture Color Space"
    }

    fn applies_to(&self, asset: &AssetInfo) -> bool {
        matches!(asset.asset_type, AssetType::Texture)
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        // Only fire when we KNOW the file is sRGB-encoded. If color_space
        // is None (unknown), skip — some perfectly fine data maps don't
        // carry an explicit chunk, and we'd rather miss those than spam
        // false positives.
        let metadata = asset.metadata.as_ref()?;
        let color_space = metadata.color_space.as_deref()?;
        if color_space != "sRGB" {
            return None;
        }

        let stem_lower = Path::new(&asset.name)
            .file_stem()
            .and_then(|s| s.to_str())?
            .to_lowercase();

        let matched = DATA_HINTS.iter().find(|&&h| stem_lower.ends_with(h))?;

        Some(Issue {
            rule_id: "texture.color_space".to_string(),
            rule_name: "Suspicious Color Space".to_string(),
            severity: Severity::Warning,
            message: format!(
                "Filename suffix `{}` implies a data texture (normal / roughness / mask / …) but the file is encoded as sRGB. The engine will de-gamma these pixels at import and corrupt the channel.",
                matched
            ),
            asset_path: asset.path.clone(),
            suggestion: Some(
                "Re-export with Linear color space, or explicitly mark the texture as non-color data (sRGB off) in the engine's import settings."
                    .to_string(),
            ),
            auto_fixable: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::AssetMetadata;

    fn texture(name: &str, color_space: Option<&str>) -> AssetInfo {
        AssetInfo {
            path: format!("/test/{}", name),
            name: name.to_string(),
            extension: name.rsplit('.').next().unwrap_or("png").to_string(),
            asset_type: AssetType::Texture,
            size: 1024,
            metadata: Some(AssetMetadata {
                color_space: color_space.map(str::to_string),
                ..Default::default()
            }),
            unity_guid: None,
        }
    }

    #[test]
    fn fires_on_normal_map_with_srgb() {
        let rule = TextureColorSpaceRule;
        let asset = texture("rock_n.png", Some("sRGB"));
        assert!(rule.check(&asset).is_some());
    }

    #[test]
    fn fires_on_roughness_map_with_srgb() {
        let rule = TextureColorSpaceRule;
        let asset = texture("metal_roughness.png", Some("sRGB"));
        assert!(rule.check(&asset).is_some());
    }

    #[test]
    fn ignores_data_map_without_color_space_info() {
        let rule = TextureColorSpaceRule;
        let asset = texture("rock_n.png", None);
        assert!(rule.check(&asset).is_none());
    }

    #[test]
    fn ignores_pure_color_texture_with_srgb() {
        let rule = TextureColorSpaceRule;
        let asset = texture("grass_albedo.png", Some("sRGB"));
        assert!(rule.check(&asset).is_none());
    }

    #[test]
    fn ignores_data_map_already_linear() {
        let rule = TextureColorSpaceRule;
        let asset = texture("rock_n.png", Some("Linear"));
        assert!(rule.check(&asset).is_none());
    }

    #[test]
    fn case_insensitive_suffix() {
        let rule = TextureColorSpaceRule;
        let asset = texture("ROCK_N.PNG", Some("sRGB"));
        assert!(rule.check(&asset).is_some());
    }
}
