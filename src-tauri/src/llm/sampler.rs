//! Per-directory by-type-ratio sampler for AI Learning.
//!
//! Goal: feed the LLM a representative slice of the project's filenames
//! without uploading the full asset list. Two pressures:
//!
//! 1. **Per-directory quota** — naming/taxonomy conventions are usually
//!    directory-local (`Characters/Hero/T_Hero_*` vs `Weapons/Sword/SM_*`),
//!    so we sample independently in each dir rather than globally.
//! 2. **By-type ratio** — a directory with 100 PNGs and 1 FBX must
//!    surface the FBX, otherwise the model can't learn the FBX naming
//!    convention. We use round-robin allocation: every type with files
//!    in the dir gets at least one slot before any type gets seconds.
//!
//! Determinism: file selection within a (dir, type) bucket is hash-
//! ranked by `(seed, path)` and sorted, so the same scan + same seed
//! produces the same samples — useful for "re-learn" UX where the user
//! expects stable output unless the project actually changed.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};

use crate::scanner::{AssetInfo, AssetType, ScanResult};

use super::learning::{DirectorySample, SampleFile};

/// Sample up to `depth` files per directory, biased so every present
/// asset type gets at least one slot before any type doubles up.
///
/// `seed` controls deterministic file selection within each
/// (dir, type) bucket — same scan + same seed = same samples.
///
/// Returns directories sorted by `rel_path` for stable diff-friendly
/// output.
pub fn sample_directories(scan: &ScanResult, depth: usize, seed: u64) -> Vec<DirectorySample> {
    if depth == 0 || scan.assets.is_empty() {
        return Vec::new();
    }

    let root = &scan.root_path;

    // Group assets by their parent directory. Borrow into the original
    // Vec to avoid copying — we serialize at the end.
    let mut by_dir: HashMap<String, Vec<&AssetInfo>> = HashMap::new();
    for asset in &scan.assets {
        let dir = parent_dir_str(&asset.path);
        by_dir.entry(dir).or_default().push(asset);
    }

    let mut samples: Vec<DirectorySample> = Vec::with_capacity(by_dir.len());
    for (abs_dir, files) in by_dir {
        let rel_path = relative_path(root, &abs_dir);
        let total_files = files.len();
        let picked = pick_per_type(&files, depth, seed);
        let sample_files: Vec<SampleFile> = picked
            .into_iter()
            .map(|a| SampleFile {
                filename: a.name.clone(),
                extension: a.extension.clone(),
                asset_type: type_to_str(&a.asset_type).to_string(),
            })
            .collect();
        samples.push(DirectorySample {
            rel_path,
            total_files,
            files: sample_files,
        });
    }
    samples.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    samples
}

