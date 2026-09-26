use crate::analyzer::{issue_args, Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Maximum vertex count before warning
    #[serde(default = "default_max_vertices")]
    pub max_vertices: u32,

    /// Maximum face count before warning
    #[serde(default = "default_max_faces")]
    pub max_faces: u32,

    /// Maximum material count
    #[serde(default = "default_max_materials")]
    pub max_materials: u32,
}

fn default_enabled() -> bool {
    // Out-of-box OFF: vertex / face / material limits are pipeline-
    // specific budgets. Users opt in via tidycraft.toml.
    false
}

fn default_max_vertices() -> u32 {
    100_000
}

fn default_max_faces() -> u32 {
    100_000
}

fn default_max_materials() -> u32 {
    10
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_vertices: 100_000,
            max_faces: 100_000,
            max_materials: 10,
        }
    }
}

pub struct ModelRule {
    config: ModelConfig,
}

impl ModelRule {
    pub fn new(config: ModelConfig) -> Self {
        Self { config }
    }
}

impl Rule for ModelRule {
    fn id(&self) -> &str {
        "model"
    }

    fn name(&self) -> &str {
        "Model Standards"
    }

    fn applies_to(&self, asset: &AssetInfo) -> bool {
        matches!(asset.asset_type, AssetType::Model)
    }

    fn check(&self, asset: &AssetInfo) -> Option<Issue> {
        let metadata = asset.metadata.as_ref()?;

        // Check vertex count
        if let Some(vertex_count) = metadata.vertex_count {
            if vertex_count > self.config.max_vertices {
                return Some(Issue {
                    rule_id: "model.vertices".to_string(),
                    rule_name: "High Vertex Count".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Model has {} vertices, maximum recommended is {}",
                        vertex_count, self.config.max_vertices
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some("Consider reducing polygon count or using LODs".to_string()),
                    auto_fixable: false,
                    related_paths: None,
                    // `vertex_count` not `count`: i18next treats a `count`
                    // interpolation value as a plural selector and looks for
                    // `key_one` / `key_other`, which we do not ship.
                    args: issue_args([
                        ("vertex_count", vertex_count.to_string()),
                        ("max", self.config.max_vertices.to_string()),
                    ]),
                });
            }
        }

        // Check face count
        if let Some(face_count) = metadata.face_count {
            if face_count > self.config.max_faces {
                return Some(Issue {
                    rule_id: "model.faces".to_string(),
                    rule_name: "High Face Count".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Model has {} faces, maximum recommended is {}",
                        face_count, self.config.max_faces
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some("Consider reducing polygon count or using LODs".to_string()),
                    auto_fixable: false,
                    related_paths: None,
                    args: issue_args([
                        ("face_count", face_count.to_string()),
                        ("max", self.config.max_faces.to_string()),
                    ]),
                });
            }
        }

        // Check material count
        if let Some(material_count) = metadata.material_count {
            if material_count > self.config.max_materials {
                return Some(Issue {
                    rule_id: "model.materials".to_string(),
                    rule_name: "Too Many Materials".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Model has {} materials, maximum recommended is {}",
                        material_count, self.config.max_materials
                    ),
                    asset_path: asset.path.clone(),
                    suggestion: Some(
                        "Consider combining materials to reduce draw calls".to_string(),
                    ),
                    auto_fixable: false,
                    related_paths: None,
                    args: issue_args([
                        ("material_count", material_count.to_string()),
                        ("max", self.config.max_materials.to_string()),
                    ]),
                });
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::AssetMetadata;

    fn model(vertex_count: u32, face_count: u32, material_count: u32) -> AssetInfo {
        AssetInfo {
            path: "/p/SM_Rock.fbx".to_string(),
            name: "SM_Rock.fbx".to_string(),
            extension: "fbx".to_string(),
            asset_type: AssetType::Model,
            size: 1024,
            modified: 0,
            metadata: Some(AssetMetadata {
                vertex_count: Some(vertex_count),
                face_count: Some(face_count),
                material_count: Some(material_count),
                ..Default::default()
            }),
            unity_guid: None,
        }
    }

    fn rule_id(asset: &AssetInfo) -> Option<String> {
        ModelRule::new(ModelConfig::default())
            .check(asset)
            .map(|i| i.rule_id)
    }

    /// "Max vertices 100,000 / faces 100,000 / materials 10" all include the limit.
    #[test]
    fn limits_are_inclusive() {
        assert_eq!(rule_id(&model(100_000, 100_000, 10)), None);
        assert_eq!(
            rule_id(&model(100_001, 1, 1)),
            Some("model.vertices".into())
        );
        assert_eq!(rule_id(&model(1, 100_001, 1)), Some("model.faces".into()));
        assert_eq!(rule_id(&model(1, 1, 11)), Some("model.materials".into()));
    }

    #[test]
    fn the_model_rule_only_applies_to_models() {
        let rule = ModelRule::new(ModelConfig::default());
        assert!(rule.applies_to(&model(1, 1, 1)));
        let texture = AssetInfo {
            asset_type: AssetType::Texture,
            ..model(1, 1, 1)
        };
        assert!(!rule.applies_to(&texture));
    }
}
