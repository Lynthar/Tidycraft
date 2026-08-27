//! Godot project support: parses `project.godot` for project information.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::scanner::AssetInfo;

/// Godot project configuration read from `project.godot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GodotProjectInfo {
    pub path: String,
    pub project_name: String,
    /// Inferred from `config_version` or the features list.
    pub godot_version: Option<String>,
    pub main_scene: Option<String>,
    pub icon: Option<String>,
    pub features: Vec<String>,
    pub autoloads: Vec<GodotAutoload>,
    pub input_actions: Vec<String>,
    pub renderer: Option<String>,
}

/// One autoload entry from `project.godot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GodotAutoload {
    /// The global variable name the script is bound to.
    pub name: String,
    pub path: String,
    pub singleton: bool,
}

/// Godot resource type. Only tests use it.
#[allow(dead_code)]
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

/// Parse `project.godot`.
pub fn parse_project_godot(path: &Path) -> Option<GodotProjectInfo> {
    let content = fs::read_to_string(path).ok()?;
    let config = parse_godot_config(&content);

    let application = config.get("application").cloned().unwrap_or_default();

    let project_name = application
        .get("config/name")
        .map(|s| unquote(s))
        .unwrap_or_else(|| "Unknown".to_string());

    let main_scene = application.get("run/main_scene").map(|s| unquote(s));

    let icon = application.get("config/icon").map(|s| unquote(s));

    let features = application
        .get("config/features")
        .map(|s| parse_godot_array(s))
        .unwrap_or_default();

    let godot_version = infer_godot_version(&config, &features);

    let autoloads = extract_autoloads(&config);

    let input_actions = extract_input_actions(&config);

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
        // Normalized like every other path we hand the frontend — on
        // Windows `to_string_lossy` alone would leak backslashes.
        path: crate::scanner::path_to_string(path),
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

/// Parse Godot's INI-shaped config into `section -> key -> value`.
fn parse_godot_config(content: &str) -> HashMap<String, HashMap<String, String>> {
    let mut result: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        // Section header: [section_name]
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            result.entry(current_section.clone()).or_default();
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();

            if let Some(section) = result.get_mut(&current_section) {
                section.insert(key, value);
            } else {
                // Keys before the first header use an empty section name.
                result.entry(String::new()).or_default().insert(key, value);
            }
        }
    }

    result
}

