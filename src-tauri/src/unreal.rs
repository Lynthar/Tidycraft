//! Unreal Engine 项目支持模块
//!
//! 解析 .uproject 文件，提取项目配置信息。
//! 为未来完整的 .uasset 解析预留扩展接口。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Unreal 项目配置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealProjectInfo {
    /// 项目文件路径
    pub path: String,
    /// 项目名称（从文件名提取）
    pub project_name: String,
    /// 引擎版本关联（如 "5.3", "5.4"）
    pub engine_association: Option<String>,
    /// 项目类别
    pub category: Option<String>,
    /// 项目描述
    pub description: Option<String>,
    /// 启用的插件列表
    pub plugins: Vec<UnrealPlugin>,
    /// 目标平台
    pub target_platforms: Vec<String>,
    /// 模块列表
    pub modules: Vec<UnrealModule>,
    /// 是否为企业项目
    pub is_enterprise_project: bool,
}

/// Unreal 插件信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealPlugin {
    /// 插件名称
    pub name: String,
    /// 是否启用
    pub enabled: bool,
}

/// Unreal 模块信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnrealModule {
    /// 模块名称
    pub name: String,
    /// 模块类型（Runtime, Editor, etc.）
    pub module_type: String,
    /// 加载阶段
    pub loading_phase: Option<String>,
}

/// .uproject 文件的原始 JSON 结构
#[derive(Debug, Deserialize)]
struct UProjectFile {
    #[serde(rename = "FileVersion")]
    file_version: Option<i32>,
    #[serde(rename = "EngineAssociation")]
    engine_association: Option<String>,
    #[serde(rename = "Category")]
    category: Option<String>,
    #[serde(rename = "Description")]
    description: Option<String>,
    #[serde(rename = "Modules")]
    modules: Option<Vec<UProjectModule>>,
    #[serde(rename = "Plugins")]
    plugins: Option<Vec<UProjectPlugin>>,
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

/// 在项目根目录下查找 .uproject 文件
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

/// 解析 .uproject 文件
pub fn parse_uproject(path: &Path) -> Option<UnrealProjectInfo> {
    let content = fs::read_to_string(path).ok()?;
    let uproject: UProjectFile = serde_json::from_str(&content).ok()?;

    let project_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let plugins = uproject
        .plugins
        .unwrap_or_default()
        .into_iter()
        .map(|p| UnrealPlugin {
            name: p.name,
            enabled: p.enabled,
        })
        .collect();

    let modules = uproject
        .modules
        .unwrap_or_default()
        .into_iter()
        .map(|m| UnrealModule {
            name: m.name,
            module_type: m.module_type,
            loading_phase: m.loading_phase,
        })
        .collect();

    Some(UnrealProjectInfo {
        path: path.to_string_lossy().to_string(),
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

/// 检查路径是否在 Unreal Content 目录中
pub fn is_content_path(path: &Path, project_root: &Path) -> bool {
    let content_dir = project_root.join("Content");
    path.starts_with(&content_dir)
}

/// 获取 Unreal 资源类型（基于扩展名）
/// 预留接口，用于未来扩展 .uasset 解析
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
    use std::io::Write;
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
        assert!(info.plugins.iter().any(|p| p.name == "Paper2D" && p.enabled));
        assert!(info.plugins.iter().any(|p| p.name == "SteamVR" && !p.enabled));

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
        assert_eq!(
            get_unreal_asset_type(Path::new("script.cpp")),
            None
        );
    }
}
