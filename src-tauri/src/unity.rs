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
    Anim,
    Unknown,
}

impl UnityFileType {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "prefab" => UnityFileType::Prefab,
            "unity" => UnityFileType::Scene,
            "mat" => UnityFileType::Material,
            // Animator override controllers reference a base controller + clips.
            "controller" | "overridecontroller" => UnityFileType::Controller,
            "asset" => UnityFileType::Asset,
            // Sprite-animation .anim files reference sprites/clips via PPtr guids.
            "anim" => UnityFileType::Anim,
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
        // Normalized like every other path we hand the frontend — on
        // Windows `to_string_lossy` alone would leak backslashes.
        path: crate::scanner::path_to_string(path),
        file_type,
        references,
        components,
    })
}

/// Unity project info surfaced on the Stats dashboard's engine card. Parsed
/// from `ProjectSettings/ProjectVersion.txt` — plain `key: value` YAML the
/// editor rewrites on every version switch and that is committed to VCS by
/// convention (standard Unity .gitignore templates keep ProjectSettings/).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnityProjectInfo {
    /// e.g. "2022.3.10f1"
    pub editor_version: String,
    /// e.g. "2022.3.10f1 (ff3792e53c62)" — includes the changeset hash.
    /// Absent in old Unity versions that only wrote `m_EditorVersion`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_version_with_revision: Option<String>,
}

