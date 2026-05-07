//! PBR material set completeness check.
//!
//! Detects when a directory contains a partial PBR texture set — e.g., a
//! BaseColor exists but the Normal sibling is missing. This is a cross-
//! asset check (it operates on groups of textures sharing the same base
//! name in the same directory), so it lives outside the per-asset Rule
//! trait and is invoked separately from `analyze_assets` like the
//! duplicate / missing-reference passes.
//!
//! Design highlights
//! - Strict `_<suffix>` parsing (the underscore must be present) so
//!   `T_brand_new.png` is not misread as a normal map (the suffix `new`
//!   doesn't match any configured channel).
//! - A set forms only when the trigger channel (default: `basecolor`) is
//!   present, so directories of UI / non-PBR textures don't form spurious
//!   sets and produce no issues — this also makes the rule safe to leave
//!   on by default for projects that don't follow PBR naming.
//! - Packed channels (ORM, MRA, RMA) satisfy multiple roles at once when
//!   listed in `packed`, so Substance Painter's bundled output doesn't
//!   trip false missing-channel warnings.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analyzer::{AnalysisResult, Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PbrSetConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Channel role → suffix list (case-insensitive). E.g.
    /// `basecolor = ["BaseColor", "Albedo"]` means a file ending in
    /// `_BaseColor` or `_Albedo` is recognized as the BaseColor channel.
    #[serde(default = "default_channels")]
    pub channels: HashMap<String, Vec<String>>,
    /// Packed-channel suffixes that satisfy multiple roles at once.
    /// E.g. `ORM = ["ao", "roughness", "metallic"]` — a file ending in
    /// `_ORM` counts as if all three channel roles were present.
    #[serde(default = "default_packed")]
    pub packed: HashMap<String, Vec<String>>,
    /// A set must contain this channel role to be checked. Default
    /// `"basecolor"` — directories of pure UI / effect textures don't
    /// trigger and produce no issues.
    #[serde(default = "default_trigger")]
    pub trigger: String,
    /// Channel roles that must exist in every triggered set.
    #[serde(default = "default_required")]
    pub required: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_trigger() -> String {
    "basecolor".into()
}

fn default_required() -> Vec<String> {
    vec_str(&["basecolor", "normal"])
}

fn default_channels() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert(
        "basecolor".into(),
        vec_str(&["BaseColor", "Albedo", "Diffuse", "Color"]),
    );
    m.insert("normal".into(), vec_str(&["Normal", "Norm"]));
    m.insert("roughness".into(), vec_str(&["Roughness", "Rough"]));
    m.insert("metallic".into(), vec_str(&["Metallic", "Metal"]));
    m.insert("ao".into(), vec_str(&["AO", "AmbientOcclusion"]));
    m.insert("emissive".into(), vec_str(&["Emissive", "Emission"]));
    m.insert("height".into(), vec_str(&["Height", "Disp"]));
    m
}

fn default_packed() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert("ORM".into(), vec_str(&["ao", "roughness", "metallic"]));
    m.insert("MRA".into(), vec_str(&["metallic", "roughness", "ao"]));
    m.insert("RMA".into(), vec_str(&["roughness", "metallic", "ao"]));
    m
}

fn vec_str(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

impl Default for PbrSetConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            channels: default_channels(),
            packed: default_packed(),
            trigger: default_trigger(),
            required: default_required(),
        }
    }
}

#[derive(Debug, Clone)]
enum RoleKind {
    /// Single-role suffix matched a `channels` entry. Carries the role
    /// key (e.g. "basecolor", "normal").
    Channel(String),
    /// Packed suffix matched a `packed` entry. Carries the packed key
    /// (e.g. "ORM"); resolution to the multiple roles it covers happens
    /// at set-aggregation time so a custom `packed` table is honored.
    Packed(String),
}

