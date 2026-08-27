//! DCC source-file linking. Pairs authoring/source files (`.blend`, `.ma`,
//! `.psd`, `.spp`, …) with their runtime exports by stem matching, then warns
//! when a source's mtime is newer than its export's.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::analyzer::{issue_args, AnalysisResult, Issue, Severity};
use crate::scanner::AssetInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DccSourceConfig {
    /// Out-of-box OFF: pairing rules are opinionated, and a fresh project rarely
    /// matches a default set. Opt in via `tidycraft.toml`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Tolerance in seconds for the "source newer than export" comparison.
    /// `git checkout` synchronizes mtimes, so a fresh clone would otherwise burst
    /// issues; 60s also covers filesystems with coarse mtime granularity.
    #[serde(default = "default_mtime_tolerance")]
    pub mtime_tolerance_secs: u64,
    /// One mapping per DCC tool family. The default list covers the common stack;
    /// users can override it wholesale to add in-house source formats or trim it
    /// to the tools their pipeline uses.
    #[serde(default = "default_mappings")]
    pub mappings: Vec<DccMapping>,
    /// Where to look for export candidates relative to the source.
    #[serde(default)]
    pub lookup: DccLookup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DccMapping {
    /// Display label — appears in the issue's suggestion text
    /// (e.g. "Re-export from blender").
    pub name: String,
    /// Source-side extensions, lowercase. A file with one of these
    /// extensions enters the pairing pipeline as a "source".
    pub sources: Vec<String>,
    /// Export-side extensions, lowercase. A candidate file in a
    /// lookup directory must have one of these to count as a matching
    /// export for this mapping's sources.
    pub exports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DccLookup {
    /// Search the source's own directory for exports. Default true —
    /// the most common layout (`models/character.blend` next to
    /// `models/character.fbx`).
    #[serde(default = "default_true")]
    pub same_dir: bool,
    /// Sibling directory names to also check. The walker rebuilds candidate paths
    /// by joining each sibling name under every ancestor of the source's parent.
    #[serde(default = "default_sibling_dirs")]
    pub sibling_dirs: Vec<String>,
}

impl Default for DccLookup {
    fn default() -> Self {
        Self {
            same_dir: true,
            sibling_dirs: default_sibling_dirs(),
        }
    }
}

fn default_enabled() -> bool {
    false
}

fn default_mtime_tolerance() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

fn default_sibling_dirs() -> Vec<String> {
    vec!["sources".into(), "_source".into(), "src".into()]
}

fn default_mappings() -> Vec<DccMapping> {
    fn m(name: &str, sources: &[&str], exports: &[&str]) -> DccMapping {
        DccMapping {
            name: name.into(),
            sources: sources.iter().map(|s| s.to_string()).collect(),
            exports: exports.iter().map(|s| s.to_string()).collect(),
        }
    }
    vec![
        m("blender", &["blend"], &["fbx", "glb", "gltf", "obj", "dae"]),
        m("maya", &["ma", "mb"], &["fbx", "obj"]),
        m("max", &["max"], &["fbx", "obj"]),
        m("zbrush", &["ztl", "zpr"], &["obj", "fbx"]),
        m("modo", &["lxo"], &["fbx", "obj"]),
        m(
            "houdini",
            &["hip", "hipnc", "hiplc"],
            &["fbx", "obj", "abc", "usd"],
        ),
        m("cinema4d", &["c4d"], &["fbx", "obj"]),
        m("marvelous", &["zprj"], &["obj", "fbx"]),
        // 1→1 stem match: .spp pairs with the newest same-stem texture.
        m(
            "substance_painter",
            &["spp"],
            &["png", "tga", "jpg", "tif", "tiff", "exr"],
        ),
        m("substance_designer", &["sbs"], &["sbsar", "png", "tga"]),
        m("photoshop", &["psd", "psb"], &["png", "jpg", "tga", "webp"]),
    ]
}

impl Default for DccSourceConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            mtime_tolerance_secs: default_mtime_tolerance(),
            mappings: default_mappings(),
            lookup: DccLookup::default(),
        }
    }
}