/// Read `ProjectSettings/ProjectVersion.txt` under `root_path`. `None` when
/// the file is missing (not a Unity project, or settings not checked out) or
/// carries no `m_EditorVersion` line — the frontend hides the card.
pub fn parse_project_version(root_path: &Path) -> Option<UnityProjectInfo> {
    let path = root_path.join("ProjectSettings").join("ProjectVersion.txt");
    let content = fs::read_to_string(path).ok()?;

    let mut editor_version: Option<String> = None;
    let mut with_revision: Option<String> = None;
    for line in content.lines() {
        // Field order isn't guaranteed; prefix-match each line. The
        // `WithRevision` check must come first: `m_EditorVersion:` is a
        // prefix of `m_EditorVersionWithRevision:`.
        if let Some(v) = line.strip_prefix("m_EditorVersionWithRevision:") {
            with_revision = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        } else if let Some(v) = line.strip_prefix("m_EditorVersion:") {
            editor_version = Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }

    Some(UnityProjectInfo {
        editor_version: editor_version?,
        editor_version_with_revision: with_revision,
    })
}

/// The all-zero GUID Unity writes as a "no reference" sentinel. The empty
/// string counts as null too (vacuously all-zero), so raw parse output can be
/// fed straight in.
pub fn is_null_guid(guid: &str) -> bool {
    guid.chars().all(|c| c == '0')
}

/// The two GUIDs Unity reserves for editor-shipped asset bundles:
/// `0000000000000000e000000000000000` ("unity default resources") and
/// `0000000000000000f000000000000000` ("unity_builtin_extra"). Any project
/// that touches a built-in shader, material, or UI sprite references them,
/// and they never correspond to a scanned .meta — so neither the
/// missing-reference rule nor the dependency graph may treat them as
/// dangling. NOTE: do not try to detect built-ins via the reference's
/// `type:` field instead — real project asset references (e.g. a .mat's
/// texture) are `type: 3` while built-ins are `type: 0`; filtering on type
/// would suppress true positives.
pub fn is_builtin_guid(guid: &str) -> bool {
    let bytes = guid.as_bytes();
    bytes.len() == 32
        && bytes[..16].iter().all(|&b| b == b'0')
        && (bytes[16] == b'e' || bytes[16] == b'f')
        && bytes[17..].iter().all(|&b| b == b'0')
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

    // HashSet iteration order is random per process — sort so the preview
    // panel's component list doesn't reshuffle between selections.
    let mut components: Vec<String> = components.into_iter().collect();
    components.sort();
    components
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

    #[test]
    fn file_type_matches_real_world_extension_casing() {
        // Unity writes `.overrideController` (camelCase) on disk, and
        // `parse_unity_file` feeds `Path::extension()` verbatim — matching must
        // not depend on the caller lowercasing first.
        assert_eq!(
            UnityFileType::from_extension("overrideController"),
            UnityFileType::Controller
        );
        assert_eq!(UnityFileType::from_extension("PREFAB"), UnityFileType::Prefab);
        assert_eq!(UnityFileType::from_extension("Anim"), UnityFileType::Anim);
    }

    #[test]
    fn parse_unity_file_path_uses_forward_slashes() {
        // Forward-slash discipline: every path we serialize to the frontend
        // is `/`-separated. Trivially true on Unix; on Windows CI the tempdir
        // path contains `\` and this guards the normalization.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Thing.prefab");
        std::fs::write(&p, "--- !u!1 &1\nGameObject:\n  m_Name: Thing\n").unwrap();

        let info = parse_unity_file(&p).expect("prefab should parse");
        assert!(
            !info.path.contains('\\'),
            "path must be forward-slash normalized: {}",
            info.path
        );
    }

    #[test]
    fn null_and_builtin_guid_classifiers() {
        assert!(is_null_guid("00000000000000000000000000000000"));
        assert!(is_null_guid("")); // vacuous all-zero — see the doc comment
        assert!(!is_null_guid("00000000000000000000000000000001"));

        // The exact two editor-shipped bundles…
        assert!(is_builtin_guid("0000000000000000e000000000000000"));
        assert!(is_builtin_guid("0000000000000000f000000000000000"));
        // …and nothing that merely resembles them (those are genuine
        // dangling references — see the missing_reference rule tests).
        assert!(!is_builtin_guid("0000000000000000a000000000000000")); // wrong marker
        assert!(!is_builtin_guid("0000000000000000e000000000000001")); // non-zero tail
        assert!(!is_builtin_guid("e0000000000000000000000000000000")); // marker misplaced
        assert!(!is_builtin_guid("0000000000000000e00000000000000")); // 31 chars
        assert!(!is_builtin_guid("00000000000000000000000000000000")); // null, not builtin
    }

    #[test]
    fn extract_components_is_sorted_and_deduped() {
        let content = "\
--- !u!4 &1\nTransform:\n\
--- !u!23 &2\nMeshRenderer:\n\
--- !u!4 &3\nTransform:\n\
--- !u!114 &4\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: abc123def456789012345678901234ab, type: 3}\n";
        let components = extract_components(content);
        // Deduped (two Transforms → one) and alphabetically sorted, so the
        // preview panel's list is stable across runs.
        assert_eq!(components, vec!["MeshRenderer", "MonoBehaviour", "Transform"]);
    }

    #[test]
    fn parse_project_version_reads_both_fields() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("ProjectSettings");
        fs::create_dir(&settings).unwrap();
        fs::write(
            settings.join("ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.10f1\nm_EditorVersionWithRevision: 2022.3.10f1 (ff3792e53c62)\n",
        )
        .unwrap();

        let info = parse_project_version(dir.path()).expect("should parse");
        assert_eq!(info.editor_version, "2022.3.10f1");
        assert_eq!(
            info.editor_version_with_revision.as_deref(),
            Some("2022.3.10f1 (ff3792e53c62)")
        );
    }

    #[test]
    fn parse_project_version_tolerates_missing_revision() {
        // Old Unity versions wrote only m_EditorVersion.
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("ProjectSettings");
        fs::create_dir(&settings).unwrap();
        fs::write(settings.join("ProjectVersion.txt"), "m_EditorVersion: 5.6.7f1\n").unwrap();

        let info = parse_project_version(dir.path()).expect("should parse");
        assert_eq!(info.editor_version, "5.6.7f1");
        assert!(info.editor_version_with_revision.is_none());
    }

    #[test]
    fn parse_project_version_none_when_absent_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        // No ProjectSettings at all.
        assert!(parse_project_version(dir.path()).is_none());

        // File exists but has no usable version line.
        let settings = dir.path().join("ProjectSettings");
        fs::create_dir(&settings).unwrap();
        fs::write(settings.join("ProjectVersion.txt"), "m_EditorVersion:\n").unwrap();
        assert!(parse_project_version(dir.path()).is_none());
    }
}