/// Parse a stem like `T_RustyMetal_BaseColor` into
/// `(base_stem, RoleKind)`. Returns `None` if no `_<suffix>` matches the
/// configured channel or packed lists.
///
/// Strict semantics: the suffix is the substring after the LAST `_`.
/// `T_brand_new` finds suffix `new` — which won't match anything by
/// default, so the file is silently skipped (it just won't form part of
/// any PBR set).
fn parse_stem(
    stem: &str,
    channels: &HashMap<String, Vec<String>>,
    packed: &HashMap<String, Vec<String>>,
) -> Option<(String, RoleKind)> {
    let last_underscore = stem.rfind('_')?;
    let (base, suffix_with_underscore) = stem.split_at(last_underscore);
    let suffix = &suffix_with_underscore[1..]; // skip the `_`
    if suffix.is_empty() {
        return None;
    }
    let suffix_lower = suffix.to_lowercase();

    for (role, suffixes) in channels {
        if suffixes.iter().any(|s| s.to_lowercase() == suffix_lower) {
            return Some((base.to_string(), RoleKind::Channel(role.to_lowercase())));
        }
    }
    for packed_key in packed.keys() {
        if packed_key.to_lowercase() == suffix_lower {
            return Some((base.to_string(), RoleKind::Packed(packed_key.clone())));
        }
    }
    None
}