/// The file's last-modified time as Unix epoch seconds, `None` on any IO or clock
/// error. A single unreadable file skips its pair rather than poisoning the
/// analyze run.
fn read_mtime_secs(path: &str) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?;
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Candidate parent directories an export might live in. Includes the source's
/// own directory when `lookup.same_dir`, and expands a sibling-named ancestor to
/// its grandparent. The walk only ever goes UP.
fn candidate_dirs(source_parent: &str, lookup: &DccLookup) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let parent_norm = source_parent.trim_end_matches('/');

    let push = |s: String, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        if !s.is_empty() && seen.insert(s.clone()) {
            out.push(s);
        }
    };

    if lookup.same_dir {
        push(parent_norm.to_string(), &mut out, &mut seen);
    }

    if !lookup.sibling_dirs.is_empty() {
        // Walk up from the source's parent. At every ancestor whose leaf name
        // matches a configured sibling-dir, the grandparent becomes a candidate
        // export site, plus that grandparent's other sibling-named subdirs.
        let parent_path = Path::new(parent_norm);
        let mut walker: Option<&Path> = Some(parent_path);
        while let Some(p) = walker {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                let name_lower = name.to_lowercase();
                let is_sibling_named = lookup
                    .sibling_dirs
                    .iter()
                    .any(|sib| sib.eq_ignore_ascii_case(&name_lower));
                if is_sibling_named {
                    if let Some(grandparent) = p.parent().and_then(|gp| gp.to_str()) {
                        let gp = grandparent.trim_end_matches('/').to_string();
                        push(gp.clone(), &mut out, &mut seen);
                        for sib in &lookup.sibling_dirs {
                            // Skip the entry that equals the matched
                            // ancestor's name — that path IS p (already
                            // covered by same_dir or seen-dedup).
                            if sib.eq_ignore_ascii_case(&name_lower) {
                                continue;
                            }
                            let sib_lower = sib.to_lowercase();
                            let candidate = if gp.is_empty() {
                                sib_lower
                            } else {
                                format!("{}/{}", gp, sib_lower)
                            };
                            push(candidate, &mut out, &mut seen);
                        }
                    }
                }
            }
            walker = p.parent();
        }
    }
    out
}

/// Find which mapping owns the given source extension. Returns `None`
/// when the extension isn't a configured DCC source.
fn mapping_for_source<'a>(mappings: &'a [DccMapping], ext: &str) -> Option<&'a DccMapping> {
    let ext_lower = ext.to_lowercase();
    mappings
        .iter()
        .find(|m| m.sources.iter().any(|s| s.eq_ignore_ascii_case(&ext_lower)))
}

/// Split a duration into a magnitude and a unit tag. The English prose renders it
/// as "3d"; locales render the tag through `issues.duration.*`. The bucket choice
/// lives only here — the frontend just looks the tag up.
fn humanize_seconds(secs: u64) -> (u64, &'static str) {
    if secs < 60 {
        (secs, "s")
    } else if secs < 3600 {
        (secs / 60, "m")
    } else if secs < 86400 {
        (secs / 3600, "h")
    } else {
        (secs / 86400, "d")
    }
}