/// Extract the autoload entries from a parsed config.
fn extract_autoloads(config: &HashMap<String, HashMap<String, String>>) -> Vec<GodotAutoload> {
    let mut autoloads = Vec::new();

    if let Some(autoload_section) = config.get("autoload") {
        for (name, value) in autoload_section {
            // Shape: name="*res://path/to/script.gd" or name="res://path/to/script.gd"
            let value = unquote(value);
            let (singleton, path) = match value.strip_prefix('*') {
                Some(rest) => (true, rest.to_string()),
                None => (false, value),
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

/// Extract the input action names from a parsed config.
fn extract_input_actions(config: &HashMap<String, HashMap<String, String>>) -> Vec<String> {
    let mut actions = Vec::new();

    if let Some(input_section) = config.get("input") {
        for key in input_section.keys() {
            // Input action keys are the action name, sometimes with a suffix
            // such as deadzone; keep only the name.
            if !key.contains('/') {
                actions.push(key.clone());
            }
        }
    }

    actions.sort();
    actions.dedup();
    actions
}

/// Infer the Godot version from the features list or `config_version`.
fn infer_godot_version(
    config: &HashMap<String, HashMap<String, String>>,
    features: &[String],
) -> Option<String> {
    // Prefer a version number in `features`.
    for feature in features {
        // Shaped like "4.2", "4.3", "3.5".
        if feature
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            && feature.contains('.')
        {
            return Some(feature.clone());
        }
    }

    // Fall back to config_version.
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

    // config_version lives in the unnamed leading section.
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

/// Parse Godot's array literals: `PackedStringArray("a", "b")` or `["a", "b"]`.
fn parse_godot_array(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let s = s.trim();

    let inner = if s.starts_with("PackedStringArray(") && s.ends_with(')') {
        &s[18..s.len() - 1]
    } else if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        return result;
    };

    for item in inner.split(',') {
        let item = item.trim();
        let item = unquote(item);
        if !item.is_empty() {
            result.push(item);
        }
    }

    result
}

/// Strip matching quotes from both ends of a value.
fn unquote(s: &str) -> String {
    let s = s.trim();
    // The length guard is load-bearing: on a lone `"` both starts_with and
    // ends_with match the same character, and the slice below would be `1..0`.
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Godot resource type from the file extension. Only tests call it.
#[allow(dead_code)]
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

// ============ Unused-asset detection ============

/// Whether `extension` is Godot metadata rather than a real asset: `.import`,
/// `.uid`, `.godot`, `.cfg`. Excluded from unused-asset reporting. Answers on
/// its own rather than depending on the scanner's sidecar filter.
pub fn is_godot_metadata(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "import" | "uid" | "godot" | "cfg"
    )
}

/// Convert an absolute asset path to its Godot `res://` form (project root =
/// `res://`). `None` for paths outside the root (not res://-addressable).
/// Forward-slashed to match the paths written into scene files.
pub fn asset_to_res_path(abs: &str, root: &Path) -> Option<String> {
    let rel = Path::new(abs).strip_prefix(root).ok()?;
    Some(format!(
        "res://{}",
        rel.to_string_lossy().replace('\\', "/")
    ))
}

/// Inverse of `asset_to_res_path`: the absolute filesystem path a `res://`
/// reference points at under `root`. `None` for non-`res://` strings
/// (`user://`, `uid://`, plain paths) and for the bare `res://` root.
pub fn res_path_to_abs(res: &str, root: &Path) -> Option<std::path::PathBuf> {
    let rel = res.strip_prefix("res://")?;
    if rel.is_empty() {
        return None;
    }
    Some(root.join(rel))
}

/// Pull every `res://` reference out of a scene, resource or script's text. All
/// such refs sit inside double quotes, so one quoted-`res://` scan catches them.
/// `uid://`-only refs are not matched — a known gap.
fn extract_res_references(content: &str, re: &regex::Regex) -> Vec<String> {
    re.captures_iter(content)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Comparison key for a `res://` path, normalizing NFD (macOS directory
/// listings) and NFC (editor-written scene files) to one form. Only comparisons
/// are keyed this way; the raw reference string keeps flowing downstream.
fn res_key(res: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    res.nfc().collect()
}

/// Find assets no scene, resource or script references, mirroring the Unity
/// check. Returns absolute paths. Known false-positive sources: `load(variable)`
/// dynamic paths, `uid://`-only refs, and hand-written relative paths.
pub fn find_unused_godot_assets(root_path: &str, assets: &[AssetInfo]) -> Vec<String> {
    let root = Path::new(root_path);
    let mut referenced: HashSet<String> = HashSet::new();

    // All res:// refs sit inside double quotes, so one quoted-res:// regex
    // catches ext_resource paths, preload/load literals, C# GD.Load strings,
    // and project.godot's resource keys alike.
    let re = regex::Regex::new(r#""(res://[^"]*)""#).expect("static regex compiles");

    // 1. Entry points from project.godot — used roots even when nothing else
    //    references them. Autoloads carry a leading `*` inside the quotes, which
    //    the quoted-res:// scan cannot match, so parse those explicitly.
    if let Ok(content) = fs::read_to_string(root.join("project.godot")) {
        for r in extract_res_references(&content, &re) {
            referenced.insert(res_key(&r));
        }
        for autoload in extract_autoloads(&parse_godot_config(&content)) {
            referenced.insert(res_key(&autoload.path));
        }
    }

    // 2. Every res:// path referenced by a scene / resource / script / C# file.
    //    `.cs` is included so Mono projects' GD.Load("res://…") references count.
    for asset in assets {
        let ext = asset.extension.to_lowercase();
        if ext == "tscn" || ext == "tres" || ext == "gd" || ext == "cs" {
            if let Ok(content) = fs::read_to_string(&asset.path) {
                for r in extract_res_references(&content, &re) {
                    referenced.insert(res_key(&r));
                }
            }
        }
    }

    // 3. Assets nobody referenced, skipping Godot metadata and scenes. Scenes
    //    are graph roots, so "no incoming reference" does not make one unused;
    //    they still count as reference sources in step 2.
    assets
        .iter()
        .filter(|a| !is_godot_metadata(&a.extension))
        .filter(|a| !matches!(a.asset_type, crate::scanner::AssetType::Scene))
        .filter(|a| match asset_to_res_path(&a.path, root) {
            Some(res) => !referenced.contains(&res_key(&res)),
            None => false, // outside the project root — not res://-addressable
        })
        .map(|a| a.path.clone())
        .collect()
}

/// Rename guardrail: for each absolute target path, the project files that
/// reference it by `res://` path. Sources are the unused-asset scan's set plus
/// `project.godot`. Self-references are skipped; unreferenced targets are absent.
pub fn referencing_files(
    root: &Path,
    assets: &[AssetInfo],
    targets: &[String],
) -> HashMap<String, Vec<String>> {
    let re = regex::Regex::new(r#""(res://[^"]*)""#).expect("static regex compiles");

    // res:// form of each requested target → its original absolute key
    // (the frontend looks results up by the exact string it sent).
    let mut target_by_res: HashMap<String, String> = HashMap::new();
    for t in targets {
        if let Some(res) = asset_to_res_path(t, root) {
            target_by_res.insert(res_key(&res), t.clone());
        }
    }
    if target_by_res.is_empty() {
        return HashMap::new();
    }

    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    let mut record = |source_rel: &str, refs: HashSet<String>| {
        for res in refs {
            if let Some(abs) = target_by_res.get(&res_key(&res)) {
                result
                    .entry(abs.clone())
                    .or_default()
                    .push(source_rel.to_string());
            }
        }
    };

    // project.godot: quoted res:// values (main scene / icon / splash / ...)
    // plus autoloads, whose leading `*` hides them from the quoted scan.
    if let Ok(content) = fs::read_to_string(root.join("project.godot")) {
        let mut refs: HashSet<String> = extract_res_references(&content, &re).into_iter().collect();
        for autoload in extract_autoloads(&parse_godot_config(&content)) {
            refs.insert(autoload.path);
        }
        record("project.godot", refs);
    }

    for asset in assets {
        let ext = asset.extension.to_lowercase();
        if ext == "tscn" || ext == "tres" || ext == "gd" || ext == "cs" {
            let Ok(content) = fs::read_to_string(&asset.path) else {
                continue;
            };
            let own_res = asset_to_res_path(&asset.path, root);
            let refs: HashSet<String> = extract_res_references(&content, &re)
                .into_iter()
                .filter(|r| Some(r) != own_res.as_ref())
                .collect();
            let source_rel = Path::new(&asset.path)
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| asset.name.clone());
            record(&source_rel, refs);
        }
    }

    result
}

/// Build the raw `(from_res, to_res)` dependency edges: each scene / resource /
/// script → every `res://` resource it references. The caller filters `to` down
/// to known nodes. uid-only refs are likewise not captured.
pub fn godot_dependency_edges(root: &Path, assets: &[AssetInfo]) -> Vec<(String, String)> {
    let re = regex::Regex::new(r#""(res://[^"]*)""#).expect("static regex compiles");
    let mut edges = Vec::new();
    for asset in assets {
        let ext = asset.extension.to_lowercase();
        if ext == "tscn" || ext == "tres" || ext == "gd" || ext == "cs" {
            let Some(from) = asset_to_res_path(&asset.path, root) else {
                continue;
            };
            if let Ok(content) = fs::read_to_string(&asset.path) {
                for to in extract_res_references(&content, &re) {
                    edges.push((from.clone(), to));
                }
            }
        }
    }
    edges
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
    fn parse_project_godot_path_uses_forward_slashes() {
        // Forward-slash discipline (bites on Windows CI, where the tempdir
        // path itself contains backslashes).
        let dir = tempdir().unwrap();
        let project_path = dir.path().join("project.godot");
        std::fs::write(&project_path, "[application]\nconfig/name=\"G\"\n").unwrap();

        let info = parse_project_godot(&project_path).expect("project.godot should parse");
        assert!(
            !info.path.contains('\\'),
            "path must be forward-slash normalized: {}",
            info.path
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

    /// A half-typed `config/name="` yields a value of one lone quote, where
    /// starts_with and ends_with match the same character. Slicing `1..0` there
    /// panics, and release builds are `panic = "abort"`.
    #[test]
    fn unquote_tolerates_a_lone_quote_character() {
        assert_eq!(unquote("\""), "\"");
        assert_eq!(unquote("'"), "'");
        assert_eq!(unquote("  \"  "), "\"");
    }

    /// End-to-end guard for the same defect: the truncated line must parse,
    /// not abort the process.
    #[test]
    fn parse_project_godot_survives_a_truncated_value() {
        let dir = tempfile::tempdir().unwrap();
        let project_path = dir.path().join("project.godot");
        std::fs::write(&project_path, "[application]\nconfig/name=\"\n").unwrap();

        let info = parse_project_godot(&project_path).expect("truncated value should still parse");
        assert_eq!(info.project_name, "\"");
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

    #[test]
    fn test_find_unused_godot_assets() {
        use crate::scanner::AssetType;
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"res://main.tscn\"\n",
        )
        .unwrap();
        fs::write(
            root.join("main.tscn"),
            "[gd_scene format=3]\n[ext_resource type=\"Texture2D\" path=\"res://hero.png\" id=\"1\"]\n",
        )
        .unwrap();
        fs::write(root.join("hero.png"), "x").unwrap();
        fs::write(root.join("orphan.png"), "x").unwrap();

        let mk = |name: &str, ext: &str| AssetInfo {
            path: root.join(name).to_string_lossy().to_string(),
            name: name.to_string(),
            extension: ext.to_string(),
            asset_type: AssetType::Other,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        };
        let assets = vec![
            mk("main.tscn", "tscn"),
            mk("hero.png", "png"),
            mk("orphan.png", "png"),
            AssetInfo {
                asset_type: AssetType::Scene,
                ..mk("level_2.tscn", "tscn")
            },
        ];

        let unused = find_unused_godot_assets(&root.to_string_lossy(), &assets);
        // orphan.png is referenced by nobody -> unused.
        assert!(unused.iter().any(|p| p.ends_with("orphan.png")));
        // hero.png is referenced by main.tscn -> not unused.
        assert!(!unused.iter().any(|p| p.ends_with("hero.png")));
        // main.tscn is the entry point -> not unused.
        assert!(!unused.iter().any(|p| p.ends_with("main.tscn")));
        // level_2.tscn is a scene nobody references, but scenes are graph
        // roots (loaded dynamically / from the editor), so it must NOT be
        // flagged unused.
        assert!(!unused.iter().any(|p| p.ends_with("level_2.tscn")));
    }

    /// macOS writes decomposed (NFD) file names to disk in routine situations,
    /// while the Godot editor writes the composed (NFC) form into scene files.
    /// The two are one name to the user and two strings to a `HashSet`.
    #[test]
    fn nfd_disk_names_match_nfc_scene_references() {
        use crate::scanner::AssetType;
        let dir = tempdir().unwrap();
        let root = dir.path();

        // "café.png": decomposed on disk (e + U+0301), composed (U+00E9) in
        // the scene file — exactly what a macOS checkout of an editor-written
        // project looks like.
        let nfd = "cafe\u{0301}.png";
        let nfc = "caf\u{e9}.png";
        assert_ne!(nfd, nfc, "the two forms must differ as strings");

        fs::write(
            root.join("main.tscn"),
            format!("[ext_resource type=\"Texture2D\" path=\"res://{nfc}\" id=\"1\"]\n"),
        )
        .unwrap();
        fs::write(root.join(nfd), "x").unwrap();

        let mk = |name: &str, ext: &str| AssetInfo {
            path: root.join(name).to_string_lossy().to_string(),
            name: name.to_string(),
            extension: ext.to_string(),
            asset_type: AssetType::Other,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        };
        let assets = vec![mk("main.tscn", "tscn"), mk(nfd, "png")];

        let unused = find_unused_godot_assets(&root.to_string_lossy(), &assets);
        assert!(
            !unused.iter().any(|p| p.ends_with(nfd)),
            "the NFD file is referenced by main.tscn in NFC form: {unused:?}"
        );

        let target = root.join(nfd).to_string_lossy().to_string();
        let refs = referencing_files(root, &assets, std::slice::from_ref(&target));
        assert_eq!(
            refs.get(&target).map(Vec::as_slice),
            Some(["main.tscn".to_string()].as_slice()),
            "the rename guard must see main.tscn's reference"
        );
    }

    /// Both Godot consumers read `project.godot` straight off disk rather than
    /// from the scan's asset list. Turning that into an asset-list lookup would
    /// silently make every autoload unused and every autoload rename unguarded.
    #[test]
    fn project_godot_is_a_reference_source_outside_the_asset_list() {
        use crate::scanner::AssetType;
        let dir = tempdir().unwrap();
        let root = dir.path();

        fs::write(
            root.join("project.godot"),
            "[application]\n\
             config/name=\"G\"\n\
             run/main_scene=\"res://main.tscn\"\n\
             \n\
             [autoload]\n\
             GameState=\"*res://autoload/game_state.gd\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("autoload")).unwrap();
        fs::write(root.join("autoload").join("game_state.gd"), "extends Node").unwrap();
        fs::write(root.join("main.tscn"), "[gd_scene]\n").unwrap();

        let mk = |rel: &Path, ext: &str, asset_type: AssetType| AssetInfo {
            path: root.join(rel).to_string_lossy().to_string(),
            name: rel.file_name().unwrap().to_string_lossy().to_string(),
            extension: ext.to_string(),
            asset_type,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        };

        // Without project.godot: this is the list a scan hands over, and the
        // references must survive its absence.
        let script = Path::new("autoload").join("game_state.gd");
        let assets = vec![
            mk(&script, "gd", AssetType::Script),
            mk(Path::new("main.tscn"), "tscn", AssetType::Scene),
        ];

        let unused = find_unused_godot_assets(&root.to_string_lossy(), &assets);
        assert!(
            unused.is_empty(),
            "the autoload is referenced by project.godot: {unused:?}"
        );

        let target = root.join(&script).to_string_lossy().to_string();
        let refs = referencing_files(root, &assets, std::slice::from_ref(&target));
        assert_eq!(
            refs.get(&target).map(Vec::as_slice),
            Some(["project.godot".to_string()].as_slice()),
            "the rename guard must attribute the autoload reference to project.godot"
        );
    }

    #[test]
    fn test_res_path_to_abs() {
        let root = Path::new("/proj");
        assert_eq!(
            res_path_to_abs("res://textures/hero.png", root),
            Some(root.join("textures/hero.png"))
        );
        // Non-res schemes and the bare root resolve to nothing.
        assert_eq!(res_path_to_abs("res://", root), None);
        assert_eq!(res_path_to_abs("user://save.dat", root), None);
        assert_eq!(res_path_to_abs("uid://c3xyz", root), None);
        // Round-trips with asset_to_res_path.
        let abs = root.join("a/b.png");
        let res = asset_to_res_path(&abs.to_string_lossy(), root).unwrap();
        assert_eq!(res_path_to_abs(&res, root), Some(abs));
    }

    #[test]
    fn test_referencing_files_for_rename_guardrail() {
        use crate::scanner::AssetType;
        let dir = tempdir().unwrap();
        let root = dir.path();

        // project.godot references main.tscn twice over (main scene + a
        // *-prefixed autoload path — the quoted scan alone misses the latter).
        fs::write(
            root.join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"res://main.tscn\"\n[autoload]\nBoot=\"*res://boot.gd\"\n",
        )
        .unwrap();
        // main.tscn references hero.png twice — must count as ONE referencing file.
        fs::write(
            root.join("main.tscn"),
            "[ext_resource type=\"Texture2D\" path=\"res://hero.png\" id=\"1\"]\n[ext_resource type=\"Texture2D\" path=\"res://hero.png\" id=\"2\"]\n",
        )
        .unwrap();
        fs::write(root.join("boot.gd"), "extends Node\n").unwrap();
        fs::write(root.join("hero.png"), "x").unwrap();
        fs::write(root.join("orphan.png"), "x").unwrap();

        let mk = |name: &str, ext: &str| AssetInfo {
            path: root.join(name).to_string_lossy().to_string(),
            name: name.to_string(),
            extension: ext.to_string(),
            asset_type: AssetType::Other,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        };
        let assets = vec![
            mk("main.tscn", "tscn"),
            mk("boot.gd", "gd"),
            mk("hero.png", "png"),
            mk("orphan.png", "png"),
        ];

        let targets: Vec<String> = ["hero.png", "main.tscn", "boot.gd", "orphan.png"]
            .iter()
            .map(|n| root.join(n).to_string_lossy().to_string())
            .collect();
        let refs = referencing_files(root, &assets, &targets);

        // hero.png ← main.tscn, once despite the double mention.
        assert_eq!(refs.get(&targets[0]).map(Vec::len), Some(1));
        assert!(refs[&targets[0]][0].ends_with("main.tscn"));
        // main.tscn ← project.godot (main scene).
        assert_eq!(
            refs.get(&targets[1]),
            Some(&vec!["project.godot".to_string()])
        );
        // boot.gd ← project.godot (autoload, `*`-prefixed value).
        assert_eq!(
            refs.get(&targets[2]),
            Some(&vec!["project.godot".to_string()])
        );
        // orphan.png ← nobody: absent, not an empty entry.
        assert!(!refs.contains_key(&targets[3]));
    }

    #[test]
    fn test_godot_dependency_edges() {
        use crate::scanner::AssetType;
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("main.tscn"),
            "[ext_resource type=\"Texture2D\" path=\"res://hero.png\" id=\"1\"]\n",
        )
        .unwrap();

        let mk = |name: &str, ext: &str| AssetInfo {
            path: root.join(name).to_string_lossy().to_string(),
            name: name.to_string(),
            extension: ext.to_string(),
            asset_type: AssetType::Other,
            size: 1,
            modified: 0,
            metadata: None,
            unity_guid: None,
        };
        let assets = vec![mk("main.tscn", "tscn"), mk("hero.png", "png")];

        let edges = godot_dependency_edges(root, &assets);
        // main.tscn references hero.png -> exactly one edge.
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "res://main.tscn");
        assert_eq!(edges[0].1, "res://hero.png");
    }
}
