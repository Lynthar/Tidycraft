//! Heuristic tag suggestions.
//!
//! `TagSuggester` is a trait so we can later plug in an external multimodal LLM
//! (`ExternalLLMSuggester` placeholder) without touching the call site. Phase 5
//! ships only `HeuristicSuggester` — it groups assets by:
//!
//! 1. **Filename token**: split filenames on `_`/`-`/space/dot/caseBoundary,
//!    drop noise, and surface tokens that recur across ≥3 assets.
//! 2. **Dimension bucket** (textures only): bucket by nearest power-of-two of
//!    `max(w,h)`, surface buckets covering ≥5 assets.
//! 3. **Path segment**: any directory component (other than the project root
//!    or generic container names) that appears in ≥3 asset paths.
//!
//! Groups are deduped by `name` (taking max confidence, union of file_paths,
//! and preferring higher-priority hints), sorted by confidence desc, and capped
//! at 12 entries.
//!
//! The frontend asks once per session via `suggest_tags`. No persistence — when
//! the user hits Apply we go through the existing `create_tag` +
//! `add_tag_to_assets` flow, so the suggestion side never owns tag state.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::scanner::{AssetType, ScanResult};

#[derive(Debug, Clone, Serialize)]
pub struct TagGroup {
    pub name: String,
    pub color: String,
    pub file_paths: Vec<String>,
    pub confidence: f32,
    pub hint: String,
    /// Up to `SAMPLE_FILENAMES` filename stems from `file_paths`, picked
    /// alphabetically for stability. Lets the UI show "matched 14 files:
    /// tree_*, bush_*, grass_*" without making the user click into the
    /// preview to inspect.
    pub samples: Vec<String>,
}

pub trait TagSuggester {
    fn suggest(&self, scan: &ScanResult) -> Vec<TagGroup>;
}

pub struct HeuristicSuggester;

const MIN_TOKEN_HITS: usize = 3;
const MIN_DIM_HITS: usize = 5;
const MIN_PATH_HITS: usize = 3;
const MAX_GROUPS: usize = 12;
const SAMPLE_FILENAMES: usize = 3;

const HINT_FILENAME: &str = "filename token";
const HINT_DIMENSION: &str = "dimension";
const HINT_PATH: &str = "path segment";

/// Tokens that recur across many filenames but carry no taxonomic value.
const TOKEN_STOPLIST: &[&str] = &[
    // file-format-ish noise
    "tex", "texture", "model", "asset", "assets", "img", "image",
    "obj", "fbx", "png", "jpg", "jpeg", "tga", "wav", "mp3", "ogg",
    // workflow noise
    "default", "new", "old", "copy", "tmp", "temp", "test", "draft",
    "final", "low", "high", "lod", "lod0", "lod1", "lod2",
];

/// Directory names that appear at the top of nearly every project and don't
/// help narrow anything down. Compared case-insensitively.
const PATH_STOPLIST: &[&str] = &[
    "assets", "content", "src", "source", "sources", "resources",
    "data", "files", "project", "projects", "import", "imports",
];

/// PBR channel-role recognition for the dimension+channel tag source.
/// Strict suffix matches — the segment after the LAST `_` in the stem
/// must equal one of these aliases (case-insensitively). Single-letter
/// suffixes (`_n`, `_r`, `_m`) are deliberately omitted because they
/// collide too readily with non-PBR tokens (`item_n` for "item N",
/// `arrow_r` for "right", …); users on a strict pipeline can extend
/// this constant if their team's convention uses them.
///
/// Tuple is `(canonical_role, alias)`. Multiple aliases map to the same
/// canonical role so `_BaseColor`, `_Albedo`, and `_Diffuse` all land
/// in the same group.
const KNOWN_CHANNEL_SUFFIXES: &[(&str, &str)] = &[
    ("BaseColor", "BaseColor"),
    ("BaseColor", "Albedo"),
    ("BaseColor", "Diffuse"),
    ("BaseColor", "Color"),
    ("Normal", "Normal"),
    ("Normal", "Norm"),
    ("Roughness", "Roughness"),
    ("Roughness", "Rough"),
    ("Metallic", "Metallic"),
    ("Metallic", "Metal"),
    ("AO", "AO"),
    ("AO", "AmbientOcclusion"),
    ("Emissive", "Emissive"),
    ("Emissive", "Emission"),
    ("Height", "Height"),
    ("Height", "Disp"),
    ("ORM", "ORM"),
    ("MRA", "MRA"),
    ("RMA", "RMA"),
];