pub fn find_dcc_source_issues(assets: &[AssetInfo], config: &DccSourceConfig) -> AnalysisResult {
    let mut result = AnalysisResult::new();
    if !config.enabled || config.mappings.is_empty() {
        return result;
    }

    // Index assets by (parent_dir, stem) for O(1) export lookup. Both keys are
    // lowercased so case-insensitive filesystems and mixed-case stems still match.
    type Key = (String, String);
    let mut by_key: HashMap<Key, Vec<&AssetInfo>> = HashMap::new();
    for a in assets {
        let path = Path::new(&a.path);
        let parent = path
            .parent()
            .and_then(|p| p.to_str())
            .map(|s| s.trim_end_matches('/').to_lowercase())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if stem.is_empty() {
            continue;
        }
        by_key.entry((parent, stem)).or_default().push(a);
    }

    // Sort sources for stable issue order across runs (HashMap iter
    // is otherwise nondeterministic and would churn the issue list).
    let mut sources: Vec<&AssetInfo> = assets
        .iter()
        .filter(|a| mapping_for_source(&config.mappings, &a.extension).is_some())
        .collect();
    sources.sort_by(|a, b| a.path.cmp(&b.path));

    for source in sources {
        // mapping_for_source already validated this returns Some; safe to unwrap.
        let mapping = match mapping_for_source(&config.mappings, &source.extension) {
            Some(m) => m,
            None => continue,
        };

        let source_path = Path::new(&source.path);
        let stem_lower = match source_path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_lowercase(),
            None => continue,
        };
        let parent_lower = source_path
            .parent()
            .and_then(|p| p.to_str())
            .map(|s| s.trim_end_matches('/').to_lowercase())
            .unwrap_or_default();

        let candidates = candidate_dirs(&parent_lower, &config.lookup);

        // Find the newest export across candidate directories.
        let mut best: Option<(&AssetInfo, u64)> = None;
        for dir in &candidates {
            if let Some(group) = by_key.get(&(dir.clone(), stem_lower.clone())) {
                for cand in group {
                    if cand.path == source.path {
                        continue;
                    }
                    let cand_ext = cand.extension.to_lowercase();
                    if !mapping
                        .exports
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&cand_ext))
                    {
                        continue;
                    }
                    let mtime = match read_mtime_secs(&cand.path) {
                        Some(m) => m,
                        None => continue,
                    };
                    if best.is_none_or(|(_, old)| mtime > old) {
                        best = Some((cand, mtime));
                    }
                }
            }
        }

        let (export, export_mtime) = match best {
            Some(p) => p,
            // No matching export found — silent, since it could legitimately be a
            // source the user has not exported yet.
            None => continue,
        };
        let source_mtime = match read_mtime_secs(&source.path) {
            Some(m) => m,
            None => continue,
        };

        // Source must be strictly newer than export by more than the
        // tolerance. Equal or "newer by < tolerance" is treated as
        // synchronized (e.g. just-after-git-checkout).
        if source_mtime <= export_mtime.saturating_add(config.mtime_tolerance_secs) {
            continue;
        }

        let diff = source_mtime - export_mtime;
        let (age_value, age_unit) = humanize_seconds(diff);
        result.add_issue(Issue {
            rule_id: "dcc_source.outdated_export".into(),
            rule_name: "Outdated DCC export".into(),
            severity: Severity::Warning,
            message: format!(
                "Source `{}` is {}{} newer than its export `{}` — possibly missing a re-export.",
                source.name, age_value, age_unit, export.name,
            ),
            asset_path: source.path.clone(),
            suggestion: Some(format!(
                "Re-export from {} and verify the new export's mtime advances past the source. To suppress, add the source path to `[ignore].patterns`.",
                mapping.name,
            )),
            auto_fixable: false,
            related_paths: None,
            args: issue_args([
                ("source", source.name.clone()),
                ("export", export.name.clone()),
                ("dcc", mapping.name.clone()),
                ("age_value", age_value.to_string()),
                ("age_unit", age_unit.to_string()),
            ]),
        });
    }

    result
}

