//! `tidycraft check` — scan, run every analyzer phase, subtract the baseline,
//! and exit 1 when what remains reaches the `--fail-on` threshold.

use crate::baseline;
use crate::util;
use crate::CliError;
use serde_json::json;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;
use tidycraft_core::analyzer::pipeline;
use tidycraft_core::analyzer::{Issue, Severity};
use tidycraft_core::scanner::{self, ScanResult};
use tidycraft_core::unity;
use tidycraft_core::warning::ScanWarning;

#[derive(clap::Args)]
pub struct CheckArgs {
    /// Project root (default: current directory)
    pub root: Option<PathBuf>,
    /// Config file to use instead of <root>/tidycraft.toml
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Lowest severity that fails the run (default: `[check] fail_on` from the
    /// config, else `error`)
    #[arg(long, value_enum)]
    pub fail_on: Option<FailOn>,
    /// Output format
    #[arg(long, value_enum, default_value_t = CheckFormat::Human)]
    pub format: CheckFormat,
    /// Baseline file (default: <root>/tidycraft.baseline.json when present)
    #[arg(long, value_name = "FILE")]
    pub baseline: Option<PathBuf>,
    /// Rewrite the baseline to accept every current finding, then exit 0
    #[arg(long)]
    pub update_baseline: bool,
    /// Fail (exit 1) when the scan is incomplete: scan warnings or unpulled
    /// git-lfs pointers
    #[arg(long)]
    pub strict: bool,
    /// Maximum issues to list in human/json output (0 = unlimited; sarif and
    /// github always carry everything)
    #[arg(long, default_value_t = 200, value_name = "N")]
    pub max_issues: usize,
    /// Print only the summary, no issue listing
    #[arg(long)]
    pub summary_only: bool,
    /// How to group the listing
    #[arg(long, value_enum, default_value_t = GroupBy::Rule)]
    pub group_by: GroupBy,
    /// Suppress the status line on stderr
    #[arg(long)]
    pub no_progress: bool,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum FailOn {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum GroupBy {
    Rule,
    Dir,
    Severity,
}

#[derive(Clone, Copy, PartialEq, clap::ValueEnum)]
pub enum CheckFormat {
    Human,
    Json,
    /// SARIF 2.1.0, ready for a code-scanning upload
    Sarif,
    /// GitHub Actions annotations plus a plain summary
    Github,
}

pub fn run(args: CheckArgs) -> Result<ExitCode, CliError> {
    let started = Instant::now();
    let (root_path, root_str) = util::resolve_root(args.root)?;
    let (config, doc, config_source) = util::load_config(&root_str, args.config.as_deref())?;
    let fail_on = resolve_fail_on(args.fail_on, doc.as_ref())?;
    let ignore_set = pipeline::build_ignore_set(&config).map_err(CliError::Config)?;

    if !args.no_progress && args.format == CheckFormat::Human && std::io::stderr().is_terminal() {
        eprintln!("scanning {root_str} ...");
    }
    let scan = util::scan_project(&root_str)?;
    let package_index = unity::build_package_guid_index(&root_path);
    let result = pipeline::run_full_analysis(
        &scan,
        &root_str,
        &config,
        ignore_set.as_ref(),
        &package_index,
    );

    let rows: Vec<Row> = result
        .issues
        .iter()
        .map(|issue| {
            let rel = util::rel_path(&root_str, &issue.asset_path);
            let (key, members) = baseline::issue_key(issue, &rel);
            Row {
                rel,
                key,
                members,
                issue,
            }
        })
        .collect();

    let baseline_path = args
        .baseline
        .clone()
        .unwrap_or_else(|| root_path.join("tidycraft.baseline.json"));
    if args.update_baseline {
        let entries = rows
            .iter()
            .map(|r| baseline::Entry {
                rule: r.issue.rule_id.clone(),
                key: r.key.clone(),
                members: r.members,
            })
            .collect();
        baseline::write(&baseline_path, entries)?;
        println!(
            "baseline written: {} issue(s) -> {}",
            rows.len(),
            scanner::path_to_string(&baseline_path)
        );
        return Ok(ExitCode::SUCCESS);
    }

    let loaded = baseline::load(&baseline_path)?;
    let (mut rows, suppressed) = match &loaded {
        Some(b) => {
            let idx = baseline::index(b);
            let total = rows.len();
            let kept: Vec<Row> = rows
                .into_iter()
                .filter(|r| !baseline::covers(&idx, &r.issue.rule_id, &r.key, r.members))
                .collect();
            let suppressed = total - kept.len();
            (kept, suppressed)
        }
        None => (rows, 0),
    };
    sort_rows(&mut rows, args.group_by);
    let counts = Counts::of(&rows);
    let lfs = detect_lfs_pointers(&scan, &root_str);
    let duration_ms = started.elapsed().as_millis() as u64;

    let listed = if args.summary_only {
        0
    } else if args.max_issues == 0 {
        rows.len()
    } else {
        rows.len().min(args.max_issues)
    };
    let truncated = listed < rows.len();

    let failed_on_issues = match fail_on {
        FailOn::Error => counts.errors > 0,
        FailOn::Warning => counts.errors + counts.warnings > 0,
        FailOn::Info => !rows.is_empty(),
    };
    let incomplete = !scan.warnings.is_empty() || lfs.count > 0;
    let failed = failed_on_issues || (args.strict && incomplete);

    let ctx = EmitCtx {
        root_str: &root_str,
        scan: &scan,
        rows: &rows,
        listed,
        truncated,
        config_source: config_source.as_deref(),
        duration_ms,
        counts,
        suppressed,
        lfs: &lfs,
        group_by: args.group_by,
        fail_on,
        failed,
        failed_on_issues,
    };
    match args.format {
        CheckFormat::Json => emit_json(&ctx)?,
        CheckFormat::Human => emit_human(&ctx),
        CheckFormat::Sarif => emit_sarif(&ctx)?,
        CheckFormat::Github => emit_github(&ctx),
    }

    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// Threshold precedence: `--fail-on` flag, then `[check] fail_on` in the
/// config, then `error`.
fn resolve_fail_on(flag: Option<FailOn>, doc: Option<&toml::Value>) -> Result<FailOn, CliError> {
    if let Some(f) = flag {
        return Ok(f);
    }
    let configured = doc
        .and_then(|d| d.get("check"))
        .and_then(|c| c.get("fail_on"));
    match configured {
        None => Ok(FailOn::Error),
        Some(v) => match v.as_str() {
            Some("error") => Ok(FailOn::Error),
            Some("warning") => Ok(FailOn::Warning),
            Some("info") => Ok(FailOn::Info),
            _ => Err(CliError::Config(format!(
                "[check] fail_on must be \"error\", \"warning\" or \"info\", got: {v}"
            ))),
        },
    }
}

struct Row<'a> {
    rel: String,
    key: String,
    members: Option<usize>,
    issue: &'a Issue,
}

#[derive(Clone, Copy)]
struct Counts {
    errors: usize,
    warnings: usize,
    infos: usize,
}

impl Counts {
    fn of(rows: &[Row]) -> Self {
        let mut c = Counts {
            errors: 0,
            warnings: 0,
            infos: 0,
        };
        for r in rows {
            match r.issue.severity {
                Severity::Error => c.errors += 1,
                Severity::Warning => c.warnings += 1,
                Severity::Info => c.infos += 1,
            }
        }
        c
    }
}

struct LfsReport {
    count: usize,
    sample: Vec<String>,
}

/// A git-lfs pointer left unpulled is a tiny text file sitting where the real
/// asset should be — every content-level rule then silently checks the wrong
/// bytes. Only files small enough to be pointers are opened.
fn detect_lfs_pointers(scan: &ScanResult, root: &str) -> LfsReport {
    const HEADER: &[u8] = b"version https://git-lfs.github.com/spec/v1";
    let mut matches: Vec<String> = Vec::new();
    for a in &scan.assets {
        if a.size == 0 || a.size > 512 {
            continue;
        }
        let Ok(mut f) = std::fs::File::open(&a.path) else {
            continue;
        };
        let mut buf = vec![0u8; HEADER.len()];
        use std::io::Read;
        if f.read_exact(&mut buf).is_ok() && buf == HEADER {
            matches.push(util::rel_path(root, &a.path));
        }
    }
    matches.sort();
    let count = matches.len();
    matches.truncate(5);
    LfsReport {
        count,
        sample: matches,
    }
}

fn severity_rank(s: &Severity) -> u8 {
    match s {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    }
}

fn severity_name(s: &Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn severity_color(s: &Severity) -> &'static str {
    match s {
        Severity::Error => "31",
        Severity::Warning => "33",
        Severity::Info => "36",
    }
}

fn dir_of(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(dir, _)| dir).unwrap_or(".")
}

fn group_key<'a>(row: &'a Row, group_by: GroupBy) -> &'a str {
    match group_by {
        GroupBy::Rule => &row.issue.rule_id,
        GroupBy::Dir => dir_of(&row.rel),
        GroupBy::Severity => severity_name(&row.issue.severity),
    }
}

