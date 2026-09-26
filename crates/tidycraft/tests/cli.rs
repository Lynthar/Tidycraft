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

fn tidycraft(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tidycraft"))
        .args(args)
        .output()
        .expect("run tidycraft")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Exit codes are a contract: 2 for usage or configuration, 3 for the environment.
#[test]
fn usage_and_environment_failures_exit_2_and_3() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let missing = p.root.join("does-not-exist");
    let missing = missing.to_str().unwrap();

    let out = tidycraft(&["check", missing]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("project root is not a directory"),
        "{}",
        text(&out.stderr)
    );

    let out = tidycraft(&["check", root, "--config", missing]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("cannot read config"),
        "{}",
        text(&out.stderr)
    );

    let toml = p.root.join("tidycraft.toml");
    fs::write(&toml, "[texture\n").unwrap();
    assert_eq!(code(&check(&p.root, &[])), 2);

    // Absent: built-in defaults, and the report carries no config_source.
    fs::remove_file(&toml).unwrap();
    let out = check(&p.root, &[]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(report(&out)["project"].get("config_source").is_none());

    // Present but unreadable: the environment failed, not the user.
    fs::create_dir(&toml).unwrap();
    let out = check(&p.root, &[]);
    assert_eq!(code(&out), 3, "{}", text(&out.stderr));
}

/// Threshold precedence: `--fail-on` flag, then `[check] fail_on`, then `error`.
#[test]
fn fail_on_comes_from_the_flag_then_the_config_then_defaults_to_error() {
    let p = project();
    let toml = p.root.join("tidycraft.toml");
    let base = fs::read_to_string(&toml).unwrap();
    let configured = |fail_on: &str| {
        fs::write(&toml, format!("{base}\n[check]\nfail_on = \"{fail_on}\"\n")).unwrap()
    };

    // The fixture yields warnings and infos, no errors.
    assert_eq!(code(&check(&p.root, &[])), 0);
    configured("info");
    assert_eq!(code(&check(&p.root, &[])), 1);
    configured("warning");
    assert_eq!(code(&check(&p.root, &[])), 1);
    assert_eq!(
        code(&check(&p.root, &["--fail-on", "error"])),
        0,
        "the flag wins"
    );
    configured("error");
    assert_eq!(code(&check(&p.root, &[])), 0);
    configured("loud");
    assert_eq!(code(&check(&p.root, &[])), 2);
}

#[test]
fn the_listing_is_bounded_while_the_summary_counts_everything() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let json = ["check", root, "--format", "json", "--no-progress"];

    let rep = report(&tidycraft(&[&json[..], &["--max-issues", "5"]].concat()));
    assert_eq!(rep["issues"].as_array().unwrap().len(), 5);
    assert_eq!(rep["summary"]["truncated"], true);
    assert_eq!(rep["summary"]["issues_total"], EXPECTED.len());

    let rep = report(&tidycraft(&[&json[..], &["--summary-only"]].concat()));
    assert_eq!(rep["issues"].as_array().unwrap().len(), 0);
    assert_eq!(rep["summary"]["issues_total"], EXPECTED.len());

    let out = tidycraft(&["check", root, "--max-issues", "5", "--no-progress"]);
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains(&format!("(showing 5 of {} issues", EXPECTED.len())),
        "{stdout}"
    );

    // The human listing: one header per group, severity column padded to one
    // width, and no completeness block when the scan was complete.
    let full = text(&tidycraft(&["check", root, "--no-progress"]).stdout);
    assert_eq!(
        full.lines().filter(|l| *l == "duplicate").count(),
        1,
        "{full}"
    );
    assert!(
        full.contains(
            "
  warning  Assets/"
        ),
        "{full}"
    );
    assert!(
        full.contains(
            "
  info     Assets/"
        ),
        "{full}"
    );
    assert!(
        !full.contains("scan warnings") && !full.contains("git-lfs"),
        "{full}"
    );
    let by_dir = text(&tidycraft(&["check", root, "--no-progress", "--group-by", "dir"]).stdout);
    assert_eq!(
        by_dir.lines().filter(|l| *l == "Assets/Textures").count(),
        1,
        "{by_dir}"
    );
    let by_sev =
        text(&tidycraft(&["check", root, "--no-progress", "--group-by", "severity"]).stdout);
    assert_eq!(
        by_sev.lines().filter(|l| *l == "warning").count(),
        1,
        "{by_sev}"
    );
    assert_eq!(
        by_sev.lines().filter(|l| *l == "info").count(),
        1,
        "{by_sev}"
    );
}