/// `pub(crate)` so `analyzer::tests`' arg-harvest can build its fixture from
/// the same two constructors this rule's own tests use, rather than a copy
/// that would quietly stop matching the rule the day the rule changes.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, AssetType};
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use tempfile::tempdir;

    /// Build an AssetInfo for a file path. Only path / name / extension
    /// are exercised by the analyzer's index; the rest can be defaults.
    pub(crate) fn make_asset(path: &str, asset_type: AssetType) -> AssetInfo {
        let p = Path::new(path);
        AssetInfo {
            path: path.to_string(),
            name: p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(path)
                .to_string(),
            extension: p
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
            asset_type,
            size: 1,
            modified: 0,
            metadata: Some(AssetMetadata::default()),
            unity_guid: None,
        }
    }

    /// Write a 1-byte fixture and stamp its mtime to N seconds ago.
    /// `filetime` is the cross-platform standard; std doesn't expose
    /// mtime setting.
    pub(crate) fn write_with_mtime(path: &Path, secs_ago: u64) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"x").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let when = FileTime::from_unix_time((now - secs_ago) as i64, 0);
        set_file_mtime(path, when).unwrap();
    }

    #[test]
    fn disabled_yields_no_issues() {
        // Early-out catches before any IO — fixture files unnecessary.
        let assets = vec![make_asset("/p/character.blend", AssetType::Model)];
        let cfg = DccSourceConfig::default();
        assert!(!cfg.enabled);
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    /// Both halves of the "search only goes up" limitation in
    /// `docs/analyzer-rules.md`. The doc tells users to restructure around
    /// this, so the claim and the workaround both need to stay true.
    #[test]
    fn an_export_subdirectory_is_unreachable_a_sibling_one_is_not() {
        let lookup = DccLookup {
            same_dir: true,
            sibling_dirs: vec!["sources".into(), "exported".into()],
        };

        // models/hero.blend looking for models/exported/hero.fbx: the
        // export dir is below the source's dir, so it is never a candidate.
        assert_eq!(
            candidate_dirs("/proj/models", &lookup),
            vec!["/proj/models".to_string()]
        );

        // Same pair, source moved into the sibling-named dir: now reachable.
        assert!(candidate_dirs("/proj/models/sources", &lookup)
            .contains(&"/proj/models/exported".to_string()));
    }

    #[test]
    fn same_dir_pairing_emits_outdated_warning() {
        let dir = tempdir().unwrap();
        let blend = dir.path().join("character.blend");
        let fbx = dir.path().join("character.fbx");
        write_with_mtime(&fbx, 7200); // 2h ago
        write_with_mtime(&blend, 60); // 1m ago — newer than fbx

        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            mtime_tolerance_secs: 5,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert_eq!(r.issues[0].rule_id, "dcc_source.outdated_export");
        assert!(r.issues[0].message.contains("character.blend"));
        assert!(r.issues[0].message.contains("character.fbx"));
        // diff = 7200s - 60s = 7140s, bucketed to whole hours: "1h", not
        // "1 h" or "h1". Comfortably clear of the 3600s/7200s bucket edges.
        assert!(r.issues[0].message.contains("1h newer than its export"));
    }

    #[test]
    fn no_issue_when_export_is_newer() {
        let dir = tempdir().unwrap();
        let blend = dir.path().join("hero.blend");
        let fbx = dir.path().join("hero.fbx");
        write_with_mtime(&blend, 7200);
        write_with_mtime(&fbx, 60);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn tolerance_suppresses_within_window() {
        // Source 30s newer; default tolerance 60s → no issue.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("prop.blend");
        let fbx = dir.path().join("prop.fbx");
        write_with_mtime(&fbx, 60);
        write_with_mtime(&blend, 30);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn no_export_candidate_no_issue() {
        // Orphan sources are silent.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("orphan.blend");
        write_with_mtime(&blend, 60);
        let assets = vec![make_asset(&blend.to_string_lossy(), AssetType::Model)];
        let cfg = DccSourceConfig {
            enabled: true,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn unknown_source_extension_no_issue() {
        // .txt isn't in any default mapping — must not crash, must not
        // falsely pair against a same-stem .png.
        let dir = tempdir().unwrap();
        let txt = dir.path().join("readme.txt");
        let png = dir.path().join("readme.png");
        write_with_mtime(&png, 7200);
        write_with_mtime(&txt, 60);
        let assets = vec![
            make_asset(&txt.to_string_lossy(), AssetType::Other),
            make_asset(&png.to_string_lossy(), AssetType::Texture),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn picks_newest_export_when_multiple_match() {
        let dir = tempdir().unwrap();
        let blend = dir.path().join("crate.blend");
        let fbx = dir.path().join("crate.fbx");
        let glb = dir.path().join("crate.glb");
        write_with_mtime(&fbx, 7200);
        write_with_mtime(&glb, 30); // newer than blend → no issue
        write_with_mtime(&blend, 60);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
            make_asset(&glb.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        // Newest export (.glb) is newer than source → no warning,
        // even though .fbx alone would have triggered.
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn sibling_dir_lookup_finds_export() {
        // Layout: <root>/sources/Hero.blend (30s ago) and <root>/Hero.fbx (7200s
        // ago). `sources` is a default sibling_dir, so walking up from
        // <root>/sources reaches <root> and finds Hero.fbx.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("sources").join("Hero.blend");
        let fbx = dir.path().join("Hero.fbx");
        write_with_mtime(&fbx, 7200);
        write_with_mtime(&blend, 30);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            mtime_tolerance_secs: 5,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0].message.contains("Hero.blend"));
        assert!(r.issues[0].message.contains("Hero.fbx"));
    }

    #[test]
    fn humanize_buckets() {
        assert_eq!(humanize_seconds(30), (30, "s"));
        assert_eq!(humanize_seconds(60), (1, "m"));
        assert_eq!(humanize_seconds(120), (2, "m"));
        assert_eq!(humanize_seconds(7200), (2, "h"));
        assert_eq!(humanize_seconds(86400 * 3), (3, "d"));
    }

    #[test]
    fn issues_sorted_by_source_path_for_stable_output() {
        // Two source/export pairs in different dirs. Both fire; the
        // issue ordering must be stable across runs (we sort sources
        // by path before iterating).
        let dir = tempdir().unwrap();
        let a_blend = dir.path().join("a").join("foo.blend");
        let a_fbx = dir.path().join("a").join("foo.fbx");
        let b_blend = dir.path().join("b").join("bar.blend");
        let b_fbx = dir.path().join("b").join("bar.fbx");
        write_with_mtime(&a_fbx, 7200);
        write_with_mtime(&a_blend, 30);
        write_with_mtime(&b_fbx, 7200);
        write_with_mtime(&b_blend, 30);
        let assets = vec![
            make_asset(&a_blend.to_string_lossy(), AssetType::Model),
            make_asset(&a_fbx.to_string_lossy(), AssetType::Model),
            make_asset(&b_blend.to_string_lossy(), AssetType::Model),
            make_asset(&b_fbx.to_string_lossy(), AssetType::Model),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            mtime_tolerance_secs: 5,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 2);
        // Path-sorted: ".../a/foo.blend" precedes ".../b/bar.blend".
        let p0 = r.issues[0].asset_path.replace('\\', "/");
        let p1 = r.issues[1].asset_path.replace('\\', "/");
        assert!(
            p0.contains("/a/"),
            "expected first issue under /a/, got {p0}"
        );
        assert!(
            p1.contains("/b/"),
            "expected second issue under /b/, got {p1}"
        );
    }

    #[test]
    fn same_stem_different_mapping_does_not_pair() {
        // `.blend` must NOT pair with a same-stem `.png` — png is not in Blender's
        // exports list. Exercises the per-mapping export filter.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("ambiguous.blend");
        let png = dir.path().join("ambiguous.png");
        write_with_mtime(&png, 7200);
        write_with_mtime(&blend, 30);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&png.to_string_lossy(), AssetType::Texture),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            mtime_tolerance_secs: 5,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn photoshop_psd_to_png_pairs() {
        // Sanity: another mapping family fires correctly (Photoshop).
        let dir = tempdir().unwrap();
        let psd = dir.path().join("ui_button.psd");
        let png = dir.path().join("ui_button.png");
        write_with_mtime(&png, 7200);
        write_with_mtime(&psd, 30);
        let assets = vec![
            make_asset(&psd.to_string_lossy(), AssetType::Texture),
            make_asset(&png.to_string_lossy(), AssetType::Texture),
        ];
        let cfg = DccSourceConfig {
            enabled: true,
            mtime_tolerance_secs: 5,
            ..Default::default()
        };
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0]
            .suggestion
            .as_deref()
            .unwrap()
            .contains("photoshop"));
    }
}
