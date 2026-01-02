use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A tag that can be assigned to assets
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
}

/// Tags storage - persisted to a JSON file in the project root
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TagsData {
    /// All defined tags
    pub tags: Vec<Tag>,
    /// Mapping from asset path to list of tag IDs
    pub asset_tags: HashMap<String, Vec<String>>,
}

const TAGS_FILE: &str = ".tidycraft-tags.json";

impl TagsData {
    /// Load tags from the project directory
    pub fn load(project_path: &Path) -> Self {
        let tags_file = project_path.join(TAGS_FILE);
        if tags_file.exists() {
            if let Ok(content) = fs::read_to_string(&tags_file) {
                if let Ok(data) = serde_json::from_str(&content) {
                    return data;
                }
            }
        }
        Self::default()
    }

    /// Save tags to the project directory
    pub fn save(&self, project_path: &Path) -> Result<(), String> {
        let tags_file = project_path.join(TAGS_FILE);
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&tags_file, content).map_err(|e| e.to_string())
    }

    /// Create a new tag
    pub fn create_tag(&mut self, name: String, color: String) -> Tag {
        let id = uuid::Uuid::new_v4().to_string();
        let tag = Tag { id, name, color };
        self.tags.push(tag.clone());
        tag
    }

    /// Delete a tag and remove it from all assets
    pub fn delete_tag(&mut self, tag_id: &str) {
        self.tags.retain(|t| t.id != tag_id);
        for tags in self.asset_tags.values_mut() {
            tags.retain(|id| id != tag_id);
        }
    }

    /// Update a tag
    pub fn update_tag(&mut self, tag_id: &str, name: Option<String>, color: Option<String>) -> Option<Tag> {
        if let Some(tag) = self.tags.iter_mut().find(|t| t.id == tag_id) {
            if let Some(n) = name {
                tag.name = n;
            }
            if let Some(c) = color {
                tag.color = c;
            }
            return Some(tag.clone());
        }
        None
    }

    /// Add a tag to an asset
    pub fn add_tag_to_asset(&mut self, asset_path: &str, tag_id: &str) {
        // Verify tag exists
        if !self.tags.iter().any(|t| t.id == tag_id) {
            return;
        }

        let tags = self.asset_tags.entry(asset_path.to_string()).or_default();
        if !tags.contains(&tag_id.to_string()) {
            tags.push(tag_id.to_string());
        }
    }

    /// Remove a tag from an asset
    pub fn remove_tag_from_asset(&mut self, asset_path: &str, tag_id: &str) {
        if let Some(tags) = self.asset_tags.get_mut(asset_path) {
            tags.retain(|id| id != tag_id);
        }
    }

    /// Get tags for an asset
    pub fn get_asset_tags(&self, asset_path: &str) -> Vec<Tag> {
        if let Some(tag_ids) = self.asset_tags.get(asset_path) {
            tag_ids
                .iter()
                .filter_map(|id| self.tags.iter().find(|t| &t.id == id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all assets with a specific tag
    pub fn get_assets_with_tag(&self, tag_id: &str) -> Vec<String> {
        self.asset_tags
            .iter()
            .filter(|(_, tags)| tags.contains(&tag_id.to_string()))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_tag() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Important".to_string(), "#ff0000".to_string());

        assert_eq!(tag.name, "Important");
        assert_eq!(tag.color, "#ff0000");
        assert_eq!(data.tags.len(), 1);
    }

    #[test]
    fn test_add_remove_tag_from_asset() {
        let mut data = TagsData::default();
        let tag = data.create_tag("Test".to_string(), "#00ff00".to_string());

        data.add_tag_to_asset("/path/to/asset.png", &tag.id);
        assert_eq!(data.get_asset_tags("/path/to/asset.png").len(), 1);

        data.remove_tag_from_asset("/path/to/asset.png", &tag.id);
        assert_eq!(data.get_asset_tags("/path/to/asset.png").len(), 0);
    }
}
