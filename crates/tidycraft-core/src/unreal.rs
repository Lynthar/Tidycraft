//! Unreal Engine project support: parses `.uproject` for project configuration.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Unreal project configuration read from a `.uproject` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealProjectInfo {
    pub path: String,
    /// Taken from the file name.
    pub project_name: String,
    /// Associated engine version, e.g. "5.3".
    pub engine_association: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    pub plugins: Vec<UnrealPlugin>,
    pub target_platforms: Vec<String>,
    pub modules: Vec<UnrealModule>,
    pub is_enterprise_project: bool,
}

/// One plugin entry from a `.uproject`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealPlugin {
    pub name: String,
    pub enabled: bool,
}

/// One module entry from a `.uproject`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealModule {
    pub name: String,
    /// Runtime, Editor, and so on.
    pub module_type: String,
    pub loading_phase: Option<String>,
}

/// Raw JSON shape of a `.uproject` file.
#[derive(Debug, Deserialize)]
struct UProjectFile {
    // Deserialized but not surfaced yet — placeholder for the planned UE
    // deep-integration (format-version-dependent parsing).
    #[allow(dead_code)]
    #[serde(rename = "FileVersion")]
    file_version: Option<i32>,
    #[serde(rename = "EngineAssociation")]
    engine_association: Option<String>,
    #[serde(rename = "Category")]
    category: Option<String>,
    #[serde(rename = "Description")]
    description: Option<String>,
    // Held as raw values and converted one at a time: a single entry missing
    // a required field must cost that entry, not the whole `.uproject`.
    #[serde(rename = "Modules")]
    modules: Option<Vec<serde_json::Value>>,
    #[serde(rename = "Plugins")]
    plugins: Option<Vec<serde_json::Value>>,
    #[serde(rename = "TargetPlatforms")]
    target_platforms: Option<Vec<String>>,
    #[serde(rename = "IsEnterpriseProject")]
    is_enterprise_project: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UProjectModule {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Type")]
    module_type: String,
    #[serde(rename = "LoadingPhase")]
    loading_phase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UProjectPlugin {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Enabled")]
    enabled: bool,
}

/// Find the `.uproject` file in a project root.
pub fn find_uproject_file(root_path: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(root_path).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "uproject" {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// Parse a `.uproject` file.
pub fn parse_uproject(path: &Path) -> Option<UnrealProjectInfo> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[unreal] failed to read {}: {e}", path.display());
            return None;
        }
    };
    let uproject: UProjectFile = match serde_json::from_str(&content) {
        Ok(u) => u,
        Err(e) => {
            // Reaching here removes the whole engine card from the interface, so
            // it must not be silent.
            eprintln!("[unreal] failed to parse {}: {e}", path.display());
            return None;
        }
    };

    let project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let plugins = uproject
        .plugins
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<UProjectPlugin>(v) {
            Ok(p) => Some(UnrealPlugin {
                name: p.name,
                enabled: p.enabled,
            }),
            Err(e) => {
                eprintln!(
                    "[unreal] skipping a Plugins entry in {}: {e}",
                    path.display()
                );
                None
            }
        })
        .collect();

    let modules = uproject
        .modules
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<UProjectModule>(v) {
            Ok(m) => Some(UnrealModule {
                name: m.name,
                module_type: m.module_type,
                loading_phase: m.loading_phase,
            }),
            Err(e) => {
                eprintln!(
                    "[unreal] skipping a Modules entry in {}: {e}",
                    path.display()
                );
                None
            }
        })
        .collect();

    Some(UnrealProjectInfo {
        // Normalized like every other path we hand the frontend — on
        // Windows `to_string_lossy` alone would leak backslashes.
        path: crate::scanner::path_to_string(path),
        project_name,
        engine_association: uproject.engine_association,
        category: uproject.category,
        description: uproject.description,
        plugins,
        target_platforms: uproject.target_platforms.unwrap_or_default(),
        modules,
        is_enterprise_project: uproject.is_enterprise_project.unwrap_or(false),
    })
}

/// Whether `path` sits inside the Unreal `Content` directory. Only tests call it.
#[allow(dead_code)]
pub fn is_content_path(path: &Path, project_root: &Path) -> bool {
    let content_dir = project_root.join("Content");
    path.starts_with(&content_dir)
}