fn sort_rows(rows: &mut [Row], group_by: GroupBy) {
    match group_by {
        GroupBy::Rule => rows.sort_by(|a, b| {
            (&a.issue.rule_id, severity_rank(&a.issue.severity), &a.rel).cmp(&(
                &b.issue.rule_id,
                severity_rank(&b.issue.severity),
                &b.rel,
            ))
        }),
        GroupBy::Dir => rows.sort_by(|a, b| {
            (dir_of(&a.rel), &a.issue.rule_id, &a.rel).cmp(&(
                dir_of(&b.rel),
                &b.issue.rule_id,
                &b.rel,
            ))
        }),
        GroupBy::Severity => rows.sort_by(|a, b| {
            (severity_rank(&a.issue.severity), &a.issue.rule_id, &a.rel).cmp(&(
                severity_rank(&b.issue.severity),
                &b.issue.rule_id,
                &b.rel,
            ))
        }),
    }
}

fn map_is_empty(m: &&HashMap<String, String>) -> bool {
    m.is_empty()
}

#[derive(serde::Serialize)]
struct IssueOut<'a> {
    rule: &'a str,
    severity: &'a Severity,
    path: &'a str,
    message: &'a str,
    fingerprint: &'a str,
    auto_fixable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<&'a String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    related_paths: Option<&'a Vec<String>>,
    #[serde(skip_serializing_if = "map_is_empty")]
    args: &'a HashMap<String, String>,
}

