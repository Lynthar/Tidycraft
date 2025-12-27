use crate::analyzer::{Issue, Severity};
use crate::scanner::{AssetInfo, AssetType};
use serde::{Deserialize, Serialize};

use super::Rule;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    true
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
            enabled: true,
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
                    suggestion: Some("Consider combining materials to reduce draw calls".to_string()),
                    auto_fixable: false,
                });
            }
        }

        None
    }
}
