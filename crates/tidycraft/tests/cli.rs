//! End to end: the real `tidycraft` binary over a real Unity project tree.
//! Every expectation here comes from docs/analyzer-rules.md and the fixture's
//! own tidycraft.toml, never from what the tool printed last time.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/unity-project");
const RUNS: usize = 3;

/// (rule, root-relative path): one finding per file, each file built to trip
/// exactly that rule at the thresholds in the fixture's tidycraft.toml.
const EXPECTED: &[(&str, &str)] = &[
    // [naming]: max_length 24, forbid_chinese, texture_prefix "T_"; forbidden characters are on by default
    (
        "naming.length",
        "Assets/Textures/T_AVeryLongTextureNameThatExceedsTheLimit.png",
    ),
    ("naming.forbidden_char", "Assets/Textures/T_Bad Name.png"),
    ("naming.chinese", "Assets/Textures/T_中文.png"),
    ("naming.prefix", "Assets/Textures/Rock.png"),
    // [texture]: require_pot, max_size 16, min_size 4, warn_non_square, max_file_size 1000
    ("texture.pot", "Assets/Textures/T_NonPot.png"), // 6x6
    ("texture.min_size", "Assets/Textures/T_Tiny.png"), // 2x2
    ("texture.max_size", "Assets/Textures/T_Huge.png"), // 32x32
    ("texture.non_square", "Assets/Textures/T_Wide.png"), // 8x4
    ("texture.file_size", "Assets/Textures/T_Heavy.png"), // 16x16 of noise, 1108 bytes
    // [texture.color_space] is on by default: an sRGB chunk under a `_normal` stem
    ("texture.color_space", "Assets/Textures/T_Rock_normal.png"),
    // [pbr_set]: T_Hero has a BaseColor and no Normal; T_Rock has both
    ("pbr_set.incomplete", "Assets/Textures/T_Hero_BaseColor.png"),
    // duplicate is always on: T_Dup_A and T_Dup_B are byte-identical, the issue anchors on the copy
    ("duplicate", "Assets/Textures/T_Dup_B.png"),
    // [model]: max_vertices 10, max_faces 12, max_materials 1
    ("model.vertices", "Assets/Models/SM_Dense.obj"), // 12 v
    ("model.faces", "Assets/Models/SM_ManyFaces.obj"), // 14 f
    ("model.materials", "Assets/Models/SM_TwoMats.obj"), // 2 newmtl
    // [audio]: 44100/48000 only, max_sfx_duration 0.5, prefer_mono_for_sfx, max_file_size 150000
    ("audio.sample_rate", "Assets/Audio/SFX_LowRate.wav"), // 22050 Hz
    ("audio.sfx_duration", "Assets/Audio/SFX_Long.wav"),   // 1.0 s
    ("audio.stereo_sfx", "Assets/Audio/SFX_Stereo.wav"),
    ("audio.file_size", "Assets/Audio/Ambient_Big.wav"), // 176444 bytes
    // missing_reference is always on for Unity: M_Broken.mat points at a guid no .meta carries
    ("missing_reference", "Assets/Materials/M_Broken.mat"),
];

/// The one cross-asset rule that needs a clock: `T_Rock_BaseColor.spp` in
/// `sources/` pairs with the `.png` beside it once its mtime is newer.
const STALE_SOURCE: (&str, &str) = (
    "dcc_source.outdated_export",
    "Assets/Textures/sources/T_Rock_BaseColor.spp",
);

struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

/// A private copy of the fixture: the binary must never scan the checked-in
/// tree, and the stale-source test needs to touch a file.
fn project() -> Project {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path().join("unity-project");
    copy_tree(Path::new(FIXTURE), &root);
    Project { _dir: dir, root }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read fixture") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Every file under the root that is not an engine sidecar, counted
/// independently of the scanner so `assets_scanned` has an outside referent.
fn non_sidecar_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("dir entry"))
        .map(|e| {
            if e.file_type().expect("file type").is_dir() {
                non_sidecar_files(&e.path())
            } else {
                usize::from(!is_sidecar(&e.path()))
            }
        })
        .sum()
}

fn is_sidecar(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("meta"))
}

fn check(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tidycraft"))
        .arg("check")
        .arg(root)
        .args(["--format", "json", "--no-progress", "--max-issues", "0"])
        .args(extra)
        .output()
        .expect("run tidycraft")
}

fn report(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "report is not JSON ({e}):\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn findings(report: &Value) -> BTreeSet<(String, String)> {
    report["issues"]
        .as_array()
        .expect("issues array")
        .iter()
        .map(|i| {
            (
                i["rule"].as_str().expect("rule").to_string(),
                i["path"].as_str().expect("path").replace('\\', "/"),
            )
        })
        .collect()
}