/// Palette for suggested tag colors. Picked to look good in Forge Dark and
/// stay distinguishable in clusters of 4-8 dots. Index by a stable hash of
/// the group name so the same suggestion lands on the same color across runs.
const PALETTE: &[&str] = &[
    "#d4924a", // amber
    "#5fb8a0", // jade
    "#b588d0", // violet
    "#5fa6cf", // cyan
    "#c9a558", // ochre
    "#8088c4", // indigo
    "#c87aa8", // fuchsia
    "#6589c7", // azure
];

fn pick_color(name: &str) -> String {
    // Tiny FNV-1a-ish hash; good enough for stable palette indexing.
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    PALETTE[(h as usize) % PALETTE.len()].to_string()
}

/// Strip extension from filename.
fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// True if `ch` is a CJK ideograph or Japanese kana — the ranges most
/// likely to appear in game-asset filenames out of mainland CN / JP /
/// TW shops. We use this for tokenizer boundary detection so a name
/// like `Hero角色` splits into `["hero", "角色"]` instead of one
/// runaway token. Korean Hangul could be added if it ever comes up.
fn is_cjk(ch: char) -> bool {
    let code = ch as u32;
    (0x4E00..=0x9FFF).contains(&code)        // CJK Unified Ideographs
        || (0x3400..=0x4DBF).contains(&code) // CJK Extension A
        || (0x3040..=0x309F).contains(&code) // Hiragana
        || (0x30A0..=0x30FF).contains(&code) // Katakana
}

/// Split a filename stem into normalized lowercase tokens. Recognizes
/// `_`/`-`/`.`/space separators, camelCase / PascalCase boundaries,
/// and the ASCII↔CJK transition.
///
/// Limitation: continuous CJK runs (`角色主角.png`) stay as one token.
/// True Chinese tokenization needs a dictionary (`jieba-rs` etc.) which
/// would balloon the binary by ~5MB; not worth it for an opt-in
/// suggestion feature. Users on connected-CJK projects who want finer
/// grouping can insert `_` between meaningful units in filenames, which
/// is best practice anyway.
fn tokenize_stem(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    let mut prev_cjk = false;

    let flush = |buf: &mut String, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(std::mem::take(buf));
        }
    };

    for ch in s.chars() {
        if ch == '_' || ch == '-' || ch == '.' || ch == ' ' {
            flush(&mut current, &mut tokens);
            prev_lower = false;
            prev_cjk = false;
            continue;
        }
        let curr_cjk = is_cjk(ch);
        // ASCII↔CJK boundary: split here so `Hero角色` → `["hero", "角色"]`
        // rather than collapsing into one untranslatable token.
        if !current.is_empty() && curr_cjk != prev_cjk {
            flush(&mut current, &mut tokens);
        }
        if ch.is_uppercase() && prev_lower {
            // camelCase boundary: emit current token, start fresh.
            flush(&mut current, &mut tokens);
        }
        current.push(ch.to_ascii_lowercase());
        prev_lower = ch.is_lowercase();
        prev_cjk = curr_cjk;
    }
    flush(&mut current, &mut tokens);
    tokens
}

fn is_useful_token(token: &str) -> bool {
    // `chars().count()` instead of `len()` because byte length lies for
    // multi-byte characters: a single CJK ideograph has byte length 3
    // and would slip past `len() < 2`. Single-character tokens (whether
    // `"a"` or `"中"`) carry near-zero taxonomic value, drop them.
    if token.chars().count() < 2 {
        return false;
    }
    if token.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // "v01", "v2", … — version markers don't help group assets.
    if token.starts_with('v')
        && token[1..].chars().all(|c| c.is_ascii_digit())
        && token.len() <= 4
    {
        return false;
    }
    if TOKEN_STOPLIST.iter().any(|s| *s == token) {
        return false;
    }
    true
}