#[derive(serde::Serialize)]
struct Summary {
    errors: usize,
    warnings: usize,
    infos: usize,
    issues_total: usize,
    baseline_suppressed: usize,
    assets_scanned: usize,
    scan_warnings: usize,
    lfs_pointers: usize,
    duration_ms: u64,
    truncated: bool,
}

#[derive(serde::Serialize)]
struct Report<'a> {
    schema_version: u32,
    tool: util::ToolInfo,
    project: util::ProjectBlock,
    summary: Summary,
    scan_warnings: &'a [ScanWarning],
    issues: Vec<IssueOut<'a>>,
}

struct EmitCtx<'a> {
    root_str: &'a str,
    scan: &'a ScanResult,
    rows: &'a [Row<'a>],
    listed: usize,
    truncated: bool,
    config_source: Option<&'a str>,
    duration_ms: u64,
    counts: Counts,
    suppressed: usize,
    lfs: &'a LfsReport,
    group_by: GroupBy,
    fail_on: FailOn,
    failed: bool,
    failed_on_issues: bool,
}

fn emit_json(ctx: &EmitCtx) -> Result<(), CliError> {
    let report = Report {
        schema_version: 1,
        tool: util::tool_info(),
        project: util::ProjectBlock {
            root: ctx.root_str.to_string(),
            engine: util::engine_name(ctx.scan.project_type.as_ref()),
            config_source: ctx.config_source.map(str::to_string),
        },
        summary: Summary {
            errors: ctx.counts.errors,
            warnings: ctx.counts.warnings,
            infos: ctx.counts.infos,
            issues_total: ctx.rows.len(),
            baseline_suppressed: ctx.suppressed,
            assets_scanned: ctx.scan.total_count,
            scan_warnings: ctx.scan.warnings.len(),
            lfs_pointers: ctx.lfs.count,
            duration_ms: ctx.duration_ms,
            truncated: ctx.truncated,
        },
        scan_warnings: &ctx.scan.warnings,
        issues: ctx.rows[..ctx.listed]
            .iter()
            .map(|row| IssueOut {
                rule: &row.issue.rule_id,
                severity: &row.issue.severity,
                path: &row.rel,
                message: &row.issue.message,
                fingerprint: &row.key,
                auto_fixable: row.issue.auto_fixable,
                suggestion: row.issue.suggestion.as_ref(),
                related_paths: row.issue.related_paths.as_ref(),
                args: &row.issue.args,
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| CliError::Runtime(format!("serialize report: {e}")))?;
    println!("{json}");
    Ok(())
}

fn emit_human(ctx: &EmitCtx) {
    let colored = util::use_color();
    println!(
        "{} — {} project · {} assets scanned in {:.1}s · config: {}",
        ctx.root_str,
        util::engine_name(ctx.scan.project_type.as_ref()).unwrap_or("generic"),
        ctx.scan.total_count,
        ctx.duration_ms as f64 / 1000.0,
        ctx.config_source.unwrap_or("defaults"),
    );

    let mut current_key: Option<String> = None;
    for row in &ctx.rows[..ctx.listed] {
        let key = group_key(row, ctx.group_by);
        if current_key.as_deref() != Some(key) {
            println!("\n{key}");
            current_key = Some(key.to_string());
        }
        let sev = util::paint(
            severity_name(&row.issue.severity),
            severity_color(&row.issue.severity),
            colored,
        );
        // Pad on the uncolored name: escape codes would break the width.
        let pad = " ".repeat(7 - severity_name(&row.issue.severity).len());
        println!("  {sev}{pad}  {} — {}", row.rel, row.issue.message);
        if let Some(s) = &row.issue.suggestion {
            println!("           fix: {s}");
        }
    }
    if ctx.truncated {
        println!(
            "\n(showing {} of {} issues — rerun with --max-issues 0 for all)",
            ctx.listed,
            ctx.rows.len()
        );
    }

    print_completeness(ctx);
    print_summary_line(ctx, colored);
}

/// Scan warnings and unpulled LFS pointers — the "results may be incomplete"
/// block, shared by the human and github outputs.
fn print_completeness(ctx: &EmitCtx) {
    if !ctx.scan.warnings.is_empty() {
        println!("\nscan warnings (results may be incomplete):");
        for w in &ctx.scan.warnings {
            println!("  {}", util::describe_scan_warning(w));
        }
    }
    if ctx.lfs.count > 0 {
        println!(
            "\n{} git-lfs pointer file(s) not pulled — content rules (texture, duplicate, audio, model) saw pointer bytes, not assets — e.g. {}",
            ctx.lfs.count,
            ctx.lfs.sample.join(", ")
        );
    }
}

fn print_summary_line(ctx: &EmitCtx, colored: bool) {
    let verdict = if !ctx.failed {
        util::paint("ok", "32", colored)
    } else if ctx.failed_on_issues {
        util::paint("FAIL", "31", colored)
    } else {
        util::paint("FAIL (strict: scan incomplete)", "31", colored)
    };
    let fail_on_name = match ctx.fail_on {
        FailOn::Error => "error",
        FailOn::Warning => "warning",
        FailOn::Info => "info",
    };
    let baselined = if ctx.suppressed > 0 {
        format!(" · {} baselined", ctx.suppressed)
    } else {
        String::new()
    };
    println!(
        "\n{} error{}, {} warning{}, {} info{} · fail-on {} → {}",
        ctx.counts.errors,
        if ctx.counts.errors == 1 { "" } else { "s" },
        ctx.counts.warnings,
        if ctx.counts.warnings == 1 { "" } else { "s" },
        ctx.counts.infos,
        baselined,
        fail_on_name,
        verdict
    );
}

/// SARIF 2.1.0, one result per post-baseline finding. Paths stay
/// project-root-relative, which matches the repo layout whenever the project
/// root is the repo root; `partialFingerprints` carries the baseline key so an
/// alert keeps its identity across renames of duplicate-group members.
fn emit_sarif(ctx: &EmitCtx) -> Result<(), CliError> {
    let help_uri = concat!(
        env!("CARGO_PKG_HOMEPAGE"),
        "/blob/main/docs/analyzer-rules.md"
    );
    let rules: Vec<serde_json::Value> = tidycraft_core::analyzer::RULE_ARGS
        .iter()
        .map(|(id, _)| json!({ "id": id, "helpUri": help_uri }))
        .collect();
    let results: Vec<serde_json::Value> = ctx
        .rows
        .iter()
        .map(|row| {
            let level = match row.issue.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "note",
            };
            json!({
                "ruleId": row.issue.rule_id,
                "level": level,
                "message": { "text": row.issue.message },
                "locations": [{ "physicalLocation": {
                    "artifactLocation": { "uri": row.rel },
                    "region": { "startLine": 1 }
                }}],
                "partialFingerprints": { "tidycraftKey/v1": row.key },
            })
        })
        .collect();
    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "tidycraft",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": env!("CARGO_PKG_HOMEPAGE"),
                "rules": rules,
            }},
            "results": results,
        }]
    });
    let out = serde_json::to_string_pretty(&doc)
        .map_err(|e| CliError::Runtime(format!("serialize sarif: {e}")))?;
    println!("{out}");
    Ok(())
}

