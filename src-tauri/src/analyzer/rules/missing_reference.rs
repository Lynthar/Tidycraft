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

/// `sources` are the files walked for references — the analysis scope, i.e.
/// post-`[ignore]`. `known` is what establishes which GUIDs exist and must be
/// the FULL scan, ignore patterns included.
///
/// The two differ on purpose. `[ignore]` means "don't report problems in these
/// files", not "pretend these files were deleted" — and the sample config's own
/// example (`"Plugins/**"`, `"ThirdParty/**"`) is exactly the case where the
/// difference bites: dropping vendored assets from the existence universe made
/// every prefab/scene/material that references them report a dangling GUID, in
/// bulk. Ignoring a *referencing* file still suppresses its findings, because
/// it never appears in `sources` — the suppression path the docs describe.
pub fn find_missing_references(
    sources: &[AssetInfo],
    known: &[AssetInfo],
    project_type: &Option<ProjectType>,
    package_index: &unity::PackageGuidIndex,
) -> AnalysisResult {
    let mut result = AnalysisResult::new();

    // Only applicable to Unity projects. Other engines have their own
    // reference schemes (Unreal's `.uasset` is binary; Godot uses path
    // strings) that we don't parse here.
    if !matches!(project_type, Some(ProjectType::Unity)) {
        return result;
    }

    // Build the set of GUIDs that DO exist in the project.
    let known_guids: HashSet<String> = known
        .iter()
        .filter_map(|a| a.unity_guid.clone())
        .collect();

    if known_guids.is_empty() {
        return result; // No .meta files scanned — Unity project state is empty or unusual.
    }

    for asset in sources {
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
            // Package assets resolve through the PackageCache index — known
            // to exist, just installed by the package manager rather than
            // living in the project. Not a finding.
            if package_index.get(&r.guid).is_some() {
                continue;
            }
            if known_guids.contains(&r.guid) {
                continue;
            }
            if !reported.insert(r.guid.clone()) {
                continue;
            }
            // Warning, not Error: known_guids only covers what the scan saw
            // and the package index only what a local Library/ cache
            // resolves (a fresh clone has neither) — a miss is strong
            // signal, not proof of breakage.
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

    /// `[ignore]` narrows what we report on, not what exists. Ignoring the
    /// sample config's own `"Plugins/**"` used to make every prefab that
    /// referenced a plugin asset report a dangling GUID — in bulk, on a
    /// perfectly healthy project.
    #[test]
    fn ignored_assets_still_count_as_existing() {
        let dir = tempdir().unwrap();
        let vendored =
            texture_with_guid(dir.path(), "plugin.png", "33333333333333333333333333333333");
        let prefab = prefab_referencing(
            dir.path(),
            "user.prefab",
            &["33333333333333333333333333333333"],
        );

        // Post-[ignore] scope dropped the vendored texture; the full scan has both.
        let sources = vec![prefab.clone()];
        let known = vec![vendored, prefab];

        let r = find_missing_references(
            &sources,
            &known,
            &Some(ProjectType::Unity),
            &unity::PackageGuidIndex::default(),
        );
        assert_eq!(r.issue_count, 0);
    }

    /// The other half of the contract, which the docs advertise as the way to
    /// silence a known-broken file: ignoring the *referencing* file still
    /// suppresses its findings, because it never enters `sources`.
    #[test]
    fn ignoring_the_referencing_file_still_suppresses_it() {
        let dir = tempdir().unwrap();
        let broken = prefab_referencing(
            dir.path(),
            "legacy.prefab",
            &["44444444444444444444444444444444"], // genuinely absent
        );
        let known = vec![broken];

        let r = find_missing_references(
            &[], // the prefab itself was ignored
            &known,
            &Some(ProjectType::Unity),
            &unity::PackageGuidIndex::default(),
        );
        assert_eq!(r.issue_count, 0);

        // Sanity: without the ignore it is still reported.
        let r = find_missing_references(
            &known,
            &known,
            &Some(ProjectType::Unity),
            &unity::PackageGuidIndex::default(),
        );
        assert_eq!(r.issue_count, 1);
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unreal), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn package_resolved_guids_are_not_missing() {
        // A guid the PackageCache index accounts for is a package asset —
        // known to exist, not a finding; an unindexed one still reports.
        let dir = tempdir().unwrap();
        let pkg = dir
            .path()
            .join("Library")
            .join("PackageCache")
            .join("com.example.pkg@1.0.0");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("Thing.shader.meta"),
            "fileFormatVersion: 2\nguid: 33333333333333333333333333333333\n",
        )
        .unwrap();
        let index = crate::unity::build_package_guid_index(dir.path());

        let assets = vec![
            texture_with_guid(dir.path(), "t.png", "11111111111111111111111111111111"),
            prefab_referencing(
                dir.path(),
                "p.prefab",
                &[
                    "33333333333333333333333333333333", // package-resolved
                    "22222222222222222222222222222222", // truly unknown
                ],
            ),
        ];
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &index);
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0].message.contains("22222222"));
    }

    #[test]
    fn empty_project_reports_nothing() {
        let r = find_missing_references(&[], &[], &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
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
        let r = find_missing_references(&assets, &assets, &Some(ProjectType::Unity), &unity::PackageGuidIndex::default());
        assert_eq!(r.issue_count, 1);
        assert!(matches!(r.issues[0].severity, Severity::Warning));
    }
}