/// Allocate `depth` slots across a directory's assets, then deterministically
/// pick which file fills each slot.
///
/// Allocation: round-robin by type so a dir with 100 PNGs + 1 FBX
/// surfaces the FBX. Within a type, files are sorted by hash rank
/// so selection is stable across runs with the same seed.
fn pick_per_type<'a>(files: &[&'a AssetInfo], depth: usize, seed: u64) -> Vec<&'a AssetInfo> {
    if files.len() <= depth {
        // Take everything — sort by rank for output stability.
        let mut copy: Vec<&AssetInfo> = files.iter().copied().collect();
        copy.sort_by_key(|a| (rank(seed, &a.path), a.path.clone()));
        return copy;
    }

    // Bucket by asset type.
    let mut by_type: HashMap<&AssetType, Vec<&AssetInfo>> = HashMap::new();
    for &a in files {
        by_type.entry(&a.asset_type).or_default().push(a);
    }
    // Sort each bucket deterministically.
    for bucket in by_type.values_mut() {
        bucket.sort_by_key(|a| (rank(seed, &a.path), a.path.clone()));
    }
    // Sort types themselves so iteration order is stable across runs.
    // We use the lowercase string repr from `type_to_str` because
    // `AssetType` doesn't impl `Ord`.
    let mut types: Vec<&AssetType> = by_type.keys().copied().collect();
    types.sort_by_key(|t| type_to_str(t));

    // Round-robin allocation: walk types repeatedly, give one slot per
    // pass to any type that still has files left. Stops when `depth`
    // slots are filled OR every type is exhausted.
    let mut quota: HashMap<&AssetType, usize> = HashMap::new();
    let mut filled = 0usize;
    'outer: loop {
        let mut progressed = false;
        for t in &types {
            let bucket = by_type.get(*t).unwrap();
            let used = *quota.get(*t).unwrap_or(&0);
            if used < bucket.len() {
                *quota.entry(*t).or_insert(0) += 1;
                filled += 1;
                progressed = true;
                if filled == depth {
                    break 'outer;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    // Materialize the picks: for each type take its first `quota[t]`
    // files (already sorted above).
    let mut picks: Vec<&AssetInfo> = Vec::with_capacity(filled);
    for t in &types {
        let bucket = by_type.get(*t).unwrap();
        let n = *quota.get(*t).unwrap_or(&0);
        picks.extend(bucket.iter().copied().take(n));
    }
    picks
}

fn rank(seed: u64, path: &str) -> u64 {
    let mut h = DefaultHasher::new();
    seed.hash(&mut h);
    path.hash(&mut h);
    h.finish()
}

/// Parent directory string. Backend paths are forward-slash normalized
/// per `scanner::path_to_string`, so `rsplit_once('/')` is enough.
/// Returns empty string for root-level files.
fn parent_dir_str(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(d, _)| d.to_string())
        .unwrap_or_default()
}

/// Strip `root` prefix from `abs_dir`. If `abs_dir == root` (assets at
/// project root) returns "". Defensive on missing prefix → returns the
/// original; happens only on path-normalization bugs.
fn relative_path(root: &str, abs_dir: &str) -> String {
    let root = root.trim_end_matches('/');
    if abs_dir == root {
        return String::new();
    }
    let prefix = format!("{root}/");
    abs_dir.strip_prefix(&prefix).unwrap_or(abs_dir).to_string()
}

fn type_to_str(t: &AssetType) -> &'static str {
    match t {
        AssetType::Texture => "texture",
        AssetType::Model => "model",
        AssetType::Audio => "audio",
        AssetType::Video => "video",
        AssetType::Animation => "animation",
        AssetType::Material => "material",
        AssetType::Prefab => "prefab",
        AssetType::Scene => "scene",
        AssetType::Script => "script",
        AssetType::Data => "data",
        AssetType::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetMetadata, DirectoryNode};

    fn asset(path: &str, asset_type: AssetType) -> AssetInfo {
        AssetInfo {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            extension: path.rsplit('.').next().unwrap_or("").to_string(),
            asset_type,
            size: 0,
            metadata: Some(AssetMetadata::default()),
            unity_guid: None,
        }
    }

    fn scan(root: &str, assets: Vec<AssetInfo>) -> ScanResult {
        ScanResult {
            root_path: root.to_string(),
            directory_tree: DirectoryNode {
                name: "".into(),
                path: root.into(),
                children: vec![],
                file_count: 0,
                total_size: 0,
            },
            total_count: assets.len(),
            total_size: 0,
            type_counts: HashMap::new(),
            project_type: None,
            assets,
        }
    }

    #[test]
    fn empty_scan_returns_no_samples() {
        let s = scan("/proj", vec![]);
        assert!(sample_directories(&s, 5, 0).is_empty());
    }

    #[test]
    fn depth_zero_returns_no_samples() {
        let s = scan("/proj", vec![asset("/proj/a.png", AssetType::Texture)]);
        assert!(sample_directories(&s, 0, 0).is_empty());
    }

    #[test]
    fn small_dir_takes_everything() {
        let s = scan(
            "/proj",
            vec![
                asset("/proj/a.png", AssetType::Texture),
                asset("/proj/b.png", AssetType::Texture),
                asset("/proj/c.fbx", AssetType::Model),
            ],
        );
        let samples = sample_directories(&s, 5, 0);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].rel_path, ""); // root-level
        assert_eq!(samples[0].total_files, 3);
        assert_eq!(samples[0].files.len(), 3);
    }

    #[test]
    fn skewed_dir_round_robins_so_minority_type_appears() {
        // 100 PNGs + 1 FBX; depth = 3 should still pick the FBX.
        let mut assets = Vec::new();
        for i in 0..100 {
            assets.push(asset(
                &format!("/proj/textures/t_{i}.png"),
                AssetType::Texture,
            ));
        }
        assets.push(asset("/proj/textures/lone.fbx", AssetType::Model));
        let s = scan("/proj", assets);
        let samples = sample_directories(&s, 3, 42);
        assert_eq!(samples.len(), 1);
        let types: Vec<&str> = samples[0]
            .files
            .iter()
            .map(|f| f.asset_type.as_str())
            .collect();
        assert!(types.contains(&"model"), "expected fbx picked, got {types:?}");
        assert!(
            types.contains(&"texture"),
            "expected at least one png picked, got {types:?}"
        );
        assert_eq!(samples[0].files.len(), 3);
        assert_eq!(samples[0].total_files, 101);
    }

    #[test]
    fn balanced_dir_distributes_evenly() {
        let assets = vec![
            asset("/proj/a/1.png", AssetType::Texture),
            asset("/proj/a/2.png", AssetType::Texture),
            asset("/proj/a/3.png", AssetType::Texture),
            asset("/proj/a/4.fbx", AssetType::Model),
            asset("/proj/a/5.fbx", AssetType::Model),
            asset("/proj/a/6.wav", AssetType::Audio),
        ];
        let s = scan("/proj", assets);
        let samples = sample_directories(&s, 3, 0);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].files.len(), 3);
        // Round-robin should give one of each type before any second.
        let types: Vec<&str> = samples[0]
            .files
            .iter()
            .map(|f| f.asset_type.as_str())
            .collect();
        assert!(types.contains(&"texture"));
        assert!(types.contains(&"model"));
        assert!(types.contains(&"audio"));
    }

    #[test]
    fn multiple_dirs_sampled_independently() {
        let assets = vec![
            asset("/proj/chars/hero.png", AssetType::Texture),
            asset("/proj/chars/villain.png", AssetType::Texture),
            asset("/proj/weapons/sword.fbx", AssetType::Model),
        ];
        let s = scan("/proj", assets);
        let samples = sample_directories(&s, 2, 0);
        assert_eq!(samples.len(), 2);
        // Sorted by rel_path
        assert_eq!(samples[0].rel_path, "chars");
        assert_eq!(samples[1].rel_path, "weapons");
    }

    #[test]
    fn deterministic_for_same_seed() {
        let assets = (0..20)
            .map(|i| asset(&format!("/proj/d/f_{i}.png"), AssetType::Texture))
            .collect();
        let s = scan("/proj", assets);
        let a = sample_directories(&s, 5, 42);
        let b = sample_directories(&s, 5, 42);
        assert_eq!(
            a[0].files
                .iter()
                .map(|f| f.filename.clone())
                .collect::<Vec<_>>(),
            b[0].files
                .iter()
                .map(|f| f.filename.clone())
                .collect::<Vec<_>>()
        );
        // Different seed → different selection (with high probability for 20 items)
        let c = sample_directories(&s, 5, 999);
        let names_a: Vec<_> = a[0].files.iter().map(|f| &f.filename).collect();
        let names_c: Vec<_> = c[0].files.iter().map(|f| &f.filename).collect();
        assert_ne!(names_a, names_c, "different seeds should yield different picks");
    }

    #[test]
    fn rel_path_strips_project_root() {
        let s = scan(
            "/Users/me/projects/cyber-rpg",
            vec![asset(
                "/Users/me/projects/cyber-rpg/Characters/Hero/T_Hero.png",
                AssetType::Texture,
            )],
        );
        let samples = sample_directories(&s, 5, 0);
        assert_eq!(samples[0].rel_path, "Characters/Hero");
    }
}
