//! Unity missing-reference detection.
//!
//! The inverse of `find_unused_assets`: walk every referenceable Unity file
//! (prefab / scene / material / controller / asset) and flag any GUID it
//! references that doesn't resolve to an asset we scanned. This catches the
//! classic "I deleted `foo.png` but still have a prefab pointing at its GUID"
//! breakage that Unity's own editor only surfaces once you open the asset.

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::{AnalysisResult, Issue, Severity};
use crate::scanner::{AssetInfo, ProjectType};
use crate::unity;

/// Extensions that Unity stores as YAML with GUID references.
const REFERENCEABLE_EXTS: &[&str] = &["prefab", "unity", "mat", "controller", "asset"];

pub fn find_missing_references(
    assets: &[AssetInfo],
    project_type: &Option<ProjectType>,
) -> AnalysisResult {
    let mut result = AnalysisResult::new();

    // Only applicable to Unity projects. Other engines have their own
    // reference schemes (Unreal's `.uasset` is binary; Godot uses path
    // strings) that we don't parse here.
    if !matches!(project_type, Some(ProjectType::Unity)) {
        return result;
    }

    // Build the set of GUIDs that DO exist in the project.
    let known_guids: HashSet<String> = assets
        .iter()
        .filter_map(|a| a.unity_guid.clone())
        .collect();

    if known_guids.is_empty() {
        return result; // No .meta files scanned — Unity project state is empty or unusual.
    }

    for asset in assets {
        let ext = asset.extension.to_lowercase();
        if !REFERENCEABLE_EXTS.iter().any(|&e| e == ext) {
            continue;
        }

        let info = match unity::parse_unity_file(Path::new(&asset.path)) {
            Some(i) => i,
            None => continue,
        };

        // Dedup per source: a prefab referencing the same missing GUID in
        // five places is still one broken link.
        let mut reported: HashSet<String> = HashSet::new();
        for r in &info.references {
            // Unity uses all-zero GUID as "no reference"; skip these. The
            // editor-shipped built-in bundles are never in the scan set by
            // design. Both classifiers live in `unity.rs` (shared with the
            // dependency graph, which applies the same exemptions).
            if unity::is_null_guid(&r.guid) || unity::is_builtin_guid(&r.guid) {
                continue;
            }
            if known_guids.contains(&r.guid) {
                continue;
            }
            if !reported.insert(r.guid.clone()) {
                continue;
            }
            // Warning, not Error: known_guids only covers what the scan saw,
            // and gitignored Library/ or Packages/ contents never enter it —
            // a miss is strong signal, not proof of breakage.
            result.add_issue(Issue {
                rule_id: "missing_reference".to_string(),
                rule_name: "Missing Reference".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "References GUID `{}` which is not in the project",
                    r.guid
                ),
                asset_path: asset.path.clone(),
                suggestion: Some(
                    "Either the target was deleted without updating this file, or its \
                     .meta was lost. Reimport the target or fix the reference in Unity."
                        .to_string(),
                ),
                auto_fixable: false,
            related_paths: None,
            });
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetInfo, AssetType};
    use std::fs;
    use tempfile::tempdir;

    fn texture_with_guid(dir: &std::path::Path, name: &str, guid: &str) -> AssetInfo {
        let path = dir.join(name);
        fs::write(&path, b"fake").unwrap();
        // Write Unity .meta sidecar so a real scan would pick up the guid,
        // though for these unit tests we set unity_guid directly on AssetInfo.
        AssetInfo {
            path: path.to_string_lossy().to_string(),
            name: name.to_string(),
            extension: "png".to_string(),
            asset_type: AssetType::Texture,
            size: 4,
            modified: 0,
            metadata: None,
            unity_guid: Some(guid.to_string()),
        }
    }

    fn prefab_referencing(dir: &std::path::Path, name: &str, refs: &[&str]) -> AssetInfo {
        let mut content = String::from("--- !u!1 &1\nGameObject:\n  m_Name: Test\n");
        for g in refs {
            content.push_str(&format!(
                "  m_Texture: {{fileID: 2800000, guid: {}, type: 3}}\n",
                g
            ));
        }
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        AssetInfo {
            path: path.to_string_lossy().to_string(),
            name: name.to_string(),
            extension: "prefab".to_string(),
            asset_type: AssetType::Prefab,
            size: 0,
            modified: 0,
            metadata: None,
            unity_guid: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
        }
    }

    #[test]
    fn reports_only_missing_guids() {
        let dir = tempdir().unwrap();
        let assets = vec![
            texture_with_guid(dir.path(), "known.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "scene.prefab",
                &[
                    "11111111111111111111111111111111", // exists
                    "22222222222222222222222222222222", // missing
                ],
            ),
        ];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0].message.contains("22222222"));
    }

    #[test]
    fn deduplicates_same_missing_guid_in_one_source() {
        let dir = tempdir().unwrap();
        let assets = vec![prefab_referencing(
            dir.path(),
            "broken.prefab",
            &[
                "99999999999999999999999999999999",
                "99999999999999999999999999999999",
                "99999999999999999999999999999999",
            ],
        )];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 1);
    }

    #[test]
    fn skips_non_unity_projects() {
        let dir = tempdir().unwrap();
        let assets = vec![prefab_referencing(
            dir.path(),
            "x.prefab",
            &["99999999999999999999999999999999"],
        )];
        let r = find_missing_references(&assets, &Some(ProjectType::Unreal));
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn skips_zero_guid_sentinel() {
        let dir = tempdir().unwrap();
        let assets = vec![
            texture_with_guid(dir.path(), "t.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "p.prefab",
                &["00000000000000000000000000000000"],
            ),
        ];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn empty_project_reports_nothing() {
        let r = find_missing_references(&[], &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn skips_unity_builtin_guids() {
        // `...e...` = "unity default resources", `...f...` = "unity_builtin_extra".
        // Both ship inside the editor, are referenced by any project touching a
        // built-in shader/material/sprite, and never have a scanned .meta —
        // flagging them buries real breakage in noise on ordinary projects.
        let dir = tempdir().unwrap();
        let assets = vec![
            texture_with_guid(dir.path(), "t.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "p.prefab",
                &[
                    "0000000000000000e000000000000000",
                    "0000000000000000f000000000000000",
                ],
            ),
        ];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn near_builtin_guids_are_still_reported() {
        // Only the two exact builtin GUIDs are exempt; anything merely
        // resembling them is a genuine dangling reference.
        let dir = tempdir().unwrap();
        let assets = vec![
            texture_with_guid(dir.path(), "t.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "p.prefab",
                &[
                    "0000000000000000a000000000000000", // wrong marker char
                    "0000000000000000e000000000000001", // non-zero tail
                    "e0000000000000000000000000000000", // marker misplaced
                ],
            ),
        ];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 3);
    }

    #[test]
    fn missing_reference_severity_is_warning() {
        // The detector's evidence is heuristic — gitignored Library/ and
        // Packages/ never enter known_guids, so a miss is strong signal but
        // not proof. Warning, not Error (user-approved downgrade).
        let dir = tempdir().unwrap();
        let assets = vec![
            texture_with_guid(dir.path(), "t.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "p.prefab",
                &["22222222222222222222222222222222"],
            ),
        ];
        let r = find_missing_references(&assets, &Some(ProjectType::Unity));
        assert_eq!(r.issue_count, 1);
        assert!(matches!(r.issues[0].severity, Severity::Warning));
    }
}