fn expected() -> BTreeSet<(String, String)> {
    EXPECTED
        .iter()
        .map(|(r, p)| (r.to_string(), p.to_string()))
        .collect()
}

#[test]
fn check_reports_exactly_the_documented_findings() {
    let p = project();
    let out = check(&p.root, &[]);
    let rep = report(&out);

    let got = findings(&rep);
    let want = expected();
    let missing: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "findings differ from docs/analyzer-rules.md\n  missing: {missing:?}\n  unexpected: {extra:?}"
    );

    // Nothing above warning is expected, so the default `fail_on = error` passes.
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(rep["summary"]["errors"], 0);
    assert_eq!(rep["summary"]["issues_total"], want.len());
    assert_eq!(rep["summary"]["truncated"], false);
    assert_eq!(rep["summary"]["scan_warnings"], 0);
    assert_eq!(rep["project"]["engine"], "unity");
}

#[test]
fn sidecars_never_enter_the_asset_list() {
    let p = project();
    let rep = report(&check(&p.root, &[]));

    for (_, path) in findings(&rep) {
        assert!(
            !is_sidecar(Path::new(&path)),
            "a .meta was analysed as an asset: {path}"
        );
    }
    assert_eq!(
        rep["summary"]["assets_scanned"]
            .as_u64()
            .expect("assets_scanned") as usize,
        non_sidecar_files(&p.root),
        "assets_scanned must equal the non-sidecar files on disk"
    );
}

#[test]
fn a_duplicate_group_lists_every_member_once() {
    let p = project();
    let rep = report(&check(&p.root, &[]));
    let dup = rep["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .find(|i| i["rule"] == "duplicate")
        .expect("one duplicate issue");
    let members: BTreeSet<String> = dup["related_paths"]
        .as_array()
        .expect("related_paths")
        .iter()
        .map(|v| v.as_str().expect("path").replace('\\', "/"))
        .collect();
    let want: BTreeSet<String> = ["Assets/Textures/T_Dup_A.png", "Assets/Textures/T_Dup_B.png"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(members, want);
}

#[test]
fn fail_on_warning_turns_the_same_report_red() {
    let p = project();
    let out = check(&p.root, &["--fail-on", "warning"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(findings(&report(&out)), expected());
}

#[test]
fn repeated_runs_agree_on_everything_but_the_clock() {
    let p = project();
    let mut reports: Vec<Value> = (0..RUNS).map(|_| report(&check(&p.root, &[]))).collect();
    for r in &mut reports {
        r["summary"]
            .as_object_mut()
            .expect("summary object")
            .remove("duration_ms");
    }
    for (i, r) in reports.iter().enumerate().skip(1) {
        assert_eq!(&reports[0], r, "run {} differs from run 0", i + 1);
    }
}

#[test]
fn a_stale_dcc_source_is_reported_once_its_mtime_says_so() {
    let p = project();
    let (rule, rel) = STALE_SOURCE;
    let export = p.root.join("Assets/Textures/T_Rock_BaseColor.png");
    let source = p.root.join(rel);
    let exported_at = fs::metadata(&export)
        .expect("export")
        .modified()
        .expect("mtime");

    // Same clock as the export: inside the 60 s tolerance, no finding.
    set_mtime(&source, exported_at);
    assert!(!findings(&report(&check(&p.root, &[]))).contains(&(rule.into(), rel.into())));

    // Two minutes newer than its export: the rule must fire, and nothing else may change.
    set_mtime(&source, exported_at + Duration::from_secs(120));
    let got = findings(&report(&check(&p.root, &[])));
    let mut want = expected();
    want.insert((rule.to_string(), rel.to_string()));
    assert_eq!(got, want);
}

fn set_mtime(path: &Path, to: SystemTime) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(to)
        .expect("set mtime");
}

#[test]
fn sarif_carries_every_finding() {
    let p = project();
    let out = Command::new(env!("CARGO_BIN_EXE_tidycraft"))
        .arg("check")
        .arg(&p.root)
        .args(["--format", "sarif", "--no-progress"])
        .output()
        .expect("run tidycraft");
    let sarif: Value = serde_json::from_slice(&out.stdout).expect("sarif is JSON");
    let results = sarif["runs"][0]["results"].as_array().expect("results");
    assert_eq!(results.len(), EXPECTED.len());
    let rules: BTreeSet<&str> = results
        .iter()
        .map(|r| r["ruleId"].as_str().expect("ruleId"))
        .collect();
    let want: BTreeSet<&str> = EXPECTED.iter().map(|(r, _)| *r).collect();
    assert_eq!(rules, want);
}