/// Run the cross-asset PBR set completeness check.
///
/// Returns issues for every set that contains the trigger channel but
/// lacks one or more of `config.required`.
pub fn find_pbr_set_issues(assets: &[AssetInfo], config: &PbrSetConfig) -> AnalysisResult {
    let mut result = AnalysisResult::new();
    if !config.enabled {
        return result;
    }

    // (directory, base_stem) → roles present in the set.
    type SetKey = (String, String);
    let mut sets: HashMap<SetKey, HashSet<String>> = HashMap::new();
    // Anchor each set's issue on the trigger channel's first matching
    // file so clicking the issue takes the user to the most relevant
    // texture in the group.
    let mut trigger_path_per_set: HashMap<SetKey, String> = HashMap::new();

    let trigger = config.trigger.to_lowercase();

    for asset in assets {
        if !matches!(asset.asset_type, AssetType::Texture) {
            continue;
        }
        let dir = Path::new(&asset.path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        let stem_owned: String = match Path::new(&asset.name)
            .file_stem()
            .and_then(|s| s.to_str())
        {
            Some(s) => s.to_string(),
            None => continue,
        };
        let parsed = match parse_stem(&stem_owned, &config.channels, &config.packed) {
            Some(p) => p,
            None => continue,
        };
        let (base_stem, role) = parsed;
        let key = (dir, base_stem);
        let entry = sets.entry(key.clone()).or_insert_with(HashSet::new);

        match role {
            RoleKind::Channel(r) => {
                if r == trigger {
                    trigger_path_per_set
                        .entry(key.clone())
                        .or_insert_with(|| asset.path.clone());
                }
                entry.insert(r);
            }
            RoleKind::Packed(packed_key) => {
                if let Some(roles) = config.packed.get(&packed_key) {
                    for r in roles {
                        entry.insert(r.to_lowercase());
                    }
                }
            }
        }
    }

    let required: Vec<String> = config
        .required
        .iter()
        .map(|r| r.to_lowercase())
        .collect();

    // Sort keys so issue order is stable across runs (HashMap iteration
    // is otherwise nondeterministic and would churn the issue list).
    let mut keys: Vec<&SetKey> = sets.keys().collect();
    keys.sort();

    for key in keys {
        let contents = sets.get(key).unwrap();
        if !contents.contains(&trigger) {
            continue;
        }
        let mut missing: Vec<String> = required
            .iter()
            .filter(|r| !contents.contains(*r))
            .cloned()
            .collect();
        if missing.is_empty() {
            continue;
        }
        missing.sort();

        let asset_path = trigger_path_per_set.get(key).cloned().unwrap_or_default();
        let base_stem = &key.1;
        result.add_issue(Issue {
            rule_id: "pbr_set.incomplete".into(),
            rule_name: "Incomplete PBR Set".into(),
            severity: Severity::Warning,
            message: format!(
                "PBR set `{}` is missing channel(s): {}",
                base_stem,
                missing.join(", ")
            ),
            asset_path,
            suggestion: Some(
                "Add the missing texture(s) to the set, or rename so the file no longer ends with a known PBR suffix. To skip a known-incomplete folder, add it to `[ignore].patterns`.".to_string(),
            ),
            auto_fixable: false,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, AssetType};

    fn texture(path: &str) -> AssetInfo {
        let name = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let extension = Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("png")
            .to_string();
        AssetInfo {
            path: path.to_string(),
            name,
            extension,
            asset_type: AssetType::Texture,
            size: 1024,
            metadata: Some(AssetMetadata::default()),
            unity_guid: None,
        }
    }

    #[test]
    fn complete_set_produces_no_issue() {
        let assets = vec![
            texture("/proj/T_Wood_BaseColor.png"),
            texture("/proj/T_Wood_Normal.png"),
        ];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn missing_normal_fires() {
        let assets = vec![texture("/proj/T_Wood_BaseColor.png")];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 1);
        assert!(result.issues[0].message.to_lowercase().contains("normal"));
    }

    #[test]
    fn no_basecolor_means_no_set() {
        // A lone normal map without a BaseColor sibling shouldn't trigger
        // — the user might be storing detail-normal libraries, height
        // lookup textures, etc.
        let assets = vec![texture("/proj/T_Wood_Normal.png")];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn cross_directory_does_not_aggregate() {
        // Same base stem but different directories → two independent sets:
        // /A has BaseColor only → fires; /B has Normal only → no trigger.
        let assets = vec![
            texture("/proj/A/T_Wood_BaseColor.png"),
            texture("/proj/B/T_Wood_Normal.png"),
        ];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 1);
    }

    #[test]
    fn orm_satisfies_packed_roles() {
        // Custom config: require all three of metallic / roughness / ao —
        // satisfied by a single `_ORM` packed map.
        let mut cfg = PbrSetConfig::default();
        cfg.required = vec_str(&[
            "basecolor",
            "normal",
            "roughness",
            "metallic",
            "ao",
        ]);
        let assets = vec![
            texture("/proj/T_Wood_BaseColor.png"),
            texture("/proj/T_Wood_Normal.png"),
            texture("/proj/T_Wood_ORM.png"),
        ];
        let result = find_pbr_set_issues(&assets, &cfg);
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn substring_match_does_not_misfire() {
        // `T_brand_new` must NOT be treated as Normal: the suffix after
        // the last `_` is "new", which isn't a configured channel.
        let assets = vec![texture("/proj/T_brand_new.png")];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn case_insensitive_suffix() {
        let assets = vec![
            texture("/proj/T_Wood_BASECOLOR.png"),
            texture("/proj/T_Wood_normal.png"),
        ];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn disabled_yields_nothing() {
        let mut cfg = PbrSetConfig::default();
        cfg.enabled = false;
        let assets = vec![texture("/proj/T_Wood_BaseColor.png")];
        let result = find_pbr_set_issues(&assets, &cfg);
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn non_texture_assets_ignored() {
        let mut asset = texture("/proj/T_Wood_BaseColor.fbx");
        asset.asset_type = AssetType::Model;
        let result = find_pbr_set_issues(&[asset], &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }

    #[test]
    fn alias_basecolor_suffix_recognized() {
        // `_Albedo` is in the default basecolor alias list — a set built
        // around `_Albedo` + `_Normal` should be considered complete.
        let assets = vec![
            texture("/proj/T_Stone_Albedo.png"),
            texture("/proj/T_Stone_Normal.png"),
        ];
        let result = find_pbr_set_issues(&assets, &PbrSetConfig::default());
        assert_eq!(result.issue_count, 0);
    }
}
