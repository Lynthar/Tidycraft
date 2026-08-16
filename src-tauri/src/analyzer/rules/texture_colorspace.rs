//! Flag likely colour-space / data-channel mismatches on textures: a normal map
//! or roughness mask exported as PNG with an `sRGB` chunk is de-gammaed at
//! import. Both the declared colour space and a filename hint must fire.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::analyzer::{issue_args, Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};

use super::Rule;

/// Lives under `[texture.color_space]` in the TOML, gated separately from
/// `[texture]`'s own flag. Default ON: this catches a real corruption bug, not a
/// stylistic convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureColorSpaceConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for TextureColorSpaceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Stem suffixes (case-insensitive, `ends_with` after lowercasing) that imply the
/// texture is data, not sRGB colour. `_n` is the only single-letter entry; `_r`
/// and `_m` collide with ordinary names too readily.
const DATA_HINTS: &[&str] = &[
    "_n",
    "_normal",
    "_norm",
    "_nrm",
    "_rough",
    "_roughness",
    "_metal",
    "_metallic",
    "_ao",
    "_mask",
    "_data",
    "_lin",
    "_linear",
    "_height",
    "_disp",
    "_displacement",
    "_orm",
    "_mra",
    "_rma",
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
        // Only fire when the file is KNOWN to be sRGB-encoded. An unknown colour
        // space is skipped: many perfectly fine data maps carry no explicit chunk.
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
            related_paths: None,
            args: issue_args([("suffix", matched.to_string())]),
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
            modified: 0,
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

    #[test]
    fn ignores_non_pbr_single_letter_collisions() {
        // `_r` ("right") and `_m` ("medium"/UI size) collide with ordinary
        // naming far too often — same reasoning as tag_suggest's suffix list.
        // Only `_n` earns single-letter status (see DATA_HINTS comment).
        let rule = TextureColorSpaceRule;
        assert!(rule.check(&texture("arrow_r.png", Some("sRGB"))).is_none());
        assert!(rule.check(&texture("icon_m.png", Some("sRGB"))).is_none());
    }

    #[test]
    fn still_fires_on_single_letter_normal_suffix() {
        // `_n` stays: it's the dominant shorthand for normal maps, and a
        // normal map de-gammaed as sRGB is the worst-case corruption.
        let rule = TextureColorSpaceRule;
        assert!(rule.check(&texture("rock_n.png", Some("sRGB"))).is_some());
    }
}
