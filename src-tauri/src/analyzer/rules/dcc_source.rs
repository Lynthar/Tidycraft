//! DCC source-file linking.
//!
//! Cross-asset analyzer: pairs authoring/source files (`.blend`, `.ma`,
//! `.psd`, `.spp`, ...) with their runtime exports (`.fbx`, `.png`, ...)
//! by stem matching, then warns when the source's mtime is newer than
//! the export's — signalling a likely missing re-export.
//!
//! ## Pairing strategy
//!
//! For each source asset:
//! 1. Find the [`DccMapping`] whose `sources` list owns this extension.
//! 2. Build a candidate-export-directories set:
//!    - The source's own directory (when `lookup.same_dir`).
//!    - Sibling directories named in `lookup.sibling_dirs`, located by
//!      walking up from the source's parent and joining each sibling
//!      name at every ancestor level.
//! 3. In each candidate directory, look for files with the source's
//!    stem and an extension in `mapping.exports`.
//! 4. Pick the **newest** export across all candidates.
//! 5. If `source.mtime > export.mtime + tolerance_secs`, emit an issue.
//!
//! ## Why mtime, not content
//!
//! `git checkout` synchronizes mtimes, so cross-commit "stale" pairs
//! escape detection. This is a documented limitation; the signal we
//! DO catch reliably is the common "edited locally, forgot to
//! re-export" loop. A future iteration could read git status to
//! upgrade severity when the source file is dirty in the working tree.
//!
//! ## Phase 1 vs Phase 2
//!
//! Phase 1 (this file) does 1→1 stem matching only. Substance Painter
//! `.spp` → multi-channel PNG output is treated as 1→newest-PNG, which
//! is a useful approximation but not exhaustive. Phase 2 will add true
//! 1→N pairing using PBR channel suffixes (probably reusing the
//! `[pbr_set.channels]` config to identify expected outputs).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::analyzer::{AnalysisResult, Issue, Severity};
use crate::scanner::AssetInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DccSourceConfig {
    /// Out-of-box OFF: pairing rules are opinionated (which sources go
    /// with which exports), and a fresh project rarely matches a
    /// default set without a quick look at the mappings. Opt in via
    /// `tidycraft.toml`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Tolerance in seconds for the "source newer than export"
    /// comparison. `git checkout` synchronizes file mtimes to current
    /// time, so a freshly cloned/pulled repo has every file's mtime
    /// within milliseconds of every other; this tolerance keeps us
    /// from bursting issues immediately after a pull. 60s also covers
    /// slow disks / VM filesystems with > 1s mtime granularity.
    #[serde(default = "default_mtime_tolerance")]
    pub mtime_tolerance_secs: u64,
    /// One mapping per DCC tool family. The default list covers the
    /// common stack (Blender / Maya / Max / ZBrush / Modo / Houdini /
    /// Cinema 4D / Marvelous / Substance Painter+Designer / Photoshop).
    /// Users can override the whole list to add in-house source
    /// formats or trim down to just the tools their pipeline uses.
    #[serde(default = "default_mappings")]
    pub mappings: Vec<DccMapping>,
    /// Where to look for export candidates relative to the source.
    #[serde(default)]
    pub lookup: DccLookup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct DccLookup {
    /// Search the source's own directory for exports. Default true —
    /// the most common layout (`models/character.blend` next to
    /// `models/character.fbx`).
    #[serde(default = "default_true")]
    pub same_dir: bool,
    /// Sibling directory names to also check. The walker rebuilds
    /// candidate paths by joining each sibling name under every
    /// ancestor of the source's parent. Defaults to common
    /// authoring/runtime split conventions.
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
        // Phase 1: 1→1 stem match. .spp pairs with the newest same-stem
        // texture; Phase 2 adds true 1→N for SP's per-channel output.
        m(
            "substance_painter",
            &["spp"],
            &["png", "tga", "jpg", "tif", "tiff", "exr"],
        ),
        m("substance_designer", &["sbs"], &["sbsar", "png", "tga"]),
        m(
            "photoshop",
            &["psd", "psb"],
            &["png", "jpg", "tga", "webp"],
        ),
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

/// Read the file's last-modified time as Unix epoch seconds. Returns
/// `None` for any IO / clock error. We deliberately don't propagate
/// the error — a single unreadable file just means that pair gets
/// silently skipped instead of poisoning the analyze run.
fn read_mtime_secs(path: &str) -> Option<u64> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?;
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Build the list of candidate parent directories an export might
/// live in for a given source. Always includes the source's own
/// directory when `lookup.same_dir`.
///
/// Sibling-dirs semantics: when an ancestor's name matches one of
/// `sibling_dirs`, that ancestor is treated as a "source home" and
/// the export is expected one level up (grandparent). If the user
/// configures multiple sibling names (e.g. `["sources", "exports"]`),
/// the OTHER names are also treated as candidate export sites under
/// the same grandparent — handles the `art/sources/x.blend` ↔
/// `art/exports/x.fbx` layout when both names are listed.
///
/// Examples (with default `sibling_dirs = ["sources", "_source", "src"]`):
/// - `source_parent="/proj/models"` (no match in ancestors) →
///   `["/proj/models"]` (just same_dir, no sibling expansion)
/// - `source_parent="/proj/sources"` (matches "sources") →
///   `["/proj/sources", "/proj", "/proj/_source", "/proj/src"]`
/// - `source_parent="/Characters/Hero/sources"` (matches at depth 1) →
///   `["/Characters/Hero/sources", "/Characters/Hero",
///     "/Characters/Hero/_source", "/Characters/Hero/src"]`
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
        // Walk up from source's parent. At every ancestor whose leaf
        // name matches a configured sibling-dir, treat the GRANDPARENT
        // as a candidate export site, plus the grandparent's other
        // sibling-named subdirs (covers art/sources ↔ art/exports
        // bidirectional layouts).
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
fn mapping_for_source<'a>(
    mappings: &'a [DccMapping],
    ext: &str,
) -> Option<&'a DccMapping> {
    let ext_lower = ext.to_lowercase();
    mappings
        .iter()
        .find(|m| m.sources.iter().any(|s| s.eq_ignore_ascii_case(&ext_lower)))
}