/// The summary's counts are the listing's counts, severities are the three
/// documented names, and paths are project-relative with forward slashes.
#[test]
fn summary_counts_match_the_listing_and_paths_are_relative() {
    let p = project();
    let rep = report(&check(&p.root, &[]));
    let issues = rep["issues"].as_array().unwrap();
    let count = |sev: &str| issues.iter().filter(|i| i["severity"] == sev).count();
    for i in issues {
        let sev = i["severity"].as_str().unwrap();
        assert!(["error", "warning", "info"].contains(&sev), "{sev}");
        let path = i["path"].as_str().unwrap();
        assert!(
            !path.contains('\\') && path.starts_with("Assets/"),
            "{path}"
        );
    }
    assert_eq!(rep["summary"]["errors"], count("error"));
    assert_eq!(rep["summary"]["warnings"], count("warning"));
    assert_eq!(rep["summary"]["infos"], count("info"));
    assert!(count("info") > 0 && count("warning") > 0);
    // A rule that interpolates nothing carries no `args` key at all.
    let chinese = issues
        .iter()
        .find(|i| i["rule"] == "naming.chinese")
        .expect("in the fixture");
    assert!(chinese.get("args").is_none());
    let pot = issues
        .iter()
        .find(|i| i["rule"] == "texture.pot")
        .expect("in the fixture");
    assert_eq!(pot["args"]["width"], "6");
}

/// An unpulled git-lfs pointer (spec header, at most 512 bytes) is reported in
/// every format and turns the run red only under `--strict`.
#[test]
fn an_unpulled_lfs_pointer_is_reported_and_strict_makes_it_red() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let header = "version https://git-lfs.github.com/spec/v1\noid sha256:0000000000000000000000000000000000000000000000000000000000000000\nsize 12345\n";
    // A complete scan is complete: nothing to admit, --strict passes.
    assert_eq!(code(&check(&p.root, &["--strict"])), 0);
    let textures = p.root.join("Assets/Textures");
    fs::write(textures.join("T_Pointer.png"), header).unwrap();
    let mut at_limit = header.as_bytes().to_vec();
    at_limit.resize(512, b'\n');
    fs::write(textures.join("T_PointerAtLimit.png"), &at_limit).unwrap();
    let mut over = at_limit.clone();
    over.push(b'\n');
    fs::write(textures.join("T_PointerOver.png"), &over).unwrap();

    let out = check(&p.root, &[]);
    assert_eq!(code(&out), 0, "reported, not red, by default");
    assert_eq!(report(&out)["summary"]["lfs_pointers"], 2);
    assert_eq!(code(&check(&p.root, &["--strict"])), 1);

    let human = tidycraft(&["check", root, "--strict", "--no-progress"]);
    assert_eq!(code(&human), 1);
    let stdout = text(&human.stdout);
    assert!(
        stdout.contains("2 git-lfs pointer file(s) not pulled"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Assets/Textures/T_Pointer.png, Assets/Textures/T_PointerAtLimit.png"),
        "{stdout}"
    );
    assert!(!stdout.contains("T_PointerOver"), "{stdout}");
    assert!(
        stdout.contains("FAIL (strict: scan incomplete)"),
        "{stdout}"
    );
}

/// GitHub caps annotations at 10 per type per step: at most 9 findings per
/// severity are annotated and the rest roll up into one line, none lost.
#[test]
fn github_annotations_cap_each_severity_at_nine_and_roll_up_the_rest() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let rep = report(&check(&p.root, &[]));
    let out = tidycraft(&["check", root, "--format", "github", "--no-progress"]);
    let stdout = text(&out.stdout);
    for (severity, command, total) in [
        ("warning", "warning", "warnings"),
        ("info", "notice", "infos"),
        ("error", "error", "errors"),
    ] {
        let detailed = stdout
            .lines()
            .filter(|l| l.starts_with(&format!("::{command} file=")))
            .count();
        assert!(detailed <= 9, "{severity}: {detailed}");
        let rolled: usize = stdout
            .lines()
            .find_map(|l| l.strip_prefix(&format!("::{command}::")))
            .and_then(|rest| rest.split_whitespace().next())
            .map(|n| n.parse().expect("a count"))
            .unwrap_or(0);
        let total = rep["summary"][total].as_u64().expect("count") as usize;
        assert_eq!(detailed + rolled, total, "{severity}");
        assert_eq!(detailed, total.min(9), "{severity}");
        if total <= 9 {
            assert!(
                !stdout.contains(&format!("::{command}::")),
                "{severity}: no rollup under the cap"
            );
        }
    }
    assert!(
        rep["summary"]["warnings"].as_u64().unwrap() > 9,
        "the fixture must overflow the cap"
    );
}

