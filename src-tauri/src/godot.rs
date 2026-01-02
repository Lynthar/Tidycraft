//! Godot 引擎项目支持模块
//!
//! 解析 project.godot 配置文件，提取项目信息。
//! 为未来完整的 .tscn/.tres 解析预留扩展接口。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Godot 项目配置信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GodotProjectInfo {
    /// 配置文件路径
    pub path: String,
    /// 项目名称
    pub project_name: String,
    /// Godot 版本（从 config_version 或 features 推断）
    pub godot_version: Option<String>,
    /// 主场景路径
    pub main_scene: Option<String>,
    /// 图标路径
    pub icon: Option<String>,
    /// 项目特性列表
    pub features: Vec<String>,
    /// 自动加载脚本
    pub autoloads: Vec<GodotAutoload>,
    /// 输入动作名称列表
    pub input_actions: Vec<String>,
    /// 渲染器设置
    pub renderer: Option<String>,
}

/// Godot 自动加载脚本配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GodotAutoload {
    /// 脚本名称（全局变量名）
    pub name: String,
    /// 脚本路径
    pub path: String,
    /// 是否单例
    pub singleton: bool,
}

/// Godot 资源类型（预留扩展）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GodotResourceType {
    Scene,
    Script,
    Texture,
    AudioStream,
    Material,
    Mesh,
    Animation,
    Font,
    Resource,
    Other,
}

/// 解析 project.godot 配置文件
pub fn parse_project_godot(path: &Path) -> Option<GodotProjectInfo> {
    let content = fs::read_to_string(path).ok()?;
    let config = parse_godot_config(&content);

    // 获取 application 部分
    let application = config.get("application").cloned().unwrap_or_default();

    // 提取项目名称
    let project_name = application
        .get("config/name")
        .map(|s| unquote(s))
        .unwrap_or_else(|| "Unknown".to_string());

    // 提取主场景
    let main_scene = application.get("run/main_scene").map(|s| unquote(s));

    // 提取图标
    let icon = application.get("config/icon").map(|s| unquote(s));

    // 提取特性列表
    let features = application
        .get("config/features")
        .map(|s| parse_godot_array(s))
        .unwrap_or_default();

    // 推断 Godot 版本
    let godot_version = infer_godot_version(&config, &features);

    // 提取自动加载
    let autoloads = extract_autoloads(&config);

    // 提取输入动作
    let input_actions = extract_input_actions(&config);

    // 提取渲染器设置
    let renderer = config
        .get("rendering")
        .and_then(|r| r.get("renderer/rendering_method"))
        .or_else(|| {
            config
                .get("rendering")
                .and_then(|r| r.get("quality/driver/driver_name"))
        })
        .map(|s| unquote(s));

    Some(GodotProjectInfo {
        path: path.to_string_lossy().to_string(),
        project_name,
        godot_version,
        main_scene,
        icon,
        features,
        autoloads,
        input_actions,
        renderer,
    })
}

/// 解析 Godot INI 格式的配置文件
fn parse_godot_config(content: &str) -> HashMap<String, HashMap<String, String>> {
    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();

        // 跳过空行和注释
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        // 解析 section header: [section_name]
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            result.entry(current_section.clone()).or_default();
            continue;
        }

        // 解析 key=value
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();

            if let Some(section) = result.get_mut(&current_section) {
                section.insert(key, value);
            } else {
                // 如果没有 section（文件开头），使用空字符串作为 section
                result
                    .entry(String::new())
                    .or_default()
                    .insert(key, value);
            }
        }
    }

    result
}

/// 从配置中提取自动加载信息
fn extract_autoloads(config: &HashMap<String, HashMap<String, String>>) -> Vec<GodotAutoload> {
    let mut autoloads = Vec::new();

    if let Some(autoload_section) = config.get("autoload") {
        for (name, value) in autoload_section {
            // 格式: name="*res://path/to/script.gd" 或 name="res://path/to/script.gd"
            let value = unquote(value);
            let (singleton, path) = if value.starts_with('*') {
                (true, value[1..].to_string())
            } else {
                (false, value)
            };

            autoloads.push(GodotAutoload {
                name: name.clone(),
                path,
                singleton,
            });
        }
    }

    autoloads
}