/// Find the nearest power of two ≤ `n`. Returns `None` if `n == 0` or the
/// result would be smaller than 256 (a 128px texture is too small to care
/// about as a category).
fn dimension_bucket(w: u32, h: u32) -> Option<u32> {
    let max = w.max(h);
    if max == 0 {
        return None;
    }
    // Largest power of two <= max. Computed via `leading_zeros` rather than the
    // old `while p.saturating_mul(2) <= max` loop, which spun forever when
    // max == u32::MAX: `saturating_mul(2)` pins p at u32::MAX, so the condition
    // stays true every iteration. A corrupt DDS header reporting 0xFFFFFFFF
    // dimensions could trigger that infinite loop while holding the project lock.
    // `max >= 1` here (0 returned above), so the shift distance is 0..=31.
    let p: u32 = 1 << (31 - max.leading_zeros());
    if p < 256 {
        return None;
    }
    Some(p)
}

/// Detect a PBR channel role from a texture's filename stem. Strict
/// `_<suffix>` matching: the substring after the LAST `_` must equal
/// one of `KNOWN_CHANNEL_SUFFIXES`'s aliases (case-insensitive).
/// Returns the canonical role label (already capitalized for direct
/// tag-name display) or None when no PBR suffix is present.
fn parse_channel(stem: &str) -> Option<&'static str> {
    let last_underscore = stem.rfind('_')?;
    let suffix = &stem[last_underscore + 1..];
    if suffix.is_empty() {
        return None;
    }
    let suffix_lower = suffix.to_lowercase();
    for (canonical, alias) in KNOWN_CHANNEL_SUFFIXES {
        if alias.to_lowercase() == suffix_lower {
            return Some(canonical);
        }
    }
    None
}

/// Capitalize the first byte (ASCII-safe; non-ASCII pass through unchanged).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Compute relative path components below the project root, excluding the
/// final filename component.
fn relative_dirs<'a>(asset_path: &'a str, root: &str) -> Vec<&'a str> {
    let rel = asset_path.strip_prefix(root).unwrap_or(asset_path);
    let rel = rel.trim_start_matches('/');
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop(); // drop filename
    parts
}