/// Convert a duration in seconds to a short human label
/// ("12s" / "5m" / "3h" / "2d") for use in issue messages.
fn humanize_seconds(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

pub fn find_dcc_source_issues(
    assets: &[AssetInfo],
    config: &DccSourceConfig,
) -> AnalysisResult {
    let mut result = AnalysisResult::new();
    if !config.enabled || config.mappings.is_empty() {
        return result;
    }

    // Index assets by (parent_dir, stem) for O(1) export lookup.
    // Both keys are lowercased so case-insensitive filesystems and
    // mixed-case stems still match. parent_dir is normalized to use
    // forward slashes (scanner already emits these).
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
                    if best.map_or(true, |(_, old)| mtime > old) {
                        best = Some((cand, mtime));
                    }
                }
            }
        }

        let (export, export_mtime) = match best {
            Some(p) => p,
            // No matching export found. Phase 1 stays silent here —
            // it could legitimately be a source the user hasn't
            // exported yet. Phase 2 may add an info-level "no export"
            // signal once we have UI to suppress it cleanly.
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
        result.add_issue(Issue {
            rule_id: "dcc_source.outdated_export".into(),
            rule_name: "Outdated DCC export".into(),
            severity: Severity::Warning,
            message: format!(
                "Source `{}` is {} newer than its export `{}` — possibly missing a re-export.",
                source.name,
                humanize_seconds(diff),
                export.name,
            ),
            asset_path: source.path.clone(),
            suggestion: Some(format!(
                "Re-export from {} and verify the new export's mtime advances past the source. To suppress, add the source path to `[ignore].patterns`.",
                mapping.name,
            )),
            auto_fixable: false,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, AssetType};
    use filetime::{set_file_mtime, FileTime};
    use std::fs;
    use tempfile::tempdir;

    /// Build an AssetInfo for a file path. Only path / name / extension
    /// are exercised by the analyzer's index; the rest can be defaults.
    fn make_asset(path: &str, asset_type: AssetType) -> AssetInfo {
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
    fn write_with_mtime(path: &Path, secs_ago: u64) {
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        cfg.mtime_tolerance_secs = 5;
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert_eq!(r.issues[0].rule_id, "dcc_source.outdated_export");
        assert!(r.issues[0].message.contains("character.blend"));
        assert!(r.issues[0].message.contains("character.fbx"));
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn no_export_candidate_no_issue() {
        // Phase 1 stays silent on orphan sources.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("orphan.blend");
        write_with_mtime(&blend, 60);
        let assets = vec![make_asset(&blend.to_string_lossy(), AssetType::Model)];
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        let r = find_dcc_source_issues(&assets, &cfg);
        // Newest export (.glb) is newer than source → no warning,
        // even though .fbx alone would have triggered.
        assert_eq!(r.issue_count, 0);
    }

    #[test]
    fn sibling_dir_lookup_finds_export() {
        // Layout:
        //   <root>/sources/Hero.blend   (modified 30s ago)
        //   <root>/Hero.fbx             (modified 7200s ago)
        // sibling_dirs contains "sources" by default, so walking up
        // from <root>/sources hits <root> → finds Hero.fbx.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("sources").join("Hero.blend");
        let fbx = dir.path().join("Hero.fbx");
        write_with_mtime(&fbx, 7200);
        write_with_mtime(&blend, 30);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&fbx.to_string_lossy(), AssetType::Model),
        ];
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        cfg.mtime_tolerance_secs = 5;
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0].message.contains("Hero.blend"));
        assert!(r.issues[0].message.contains("Hero.fbx"));
    }

    #[test]
    fn humanize_buckets() {
        assert_eq!(humanize_seconds(30), "30s");
        assert_eq!(humanize_seconds(60), "1m");
        assert_eq!(humanize_seconds(120), "2m");
        assert_eq!(humanize_seconds(7200), "2h");
        assert_eq!(humanize_seconds(86400 * 3), "3d");
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        cfg.mtime_tolerance_secs = 5;
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 2);
        // Path-sorted: ".../a/foo.blend" precedes ".../b/bar.blend".
        let p0 = r.issues[0].asset_path.replace('\\', "/");
        let p1 = r.issues[1].asset_path.replace('\\', "/");
        assert!(p0.contains("/a/"), "expected first issue under /a/, got {p0}");
        assert!(p1.contains("/b/"), "expected second issue under /b/, got {p1}");
    }

    #[test]
    fn same_stem_different_mapping_does_not_pair() {
        // .blend (Blender) should NOT pair with a same-stem .png — png
        // is not in Blender's exports list. Exercises the per-mapping
        // export filter so a Painter .spp doesn't accidentally claim
        // a Blender export's siblings, etc.
        let dir = tempdir().unwrap();
        let blend = dir.path().join("ambiguous.blend");
        let png = dir.path().join("ambiguous.png");
        write_with_mtime(&png, 7200);
        write_with_mtime(&blend, 30);
        let assets = vec![
            make_asset(&blend.to_string_lossy(), AssetType::Model),
            make_asset(&png.to_string_lossy(), AssetType::Texture),
        ];
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        cfg.mtime_tolerance_secs = 5;
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
        let mut cfg = DccSourceConfig::default();
        cfg.enabled = true;
        cfg.mtime_tolerance_secs = 5;
        let r = find_dcc_source_issues(&assets, &cfg);
        assert_eq!(r.issue_count, 1);
        assert!(r.issues[0].suggestion.as_deref().unwrap().contains("photoshop"));
    }
}