/// 从配置中提取输入动作
fn extract_input_actions(config: &HashMap<String, HashMap<String, String>>) -> Vec<String> {
    let mut actions = Vec::new();

    if let Some(input_section) = config.get("input") {
        for key in input_section.keys() {
            // 输入动作的键通常是 "action_name" 或带有 deadzone 等后缀
            // 只取动作名称部分
            if !key.contains('/') {
                actions.push(key.clone());
            }
        }
    }

    actions.sort();
    actions.dedup();
    actions
}

/// 从特性列表或配置版本推断 Godot 版本
fn infer_godot_version(
    config: &HashMap<String, HashMap<String, String>>,
    features: &[String],
) -> Option<String> {
    // 首先从 features 中查找版本号
    for feature in features {
        // 版本号通常是 "4.2", "4.3", "3.5" 等格式
        if feature
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            if feature.contains('.') {
                return Some(feature.clone());
            }
        }
    }

    // 从 config_version 推断
    if let Some(gd_resource) = config.get("gd_resource") {
        if let Some(version) = gd_resource.get("config_version") {
            let version_num: i32 = version.parse().unwrap_or(0);
            return match version_num {
                5 => Some("4.x".to_string()),
                4 => Some("3.x".to_string()),
                _ => None,
            };
        }
    }

    // 从空 section 中查找 config_version
    if let Some(root) = config.get("") {
        if let Some(version) = root.get("config_version") {
            let version_num: i32 = version.parse().unwrap_or(0);
            return match version_num {
                5 => Some("4.x".to_string()),
                4 => Some("3.x".to_string()),
                _ => None,
            };
        }
    }

    None
}

/// 解析 Godot 数组格式: PackedStringArray("a", "b", "c") 或 ["a", "b", "c"]
fn parse_godot_array(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let s = s.trim();

    // 处理 PackedStringArray(...) 格式
    let inner = if s.starts_with("PackedStringArray(") && s.ends_with(')') {
        &s[18..s.len() - 1]
    } else if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        return result;
    };

    // 简单的逗号分隔解析
    for item in inner.split(',') {
        let item = item.trim();
        let item = unquote(item);
        if !item.is_empty() {
            result.push(item);
        }
    }

    result
}