/// The baseline accepts today's findings; a grown duplicate group re-fires; a
/// broken baseline is a usage error rather than a silent reset.
#[test]
fn a_baseline_accepts_current_findings_and_a_broken_one_is_a_usage_error() {
    let p = project();
    let baseline = p.root.join("tidycraft.baseline.json");
    let out = check(&p.root, &["--update-baseline"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let written = format!("baseline written: {} issue(s)", EXPECTED.len());
    assert!(
        text(&out.stdout).starts_with(&written),
        "{}",
        text(&out.stdout)
    );
    assert!(baseline.exists());

    let out = check(&p.root, &["--fail-on", "info"]);
    assert_eq!(code(&out), 0);
    let rep = report(&out);
    assert_eq!(rep["summary"]["baseline_suppressed"], EXPECTED.len());
    assert_eq!(rep["summary"]["issues_total"], 0);
    assert_eq!(code(&check(&p.root, &["--fail-on", "warning"])), 0);

    // A third copy grows the accepted duplicate group, so it fires again.
    fs::copy(
        p.root.join("Assets/Textures/T_Dup_A.png"),
        p.root.join("Assets/Textures/T_Dup_C.png"),
    )
    .unwrap();
    let out = check(&p.root, &["--fail-on", "warning"]);
    assert_eq!(code(&out), 1);
    let rep = report(&out);
    let got = findings(&rep);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got.iter().all(|(rule, _)| rule == "duplicate"), "{got:?}");
    assert_eq!(rep["summary"]["baseline_suppressed"], EXPECTED.len() - 1);

    // A baseline path that does not exist is simply no baseline.
    let nope = p.root.join("nope.json");
    let out = check(
        &p.root,
        &["--baseline", nope.to_str().unwrap(), "--fail-on", "warning"],
    );
    assert_eq!(code(&out), 1);
    assert_eq!(report(&out)["summary"]["baseline_suppressed"], 0);

    fs::write(&baseline, "not json").unwrap();
    let out = check(&p.root, &[]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("invalid baseline"),
        "{}",
        text(&out.stderr)
    );

    // Present but unreadable is a usage error too, never "no baseline".
    let unreadable = p.root.join("Assets");
    let out = check(&p.root, &["--baseline", unreadable.to_str().unwrap()]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("cannot read baseline"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn rules_lists_every_documented_rule_id_and_the_effective_config() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let out = tidycraft(&["rules", root]);
    assert_eq!(code(&out), 0);
    let stdout = text(&out.stdout);
    for id in EXPECTED.iter().map(|(r, _)| *r).chain([STALE_SOURCE.0]) {
        assert!(
            stdout.lines().any(|l| l.trim() == id),
            "{id} missing from:\n{stdout}"
        );
    }

    let out = tidycraft(&["rules", root, "--format", "json"]);
    let rep: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(rep["config_source"], "tidycraft.toml");
    assert_eq!(rep["config"]["texture"]["enabled"], true);
    assert_eq!(rep["config"]["texture"]["max_size"], 16);
    let ids: BTreeSet<&str> = rep["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains("dcc_source.outdated_export") && ids.contains("naming.prefix"));
}

#[test]
fn explain_prints_the_rule_doc_or_fails_as_a_usage_error() {
    let out = tidycraft(&["explain", "naming.prefix"]);
    assert_eq!(code(&out), 0);
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("naming.prefix — "), "{stdout}");
    assert!(stdout.contains("docs/analyzer-rules.md"), "{stdout}");

    let out = tidycraft(&["explain", "nonsense"]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("unknown rule `nonsense`"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn scan_lists_every_asset_and_filters_and_bounds_on_request() {
    let p = project();
    let root = p.root.to_str().expect("utf-8 path");
    let scan = |extra: &[&str]| -> Value {
        let mut args = vec!["scan", root];
        args.extend_from_slice(extra);
        let out = tidycraft(&args);
        assert_eq!(code(&out), 0, "{}", text(&out.stderr));
        serde_json::from_slice(&out.stdout).expect("json")
    };
    let matched = |rep: &Value| rep["summary"]["matched"].as_u64().expect("matched");

    let all = scan(&[]);
    assert_eq!(all["summary"]["assets_total"], non_sidecar_files(&p.root));
    let paths: Vec<&str> = all["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.windows(2).all(|w| w[0] < w[1]),
        "sorted by path: {paths:?}"
    );
    assert!(
        paths.iter().all(|p| !p.contains('\\') && !p.contains(':')),
        "{paths:?}"
    );

    let textures = scan(&["--types", "texture"]);
    let returned = textures["assets"].as_array().unwrap();
    assert!(returned.iter().all(|a| a["type"] == "texture"));
    assert_eq!(matched(&textures) as usize, returned.len());
    let audio = scan(&["--types", "audio"]);
    let both = scan(&["--types", "texture,audio"]);
    assert_eq!(matched(&both), matched(&textures) + matched(&audio));
    assert!(matched(&both) < all["summary"]["assets_total"].as_u64().unwrap());

    let bounded = scan(&["--max-assets", "3"]);
    assert_eq!(bounded["assets"].as_array().unwrap().len(), 3);
    assert_eq!(bounded["summary"]["truncated"], true);
    assert_eq!(bounded["summary"]["matched"], non_sidecar_files(&p.root));
    let exact = scan(&["--max-assets", &all["summary"]["assets_total"].to_string()]);
    assert_eq!(exact["summary"]["truncated"], false);

    let out = tidycraft(&["scan", root, "--types", "bogus"]);
    assert_eq!(code(&out), 2);
    assert!(
        text(&out.stderr).contains("unknown asset type `bogus`"),
        "{}",
        text(&out.stderr)
    );
}