/// GitHub Actions annotations (`::error` / `::warning` / `::notice`). The
/// runner caps annotations at 10 per type per step, so each type shows at most
/// 9 findings plus one rollup; the plain tail still lands in the job log.
fn emit_github(ctx: &EmitCtx) {
    const PER_TYPE: usize = 9;
    let mut shown = [0usize; 3];
    let mut overflow = [0usize; 3];
    for row in ctx.rows {
        let slot = severity_rank(&row.issue.severity) as usize;
        if shown[slot] < PER_TYPE {
            shown[slot] += 1;
            println!(
                "::{} file={},title={}::{}",
                annotation_command(&row.issue.severity),
                esc_property(&row.rel),
                esc_property(&row.issue.rule_id),
                esc_data(&row.issue.message)
            );
        } else {
            overflow[slot] += 1;
        }
    }
    for (slot, n) in overflow.iter().enumerate() {
        if *n > 0 {
            let sev = ["error", "warning", "info"][slot];
            let cmd = ["error", "warning", "notice"][slot];
            println!(
                "::{cmd}::{n} more {sev} finding(s) — run `tidycraft check` locally for the full list"
            );
        }
    }
    print_completeness(ctx);
    print_summary_line(ctx, false);
}

fn annotation_command(s: &Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "notice",
    }
}

