use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Reference to another Unity asset via GUID
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct UnityReference {
    pub guid: String,
    pub file_id: Option<i64>,
    pub ref_type: Option<i32>,
}

/// Parsed Unity file data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnityFileInfo {
    pub path: String,
    pub file_type: UnityFileType,
    pub references: Vec<UnityReference>,
    pub components: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UnityFileType {
    Prefab,
    Scene,
    Material,
    Controller,
    Asset,
    Unknown,
}

impl UnityFileType {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "prefab" => UnityFileType::Prefab,
            "unity" => UnityFileType::Scene,
            "mat" => UnityFileType::Material,
            "controller" => UnityFileType::Controller,
            "asset" => UnityFileType::Asset,
            _ => UnityFileType::Unknown,
        }
    }
}

/// Parse Unity YAML file and extract references
pub fn parse_unity_file(path: &Path) -> Option<UnityFileInfo> {
    let extension = path.extension()?.to_str()?;
    let file_type = UnityFileType::from_extension(extension);

    if matches!(file_type, UnityFileType::Unknown) {
        return None;
    }

    let content = fs::read_to_string(path).ok()?;

    // Extract all GUID references
    let references = extract_references(&content);

    // Extract component types (for prefabs)
    let components = if matches!(file_type, UnityFileType::Prefab | UnityFileType::Scene) {
        extract_components(&content)
    } else {
        Vec::new()
    };

    Some(UnityFileInfo {
        path: path.to_string_lossy().to_string(),
        file_type,
        references,
        components,
    })
}

/// Extract all GUID references from Unity YAML content
fn extract_references(content: &str) -> Vec<UnityReference> {
    let mut refs = HashSet::new();

    // Pattern: {fileID: xxx, guid: yyy, type: z}
    // Also match: guid: xxx
    for line in content.lines() {
        let line = line.trim();

        // Skip comments
        if line.starts_with('#') || line.starts_with('%') {
            continue;
        }

        // Look for guid patterns
        if let Some(guid_start) = line.find("guid:") {
            let rest = &line[guid_start + 5..].trim_start();

            // Extract the GUID (32 hex chars)
            let guid: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();

            if guid.len() == 32 {
                // Try to extract fileID if present
                let file_id = extract_file_id(line);
                let ref_type = extract_type(line);

                refs.insert(UnityReference {
                    guid,
                    file_id,
                    ref_type,
                });
            }
        }
    }

    refs.into_iter().collect()
}

/// Extract fileID from a line
fn extract_file_id(line: &str) -> Option<i64> {
    if let Some(start) = line.find("fileID:") {
        let rest = &line[start + 7..].trim_start();
        let num_str: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        num_str.parse().ok()
    } else {
        None
    }
}

/// Extract type from a line
fn extract_type(line: &str) -> Option<i32> {
    if let Some(start) = line.find("type:") {
        let rest = &line[start + 5..].trim_start();
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        num_str.parse().ok()
    } else {
        None
    }
}

/// Extract component types from prefab/scene content
fn extract_components(content: &str) -> Vec<String> {
    let mut components = HashSet::new();

    // Look for component markers like "--- !u!xxx" where xxx is the class ID
    // and "m_Script:" references
    for line in content.lines() {
        let line = line.trim();

        // Look for MonoBehaviour components with script references
        if line.starts_with("m_Script:") {
            if let Some(_) = line.find("guid:") {
                components.insert("MonoBehaviour".to_string());
            }
        }

        // Extract Unity built-in component types
        if line.starts_with("---") && line.contains("!u!") {
            if let Some(class_id) = extract_unity_class_id(line) {
                if let Some(name) = unity_class_name(class_id) {
                    components.insert(name.to_string());
                }
            }
        }
    }

    components.into_iter().collect()
}

/// Extract Unity class ID from YAML header
fn extract_unity_class_id(line: &str) -> Option<i32> {
    // Format: --- !u!xxx &yyy
    if let Some(start) = line.find("!u!") {
        let rest = &line[start + 3..];
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        num_str.parse().ok()
    } else {
        None
    }
}

/// Map Unity class ID to human-readable name
fn unity_class_name(class_id: i32) -> Option<&'static str> {
    match class_id {
        1 => Some("GameObject"),
        2 => Some("Component"),
        3 => Some("LevelGameManager"),
        4 => Some("Transform"),
        20 => Some("Camera"),
        21 => Some("Material"),
        23 => Some("MeshRenderer"),
        25 => Some("Renderer"),
        28 => Some("Texture2D"),
        33 => Some("MeshFilter"),
        43 => Some("Mesh"),
        48 => Some("Shader"),
        54 => Some("Rigidbody"),
        56 => Some("Collider"),
        64 => Some("MeshCollider"),
        65 => Some("BoxCollider"),
        82 => Some("AudioSource"),
        83 => Some("AudioClip"),
        84 => Some("RenderTexture"),
        91 => Some("AnimationClip"),
        95 => Some("Animator"),
        102 => Some("TextMesh"),
        104 => Some("RenderSettings"),
        108 => Some("Light"),
        114 => Some("MonoBehaviour"),
        115 => Some("MonoScript"),
        120 => Some("LineRenderer"),
        128 => Some("Font"),
        137 => Some("PhysicMaterial"),
        142 => Some("AssetBundle"),
        150 => Some("PreloadData"),
        156 => Some("Terrain"),
        157 => Some("TerrainCollider"),
        158 => Some("TerrainData"),
        184 => Some("AudioBehaviour"),
        195 => Some("NavMeshAgent"),
        196 => Some("NavMeshSettings"),
        212 => Some("SpriteRenderer"),
        213 => Some("Sprite"),
        221 => Some("AnimatorController"),
        222 => Some("Canvas"),
        223 => Some("CanvasGroup"),
        224 => Some("RectTransform"),
        225 => Some("CanvasRenderer"),
        226 => Some("TextMeshPro"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_guid() {
        let content = r#"
        m_Texture: {fileID: 2800000, guid: abc123def456789012345678901234ab, type: 3}
        "#;
        let refs = extract_references(content);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].guid, "abc123def456789012345678901234ab");
    }

    #[test]
    fn test_file_type() {
        assert_eq!(UnityFileType::from_extension("prefab"), UnityFileType::Prefab);
        assert_eq!(UnityFileType::from_extension("unity"), UnityFileType::Scene);
        assert_eq!(UnityFileType::from_extension("mat"), UnityFileType::Material);
    }
}