impl TagSuggester for HeuristicSuggester {
    fn suggest(&self, scan: &ScanResult) -> Vec<TagGroup> {
        if scan.assets.is_empty() {
            return Vec::new();
        }

        // Adaptive thresholds: tiny projects (< 30 assets) would
        // otherwise produce zero suggestions because every group falls
        // below the noise filter. Loosen so users still see something;
        // larger projects keep the stricter baselines that matter when
        // the result list could otherwise drown in tokens.
        let total_assets = scan.assets.len();
        let (min_token_hits, min_dim_hits, min_path_hits) = if total_assets < 30 {
            (2usize, 3usize, 2usize)
        } else {
            (MIN_TOKEN_HITS, MIN_DIM_HITS, MIN_PATH_HITS)
        };

        // name → (paths, hint). Lower-priority hits append to the same key but
        // don't overwrite the hint; the first hint stays. Ordering of source
        // passes (filename → dimension → path) implicitly sets priority.
        let mut groups: HashMap<String, (HashSet<String>, String)> = HashMap::new();

        // ----- Source 1: filename tokens -----
        let mut token_assets: HashMap<String, HashSet<String>> = HashMap::new();
        for asset in &scan.assets {
            let stem_str = stem(&asset.name);
            let mut seen_in_this_file: HashSet<String> = HashSet::new();
            for tok in tokenize_stem(stem_str) {
                if !is_useful_token(&tok) {
                    continue;
                }
                if seen_in_this_file.insert(tok.clone()) {
                    token_assets
                        .entry(tok)
                        .or_default()
                        .insert(asset.path.clone());
                }
            }
        }
        for (token, paths) in token_assets {
            if paths.len() < min_token_hits {
                continue;
            }
            let entry = groups
                .entry(capitalize(&token))
                .or_insert_with(|| (HashSet::new(), HINT_FILENAME.to_string()));
            entry.0.extend(paths);
        }

        // ----- Source 2: texture dimension + optional channel role -----
        // Each texture lands in exactly one bucket, keyed by either
        // `"{bucket}px {channel}"` or fallback `"{bucket}px"` when the
        // stem doesn't match a known PBR suffix. The channel-aware
        // variant gives the user actionable groups like "1024px
        // BaseColor" instead of dumping albedos and normal maps into
        // a single "1024px" pile.
        let mut dim_assets: HashMap<String, HashSet<String>> = HashMap::new();
        for asset in &scan.assets {
            if !matches!(asset.asset_type, AssetType::Texture) {
                continue;
            }
            let (w, h) = match asset
                .metadata
                .as_ref()
                .and_then(|m| Some((m.width?, m.height?)))
            {
                Some(d) => d,
                None => continue,
            };
            if let Some(bucket) = dimension_bucket(w, h) {
                let stem_str = stem(&asset.name);
                let key = match parse_channel(stem_str) {
                    Some(role) => format!("{}px {}", bucket, role),
                    None => format!("{}px", bucket),
                };
                dim_assets
                    .entry(key)
                    .or_default()
                    .insert(asset.path.clone());
            }
        }
        for (key, paths) in dim_assets {
            if paths.len() < min_dim_hits {
                continue;
            }
            let entry = groups
                .entry(key)
                .or_insert_with(|| (HashSet::new(), HINT_DIMENSION.to_string()));
            entry.0.extend(paths);
        }

        // ----- Source 3: path segments -----
        let mut path_assets: HashMap<String, HashSet<String>> = HashMap::new();
        for asset in &scan.assets {
            for seg in relative_dirs(&asset.path, &scan.root_path) {
                let lower = seg.to_lowercase();
                if lower.is_empty() {
                    continue;
                }
                if PATH_STOPLIST.iter().any(|s| *s == lower) {
                    continue;
                }
                path_assets
                    .entry(seg.to_string())
                    .or_default()
                    .insert(asset.path.clone());
            }
        }
        for (seg, paths) in path_assets {
            if paths.len() < min_path_hits {
                continue;
            }
            // Path-segment names get the segment as-is (it's already
            // human-readable like "Characters" or "Props"), but if it
            // collides with a filename-token group, the existing hint wins.
            let name = capitalize(&seg);
            let entry = groups
                .entry(name)
                .or_insert_with(|| (HashSet::new(), HINT_PATH.to_string()));
            entry.0.extend(paths);
        }

        // ----- Build TagGroups + sort + cap -----
        let total = scan.assets.len() as f32;
        let mut out: Vec<TagGroup> = groups
            .into_iter()
            .map(|(name, (paths, hint))| {
                let count = paths.len();
                let mut file_paths: Vec<String> = paths.into_iter().collect();
                file_paths.sort();
                let samples: Vec<String> = file_paths
                    .iter()
                    .take(SAMPLE_FILENAMES)
                    .map(|p| {
                        // Filename only — full paths are noisy in tight UI.
                        Path::new(p)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(p)
                            .to_string()
                    })
                    .collect();
                let confidence = ((count as f32) / total).min(1.0);
                let color = pick_color(&name);
                TagGroup {
                    name,
                    color,
                    file_paths,
                    confidence,
                    hint,
                    samples,
                }
            })
            .collect();

        out.sort_by(|a, b| {
            // Higher confidence first; tie-break by larger group, then by name.
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.file_paths.len().cmp(&a.file_paths.len()))
                .then_with(|| a.name.cmp(&b.name))
        });
        out.truncate(MAX_GROUPS);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetInfo, AssetMetadata, AssetType, DirectoryNode};

    fn fixture_scan(assets: Vec<AssetInfo>) -> ScanResult {
        ScanResult {
            root_path: "/proj".to_string(),
            directory_tree: DirectoryNode {
                name: "proj".to_string(),
                path: "/proj".to_string(),
                children: vec![],
                file_count: assets.len(),
                total_size: 0,
            },
            total_count: assets.len(),
            total_size: 0,
            type_counts: std::collections::HashMap::new(),
            project_type: None,
            assets,
        }
    }

    fn asset(path: &str, name: &str, ty: AssetType) -> AssetInfo {
        AssetInfo {
            path: path.to_string(),
            name: name.to_string(),
            extension: name.rsplit('.').next().unwrap_or("").to_string(),
            asset_type: ty,
            size: 0,
            metadata: None,
            unity_guid: None,
        }
    }

    fn texture(path: &str, name: &str, w: u32, h: u32) -> AssetInfo {
        let mut a = asset(path, name, AssetType::Texture);
        a.metadata = Some(AssetMetadata {
            width: Some(w),
            height: Some(h),
            ..Default::default()
        });
        a
    }

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_stem("foo_bar"), vec!["foo", "bar"]);
        assert_eq!(tokenize_stem("tex-diffuse-01"), vec!["tex", "diffuse", "01"]);
        assert_eq!(tokenize_stem("PlayerController"), vec!["player", "controller"]);
        assert_eq!(tokenize_stem("Shield Hero"), vec!["shield", "hero"]);
    }

    #[test]
    fn token_filter_drops_noise() {
        assert!(!is_useful_token("a"));
        assert!(!is_useful_token("01"));
        assert!(!is_useful_token("v01"));
        assert!(!is_useful_token("tex"));
        assert!(is_useful_token("hero"));
        assert!(is_useful_token("diffuse"));
    }

    #[test]
    fn tokenize_splits_at_ascii_cjk_boundary() {
        assert_eq!(tokenize_stem("Hero角色"), vec!["hero", "角色"]);
        assert_eq!(tokenize_stem("角色Hero"), vec!["角色", "hero"]);
        assert_eq!(tokenize_stem("武器Sword剑"), vec!["武器", "sword", "剑"]);
    }

    #[test]
    fn tokenize_continuous_cjk_kept_together() {
        // Documented limitation — without a CJK dictionary we can't
        // split connected ideographs into meaning-bearing units.
        assert_eq!(tokenize_stem("角色主角"), vec!["角色主角"]);
    }

    #[test]
    fn token_filter_drops_single_cjk() {
        // `len()` is byte length; a single ideograph has byte length 3
        // so the old `len() < 2` check let it through. The fix uses
        // `chars().count()` which correctly counts a single ideograph
        // as 1 character and filters it.
        assert!(!is_useful_token("中"));
        assert!(!is_useful_token("旧"));
        assert!(is_useful_token("主角"));
        assert!(is_useful_token("武器"));
    }

    #[test]
    fn small_project_uses_loose_thresholds() {
        // 6 assets with `hero` appearing 2 times: below the baseline of
        // 3, but above the adaptive floor of 2. User shouldn't open the
        // panel and see nothing on a small project.
        let scan = fixture_scan(vec![
            asset("/proj/a/hero_red.png", "hero_red.png", AssetType::Texture),
            asset("/proj/a/hero_blue.png", "hero_blue.png", AssetType::Texture),
            asset("/proj/a/sword.png", "sword.png", AssetType::Texture),
            asset("/proj/a/shield.png", "shield.png", AssetType::Texture),
            asset("/proj/a/potion.png", "potion.png", AssetType::Texture),
        ]);
        let groups = HeuristicSuggester.suggest(&scan);
        assert!(
            groups.iter().any(|g| g.name == "Hero"),
            "Expected `Hero` to surface on a 5-asset project under adaptive thresholds"
        );
    }

    #[test]
    fn dim_bucket_pot() {
        assert_eq!(dimension_bucket(1024, 1024), Some(1024));
        assert_eq!(dimension_bucket(2048, 1024), Some(2048));
        assert_eq!(dimension_bucket(1500, 1500), Some(1024));
        assert_eq!(dimension_bucket(128, 128), None); // too small
        assert_eq!(dimension_bucket(0, 0), None);
    }

    #[test]
    fn filename_source_groups_by_token() {
        let scan = fixture_scan(vec![
            asset("/proj/a/hero_diffuse.png", "hero_diffuse.png", AssetType::Texture),
            asset("/proj/a/hero_normal.png", "hero_normal.png", AssetType::Texture),
            asset("/proj/a/hero_specular.png", "hero_specular.png", AssetType::Texture),
            asset("/proj/b/villain_one.png", "villain_one.png", AssetType::Texture),
        ]);
        let groups = HeuristicSuggester.suggest(&scan);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        // "hero" hits 3 → in. "villain" hits 1 → below MIN_TOKEN_HITS.
        assert!(names.contains(&"Hero"));
        assert!(!names.contains(&"Villain"));
    }

    #[test]
    fn parse_channel_recognizes_aliases() {
        assert_eq!(parse_channel("T_Wood_BaseColor"), Some("BaseColor"));
        assert_eq!(parse_channel("T_Wood_Albedo"), Some("BaseColor"));
        assert_eq!(parse_channel("T_Wood_normal"), Some("Normal"));
        assert_eq!(parse_channel("T_Wood_ORM"), Some("ORM"));
        // Unrecognized suffix → None (so the texture falls back to the
        // bucket-only "1024px" group rather than getting mis-labeled).
        assert_eq!(parse_channel("T_brand_new"), None);
        // No underscore at all → None.
        assert_eq!(parse_channel("foo"), None);
        // Trailing underscore → empty suffix → None.
        assert_eq!(parse_channel("foo_"), None);
    }

    #[test]
    fn dim_with_channel_groups_separately() {
        // 6 BaseColor textures + 6 Normal textures at 1024px should
        // produce two distinct groups, not one merged "1024px" pile.
        let mut assets = Vec::new();
        for i in 0..6 {
            assets.push(texture(
                &format!("/proj/T_Stone{}_BaseColor.png", i),
                &format!("T_Stone{}_BaseColor.png", i),
                1024,
                1024,
            ));
        }
        for i in 0..6 {
            assets.push(texture(
                &format!("/proj/T_Stone{}_Normal.png", i),
                &format!("T_Stone{}_Normal.png", i),
                1024,
                1024,
            ));
        }
        let scan = fixture_scan(assets);
        let groups = HeuristicSuggester.suggest(&scan);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"1024px BaseColor"));
        assert!(names.contains(&"1024px Normal"));
        // The catch-all "1024px" group should NOT appear when every
        // texture had a recognized channel.
        assert!(!names.contains(&"1024px"));
    }

    #[test]
    fn dim_falls_back_when_no_channel() {
        // Generic filenames without PBR suffixes still group by raw
        // size so projects without a PBR pipeline aren't left without
        // any dimension-based suggestion.
        let mut assets = Vec::new();
        for i in 0..6 {
            assets.push(texture(
                &format!("/proj/icon_{}.png", i),
                &format!("icon_{}.png", i),
                512,
                512,
            ));
        }
        let scan = fixture_scan(assets);
        let groups = HeuristicSuggester.suggest(&scan);
        assert!(groups.iter().any(|g| g.name == "512px"));
    }

    #[test]
    fn dimension_source_groups_textures() {
        let mut assets = Vec::new();
        for i in 0..6 {
            assets.push(texture(
                &format!("/proj/t/big_{}.png", i),
                &format!("big_{}.png", i),
                1024,
                1024,
            ));
        }
        let scan = fixture_scan(assets);
        let groups = HeuristicSuggester.suggest(&scan);
        assert!(groups.iter().any(|g| g.name == "1024px" && g.hint == HINT_DIMENSION));
    }

    #[test]
    fn path_source_groups_by_dir() {
        let scan = fixture_scan(vec![
            asset("/proj/Characters/hero.fbx", "hero.fbx", AssetType::Model),
            asset("/proj/Characters/villain.fbx", "villain.fbx", AssetType::Model),
            asset("/proj/Characters/npc.fbx", "npc.fbx", AssetType::Model),
            asset("/proj/Props/barrel.fbx", "barrel.fbx", AssetType::Model),
        ]);
        let groups = HeuristicSuggester.suggest(&scan);
        let chars = groups.iter().find(|g| g.name == "Characters");
        assert!(chars.is_some(), "Expected `Characters` group");
        assert_eq!(chars.unwrap().hint, HINT_PATH);
        // Props only has 1 → below MIN_PATH_HITS.
        assert!(groups.iter().all(|g| g.name != "Props"));
    }

    #[test]
    fn path_stoplist_skips_generic_dirs() {
        let scan = fixture_scan(vec![
            asset("/proj/Assets/a.png", "a.png", AssetType::Texture),
            asset("/proj/Assets/b.png", "b.png", AssetType::Texture),
            asset("/proj/Assets/c.png", "c.png", AssetType::Texture),
        ]);
        let groups = HeuristicSuggester.suggest(&scan);
        assert!(groups.iter().all(|g| g.name.to_lowercase() != "assets"));
    }

    #[test]
    fn caps_at_max_groups() {
        let mut assets = Vec::new();
        for i in 0..MAX_GROUPS + 5 {
            // Each "groupN_token" appears 3 times under its own dir, producing
            // multiple groups well above the cap.
            for j in 0..3 {
                let p = format!("/proj/dir{}/groupNtoken{}_{}.png", i, i, j);
                let n = format!("groupNtoken{}_{}.png", i, j);
                assets.push(asset(&p, &n, AssetType::Texture));
            }
        }
        let scan = fixture_scan(assets);
        let groups = HeuristicSuggester.suggest(&scan);
        assert!(groups.len() <= MAX_GROUPS);
    }
}