/// Escape a workflow-command message. `%` must go first or the other escapes
/// get double-escaped.
fn esc_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escape a workflow-command property (`file=`, `title=`): the data escapes
/// plus the property delimiters.
fn esc_property(s: &str) -> String {
    esc_data(s).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> toml::Value {
        s.parse().unwrap()
    }

    #[test]
    fn fail_on_precedence_is_flag_then_config_then_error() {
        // No flag, no config → error.
        assert!(matches!(resolve_fail_on(None, None), Ok(FailOn::Error)));
        // Config alone wins over the default.
        let d = doc("[check]\nfail_on = \"warning\"");
        assert!(matches!(
            resolve_fail_on(None, Some(&d)),
            Ok(FailOn::Warning)
        ));
        // The flag wins over the config.
        assert!(matches!(
            resolve_fail_on(Some(FailOn::Info), Some(&d)),
            Ok(FailOn::Info)
        ));
        // An invalid config value is a usage error, not a silent default.
        let bad = doc("[check]\nfail_on = \"fatal\"");
        assert!(matches!(
            resolve_fail_on(None, Some(&bad)),
            Err(CliError::Config(_))
        ));
    }

    #[test]
    fn workflow_command_escaping_covers_delimiters() {
        assert_eq!(esc_data("50% done\nnext"), "50%25 done%0Anext");
        assert_eq!(esc_property("a,b:c%d"), "a%2Cb%3Ac%25d");
    }
}