/// Unreal asset type from the file extension. Only tests call it.
#[allow(dead_code)]
pub fn get_unreal_asset_type(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    match ext.to_lowercase().as_str() {
        "uasset" => Some("Asset".to_string()),
        "umap" => Some("Map".to_string()),
        "uplugin" => Some("Plugin".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_find_uproject_file() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("TestProject.uproject");
        fs::write(&uproject_path, "{}").unwrap();

        let found = find_uproject_file(dir.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap().file_name().unwrap(), "TestProject.uproject");
    }

    #[test]
    fn test_find_uproject_file_not_found() {
        let dir = tempdir().unwrap();
        let found = find_uproject_file(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn parse_uproject_path_uses_forward_slashes() {
        // Forward-slash discipline (bites on Windows CI, where the tempdir
        // path itself contains backslashes).
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("Slashes.uproject");
        fs::write(&uproject_path, r#"{"FileVersion": 3}"#).unwrap();

        let info = parse_uproject(&uproject_path).expect("uproject should parse");
        assert!(
            !info.path.contains('\\'),
            "path must be forward-slash normalized: {}",
            info.path
        );
    }

    #[test]
    fn test_parse_uproject_minimal() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("MyGame.uproject");

        let content = r#"{
            "FileVersion": 3,
            "EngineAssociation": "5.3"
        }"#;
        fs::write(&uproject_path, content).unwrap();

        let info = parse_uproject(&uproject_path);
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.project_name, "MyGame");
        assert_eq!(info.engine_association, Some("5.3".to_string()));
        assert!(info.plugins.is_empty());
        assert!(info.modules.is_empty());
    }

    /// One malformed entry in a list must cost that entry, not the project:
    /// `from_str(...).ok()?` made any missing field anywhere return `None` for
    /// the whole file, silently removing the Unreal card from the interface.
    #[test]
    fn one_malformed_plugin_entry_does_not_take_the_project_with_it() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("MyGame.uproject");

        fs::write(
            &uproject_path,
            r#"{
                "FileVersion": 3,
                "EngineAssociation": "5.3",
                "Plugins": [
                    { "Name": "Niagara", "Enabled": true },
                    { "Name": "MissingEnabledField" },
                    { "Name": "Water", "Enabled": false }
                ]
            }"#,
        )
        .unwrap();

        let info = parse_uproject(&uproject_path).expect("the project still parses");
        assert_eq!(info.engine_association, Some("5.3".to_string()));

        let names: Vec<&str> = info.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Niagara", "Water"], "only the bad entry is dropped");
    }

    #[test]
    fn one_malformed_module_entry_does_not_take_the_project_with_it() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("MyGame.uproject");

        fs::write(
            &uproject_path,
            r#"{
                "FileVersion": 3,
                "EngineAssociation": "5.3",
                "Modules": [
                    { "Name": "MyGame", "Type": "Runtime" },
                    { "Name": "NoType" }
                ]
            }"#,
        )
        .unwrap();

        let info = parse_uproject(&uproject_path).expect("the project still parses");
        let names: Vec<&str> = info.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["MyGame"]);
    }

    #[test]
    fn test_parse_uproject_full() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("FullProject.uproject");

        let content = r#"{
            "FileVersion": 3,
            "EngineAssociation": "5.4",
            "Category": "Game",
            "Description": "A test project",
            "Modules": [
                {
                    "Name": "MyModule",
                    "Type": "Runtime",
                    "LoadingPhase": "Default"
                }
            ],
            "Plugins": [
                {
                    "Name": "Paper2D",
                    "Enabled": true
                },
                {
                    "Name": "SteamVR",
                    "Enabled": false
                }
            ],
            "TargetPlatforms": ["Windows", "Linux"],
            "IsEnterpriseProject": false
        }"#;
        fs::write(&uproject_path, content).unwrap();

        let info = parse_uproject(&uproject_path).unwrap();

        assert_eq!(info.project_name, "FullProject");
        assert_eq!(info.engine_association, Some("5.4".to_string()));
        assert_eq!(info.category, Some("Game".to_string()));
        assert_eq!(info.description, Some("A test project".to_string()));

        assert_eq!(info.modules.len(), 1);
        assert_eq!(info.modules[0].name, "MyModule");
        assert_eq!(info.modules[0].module_type, "Runtime");

        assert_eq!(info.plugins.len(), 2);
        assert!(info
            .plugins
            .iter()
            .any(|p| p.name == "Paper2D" && p.enabled));
        assert!(info
            .plugins
            .iter()
            .any(|p| p.name == "SteamVR" && !p.enabled));

        assert_eq!(info.target_platforms, vec!["Windows", "Linux"]);
        assert!(!info.is_enterprise_project);
    }

    #[test]
    fn test_parse_uproject_invalid_json() {
        let dir = tempdir().unwrap();
        let uproject_path = dir.path().join("Invalid.uproject");
        fs::write(&uproject_path, "not valid json").unwrap();

        let info = parse_uproject(&uproject_path);
        assert!(info.is_none());
    }

    #[test]
    fn test_is_content_path() {
        let project_root = Path::new("/game/MyProject");
        let content_file = Path::new("/game/MyProject/Content/Textures/logo.png");
        let source_file = Path::new("/game/MyProject/Source/MyModule/main.cpp");

        assert!(is_content_path(content_file, project_root));
        assert!(!is_content_path(source_file, project_root));
    }

    #[test]
    fn test_get_unreal_asset_type() {
        assert_eq!(
            get_unreal_asset_type(Path::new("texture.uasset")),
            Some("Asset".to_string())
        );
        assert_eq!(
            get_unreal_asset_type(Path::new("level.umap")),
            Some("Map".to_string())
        );
        assert_eq!(get_unreal_asset_type(Path::new("script.cpp")), None);
    }
}