/// 去除字符串两端的引号
fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// 根据扩展名获取 Godot 资源类型
/// 预留接口，用于未来扩展
pub fn get_godot_resource_type(path: &Path) -> Option<GodotResourceType> {
    let ext = path.extension()?.to_str()?;
    match ext.to_lowercase().as_str() {
        "tscn" => Some(GodotResourceType::Scene),
        "gd" | "gdscript" => Some(GodotResourceType::Script),
        "tres" => Some(GodotResourceType::Resource),
        "png" | "jpg" | "jpeg" | "webp" | "svg" => Some(GodotResourceType::Texture),
        "ogg" | "wav" | "mp3" => Some(GodotResourceType::AudioStream),
        "material" | "shader" => Some(GodotResourceType::Material),
        "mesh" | "obj" | "gltf" | "glb" => Some(GodotResourceType::Mesh),
        "anim" => Some(GodotResourceType::Animation),
        "ttf" | "otf" | "woff" | "woff2" => Some(GodotResourceType::Font),
        _ => Some(GodotResourceType::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_godot_config() {
        let content = r#"
; This is a comment
[application]
config/name="My Game"
config/description="A test game"
run/main_scene="res://main.tscn"

[autoload]
GameManager="*res://scripts/game_manager.gd"
Utils="res://scripts/utils.gd"

[input]
move_left=null
move_right=null
"#;
        let config = parse_godot_config(content);

        assert!(config.contains_key("application"));
        assert!(config.contains_key("autoload"));
        assert!(config.contains_key("input"));

        let app = config.get("application").unwrap();
        assert_eq!(app.get("config/name"), Some(&"\"My Game\"".to_string()));
        assert_eq!(
            app.get("run/main_scene"),
            Some(&"\"res://main.tscn\"".to_string())
        );
    }

    #[test]
    fn test_parse_project_godot() {
        let dir = tempdir().unwrap();
        let project_path = dir.path().join("project.godot");

        let content = r#"
; Engine configuration file.

config_version=5

[application]

config/name="Test Project"
config/features=PackedStringArray("4.2", "Forward Plus")
run/main_scene="res://scenes/main.tscn"
config/icon="res://icon.svg"

[autoload]

GameState="*res://autoload/game_state.gd"

[input]

jump=null
attack=null

[rendering]

renderer/rendering_method="forward_plus"
"#;
        fs::write(&project_path, content).unwrap();

        let info = parse_project_godot(&project_path).unwrap();

        assert_eq!(info.project_name, "Test Project");
        assert_eq!(info.godot_version, Some("4.2".to_string()));
        assert_eq!(info.main_scene, Some("res://scenes/main.tscn".to_string()));
        assert_eq!(info.icon, Some("res://icon.svg".to_string()));
        assert!(info.features.contains(&"4.2".to_string()));
        assert!(info.features.contains(&"Forward Plus".to_string()));

        assert_eq!(info.autoloads.len(), 1);
        assert_eq!(info.autoloads[0].name, "GameState");
        assert!(info.autoloads[0].singleton);

        assert!(info.input_actions.contains(&"jump".to_string()));
        assert!(info.input_actions.contains(&"attack".to_string()));

        assert_eq!(info.renderer, Some("forward_plus".to_string()));
    }

    #[test]
    fn test_parse_project_godot_minimal() {
        let dir = tempdir().unwrap();
        let project_path = dir.path().join("project.godot");

        let content = r#"
config_version=5

[application]
config/name="Minimal"
"#;
        fs::write(&project_path, content).unwrap();

        let info = parse_project_godot(&project_path).unwrap();

        assert_eq!(info.project_name, "Minimal");
        assert!(info.main_scene.is_none());
        assert!(info.autoloads.is_empty());
        assert!(info.input_actions.is_empty());
    }

    #[test]
    fn test_parse_godot_array() {
        let packed = r#"PackedStringArray("4.2", "Forward Plus", "GL Compatibility")"#;
        let result = parse_godot_array(packed);
        assert_eq!(result, vec!["4.2", "Forward Plus", "GL Compatibility"]);

        let bracket = r#"["a", "b", "c"]"#;
        let result = parse_godot_array(bracket);
        assert_eq!(result, vec!["a", "b", "c"]);

        let empty = r#"PackedStringArray()"#;
        let result = parse_godot_array(empty);
        assert!(result.is_empty());
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("'world'"), "world");
        assert_eq!(unquote("no quotes"), "no quotes");
        assert_eq!(unquote("  \"spaced\"  "), "spaced");
    }

    #[test]
    fn test_extract_autoloads() {
        let mut config = HashMap::new();
        let mut autoload = HashMap::new();
        autoload.insert(
            "Singleton".to_string(),
            "\"*res://singleton.gd\"".to_string(),
        );
        autoload.insert("Helper".to_string(), "\"res://helper.gd\"".to_string());
        config.insert("autoload".to_string(), autoload);

        let result = extract_autoloads(&config);

        assert_eq!(result.len(), 2);

        let singleton = result.iter().find(|a| a.name == "Singleton").unwrap();
        assert!(singleton.singleton);
        assert_eq!(singleton.path, "res://singleton.gd");

        let helper = result.iter().find(|a| a.name == "Helper").unwrap();
        assert!(!helper.singleton);
        assert_eq!(helper.path, "res://helper.gd");
    }

    #[test]
    fn test_infer_godot_version_from_features() {
        let config = HashMap::new();
        let features = vec!["4.3".to_string(), "Forward Plus".to_string()];

        let version = infer_godot_version(&config, &features);
        assert_eq!(version, Some("4.3".to_string()));
    }

    #[test]
    fn test_infer_godot_version_from_config() {
        let mut config = HashMap::new();
        let mut root = HashMap::new();
        root.insert("config_version".to_string(), "5".to_string());
        config.insert(String::new(), root);

        let version = infer_godot_version(&config, &[]);
        assert_eq!(version, Some("4.x".to_string()));
    }

    #[test]
    fn test_get_godot_resource_type() {
        assert_eq!(
            get_godot_resource_type(Path::new("main.tscn")),
            Some(GodotResourceType::Scene)
        );
        assert_eq!(
            get_godot_resource_type(Path::new("player.gd")),
            Some(GodotResourceType::Script)
        );
        assert_eq!(
            get_godot_resource_type(Path::new("logo.png")),
            Some(GodotResourceType::Texture)
        );
        assert_eq!(
            get_godot_resource_type(Path::new("bgm.ogg")),
            Some(GodotResourceType::AudioStream)
        );
    }
}
