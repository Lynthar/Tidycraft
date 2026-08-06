//! Rule-driven tag suggestions — runs `LearnedRule`s produced by AI
//! Learning over the current scan and groups matched assets by tag
//! label.
//!
//! Uses the same `TagSuggester` interface as `HeuristicSuggester` so the
//! `suggest_tags` command can swap between them transparently. Output
//! is `Vec<TagGroup>` with the same shape — frontend doesn't need to
//! know whether suggestions came from heuristic clustering or LLM-
//! derived rules.
//!
//! Rules apply independently per asset; the suggester aggregates by tag
//! label (multiple rules may target the same tag, in which case the
//! group inherits the highest confidence and a hint pointing to the
//! winning rule).

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::Serialize;

use crate::scanner::ScanResult;

use super::tag_suggest::{TagGroup, TagSuggester};
use crate::llm::learning::LearnedRule;

const MAX_GROUPS: usize = 24;
const SAMPLE_FILENAMES: usize = 3;

/// Color palette mirroring `tag_suggest::PALETTE` so heuristic and rule
/// suggestions look at home next to each other in the panel.
const PALETTE: &[&str] = &[
    "#7ab97a", "#c47a7a", "#7ac4c4", "#b67ac4", "#c4a87a",
    "#5fa6cf", "#c9a558", "#8088c4", "#c87aa8", "#6589c7",
];

fn pick_color(name: &str) -> String {
    // FNV-1a, identical to tag_suggest so colors are stable across both.
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    PALETTE[(h as usize) % PALETTE.len()].to_string()
}

fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// A non-fatal problem that stopped AI-learned rules from running as
/// written.
///
/// These reached `eprintln!` and nowhere else, which in a shipped app means
/// nowhere at all — a Finder-launched `.app` has no stderr attached to
/// anything. Nor does the panel give the user another signal to read: rule
/// groups and heuristic groups render identically, so falling back looks
/// exactly like a rule set that happened to produce these groups. They now
/// ride back with the suggestions and the panel states them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleWarning {
    /// `tidycraft.ai.toml` is there but unreadable or malformed: every rule
    /// in it was skipped and the heuristic suggester ran in their place.
    RulesUnreadable { detail: String },
    /// One `filename_regex` rule's pattern didn't compile, so that rule
    /// matched nothing. The review panel pre-validates with JS `RegExp`,
    /// which accepts what this engine rejects (backreferences `\1`,
    /// look-around `(?=`) — those pass review and land here.
    InvalidPattern { pattern: String, detail: String },
}

/// What `suggest_tags` hands the panel: the groups to render, plus whatever
/// the user needs to know about rules that didn't run.
#[derive(Debug, Serialize)]
pub struct TagSuggestions {
    pub groups: Vec<TagGroup>,
    pub warnings: Vec<RuleWarning>,
}

/// Load AI-derived rules from `<root>/tidycraft.ai.toml` and run them.
/// Falls back to `HeuristicSuggester` when there is nothing to run — no
/// rules file (the user hasn't run AI Learning yet) or an empty rule list.
///
/// A corrupt rules file falls back the same way rather than failing the
/// call, so the panel still shows something, but it comes back as a
/// `RulesUnreadable` warning instead of vanishing.
pub fn load_or_fallback(scan: &ScanResult, project_root: &std::path::Path) -> TagSuggestions {
    match crate::llm::rule_store::AiRulesDoc::load(project_root) {
        Ok(Some(d)) if !d.rules.is_empty() => {
            let suggester = RuleSuggester::new(d.rules);
            let groups = suggester.suggest(scan);
            TagSuggestions {
                groups,
                warnings: suggester.warnings,
            }
        }
        Ok(_) => TagSuggestions {
            groups: super::tag_suggest::HeuristicSuggester.suggest(scan),
            warnings: Vec::new(),
        },
        Err(detail) => TagSuggestions {
            groups: super::tag_suggest::HeuristicSuggester.suggest(scan),
            warnings: vec![RuleWarning::RulesUnreadable { detail }],
        },
    }
}

/// A `LearnedRule` paired with its compiled `Regex` (only populated for
/// `FilenameRegex` kind). Pre-compiling at construction time means the
/// per-asset hot loop in `suggest()` doesn't pay parse cost N×M times.
struct CompiledRule {
    rule: LearnedRule,
    regex: Option<Regex>,
}

pub struct RuleSuggester {
    rules: Vec<CompiledRule>,
    /// Patterns that failed to compile, in rule order. Read by
    /// `load_or_fallback` on its way back to the panel.
    warnings: Vec<RuleWarning>,
}

impl RuleSuggester {
    pub fn new(rules: Vec<LearnedRule>) -> Self {
        let mut warnings = Vec::new();
        let compiled = rules
            .into_iter()
            .map(|rule| {
                let (compiled, warning) = compile_one(rule);
                warnings.extend(warning);
                compiled
            })
            .collect();
        Self {
            rules: compiled,
            warnings,
        }
    }
}

/// Compile one rule. For `FilenameRegex`, attempts `Regex::new`; on
/// failure returns a `None` regex so the rule skips at match time, plus
/// the warning that says so. We deliberately do NOT propagate the error —
/// a single malformed pattern shouldn't poison the whole rule set when the
/// rest are usable. The LearnReviewPanel runs a similar validity check on
/// the UI side via JS `RegExp` — close enough for the simple patterns the
/// LLM emits, but the dialects diverge both ways: JS accepts what this
/// engine rejects (backreferences `\1`, look-around `(?=`), and those land
/// here and skip; while this engine accepts what JS rejects
/// (`(?P<name>...)` named groups), so the panel can warn about a pattern
/// that compiles fine here.
fn compile_one(rule: LearnedRule) -> (CompiledRule, Option<RuleWarning>) {
    let (regex, warning) = match &rule {
        LearnedRule::FilenameRegex { pattern, .. } => match Regex::new(pattern) {
            Ok(r) => (Some(r), None),
            Err(e) => (
                None,
                Some(RuleWarning::InvalidPattern {
                    pattern: pattern.clone(),
                    detail: e.to_string(),
                }),
            ),
        },
        _ => (None, None),
    };
    (CompiledRule { rule, regex }, warning)
}

struct GroupAcc {
    paths: HashSet<String>,
    confidence: f32,
    /// Hint string for the rule that produced the highest confidence.
    /// Format: `ai · {kind} "{pattern}"`.
    hint: String,
}

impl TagSuggester for RuleSuggester {
    fn suggest(&self, scan: &ScanResult) -> Vec<TagGroup> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        let root = scan.root_path.trim_end_matches('/');
        let mut by_label: HashMap<String, GroupAcc> = HashMap::new();

        for asset in &scan.assets {
            let rel = relative_path(root, &asset.path);
            for cr in &self.rules {
                if let Some((tags, conf, hint)) = match_rule(cr, &rel, &asset.name) {
                    for tag in tags {
                        let entry = by_label.entry(tag.clone()).or_insert_with(|| GroupAcc {
                            paths: HashSet::new(),
                            confidence: 0.0,
                            hint: hint.clone(),
                        });
                        entry.paths.insert(asset.path.clone());
                        if conf > entry.confidence {
                            entry.confidence = conf;
                            entry.hint = hint.clone();
                        }
                    }
                }
            }
        }

        // Materialize. Sort sample filenames alphabetically for stable
        // diff-friendly output.
        let mut groups: Vec<TagGroup> = by_label
            .into_iter()
            .map(|(name, acc)| {
                let mut paths: Vec<String> = acc.paths.into_iter().collect();
                paths.sort();
                let samples: Vec<String> = paths
                    .iter()
                    .take(SAMPLE_FILENAMES)
                    .map(|p| {
                        p.rsplit('/')
                            .next()
                            .map(|n| stem(n).to_string())
                            .unwrap_or_default()
                    })
                    .collect();
                let color = pick_color(&name);
                TagGroup {
                    name,
                    color,
                    file_paths: paths,
                    confidence: acc.confidence,
                    hint: acc.hint,
                    samples,
                }
            })
            .collect();

        // Sort by confidence desc; ties broken by file_paths.len() desc
        // so a rule that matched 50 assets surfaces above one that
        // matched 3 at the same confidence.
        groups.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.file_paths.len().cmp(&a.file_paths.len()))
                .then_with(|| a.name.cmp(&b.name))
        });
        groups.truncate(MAX_GROUPS);
        groups
    }
}

/// Decide whether a rule fires on the given asset. Returns `(tags,
/// confidence, hint)` on hit, `None` on miss.
///
/// Matching semantics:
/// - filename_token: case-insensitive substring of basename
/// - path_prefix: case-sensitive prefix of relative path
/// - path_segment: case-insensitive equality against any `/`-split
///   segment of the relative path (so "hero" matches "a/hero/b" but
///   not "a/heroic/b")
/// - filename_regex: pre-compiled regex (linear-time `regex` crate, no
///   backtracking) applied to the relative path. Patterns that failed
///   to compile at construction time silent-skip here.
fn match_rule<'r>(
    cr: &'r CompiledRule,
    rel_path: &str,
    filename: &str,
) -> Option<(&'r [String], f32, String)> {
    match &cr.rule {
        LearnedRule::FilenameToken {
            pattern,
            tags,
            confidence,
        } => {
            if filename.to_lowercase().contains(&pattern.to_lowercase()) {
                Some((tags, *confidence, format!("ai · token \"{pattern}\"")))
            } else {
                None
            }
        }
        LearnedRule::PathPrefix {
            pattern,
            tags,
            confidence,
        } => {
            if rel_path.starts_with(pattern.as_str()) {
                Some((tags, *confidence, format!("ai · prefix {pattern}")))
            } else {
                None
            }
        }
        LearnedRule::PathSegment {
            pattern,
            tags,
            confidence,
        } => {
            if rel_path.split('/').any(|s| s.eq_ignore_ascii_case(pattern)) {
                Some((tags, *confidence, format!("ai · segment {pattern}")))
            } else {
                None
            }
        }
        LearnedRule::FilenameRegex {
            pattern,
            tags,
            confidence,
        } => {
            // None means the pattern failed to compile in `compile_one`
            // — we skip it silently rather than poison the whole call.
            cr.regex.as_ref().and_then(|re| {
                if re.is_match(rel_path) {
                    Some((
                        tags.as_slice(),
                        *confidence,
                        format!("ai · regex {pattern}"),
                    ))
                } else {
                    None
                }
            })
        }
        // Unrecognized kind (serde catch-all). Providers strip these right
        // after parse; a hand-edited tidycraft.ai.toml can still carry one,
        // and it simply never matches.
        LearnedRule::Unknown => None,
    }
}

fn relative_path(root: &str, abs_path: &str) -> String {
    let prefix = format!("{root}/");
    abs_path
        .strip_prefix(&prefix)
        .unwrap_or(abs_path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{AssetInfo, AssetMetadata, AssetType, DirectoryNode};

    fn asset(path: &str) -> AssetInfo {
        AssetInfo {
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            extension: path.rsplit('.').next().unwrap_or("").into(),
            asset_type: AssetType::Texture,
            size: 0,
            modified: 0,
            metadata: Some(AssetMetadata::default()),
            unity_guid: None,
        }
    }

    fn scan(root: &str, paths: &[&str]) -> ScanResult {
        ScanResult {
            root_path: root.into(),
            directory_tree: DirectoryNode {
                name: "".into(),
                path: root.into(),
                children: vec![],
                file_count: 0,
                total_size: 0,
            },
            total_count: paths.len(),
            total_size: 0,
            type_counts: HashMap::new(),
            project_type: None,
            assets: paths.iter().map(|p| asset(p)).collect(),
        }
    }

    #[test]
    fn empty_rules_yield_no_groups() {
        let s = scan("/p", &["/p/a.png"]);
        assert!(RuleSuggester::new(vec![]).suggest(&s).is_empty());
    }

    #[test]
    fn filename_token_matches_case_insensitive() {
        let s = scan(
            "/p",
            &["/p/T_Hero_BaseColor.png", "/p/T_Villain_Normal.png"],
        );
        let r = vec![LearnedRule::FilenameToken {
            pattern: "basecolor".into(),
            tags: vec!["diffuse-map".into()],
            confidence: 0.95,
        }];
        let groups = RuleSuggester::new(r).suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "diffuse-map");
        assert_eq!(groups[0].file_paths.len(), 1);
        assert!(groups[0].hint.contains("basecolor"));
    }

    #[test]
    fn path_prefix_and_segment_combine_under_one_label() {
        // Two rules both target "hero" — should merge into one group.
        let s = scan(
            "/p",
            &[
                "/p/Characters/Hero/T_Hero.png",
                "/p/Animations/hero/idle.anim",
            ],
        );
        let rules = vec![
            LearnedRule::PathPrefix {
                pattern: "Characters/Hero/".into(),
                tags: vec!["hero".into()],
                confidence: 0.99,
            },
            LearnedRule::PathSegment {
                pattern: "hero".into(),
                tags: vec!["hero".into()],
                confidence: 0.85,
            },
        ];
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "hero");
        assert_eq!(groups[0].file_paths.len(), 2);
        // Highest confidence wins.
        assert!((groups[0].confidence - 0.99).abs() < 1e-5);
        // Hint should reference the prefix rule (the higher-confidence one).
        assert!(groups[0].hint.contains("prefix"));
    }

    #[test]
    fn path_segment_does_not_partial_match() {
        let s = scan("/p", &["/p/Heroic/x.png", "/p/hero/y.png"]);
        let rules = vec![LearnedRule::PathSegment {
            pattern: "hero".into(),
            tags: vec!["hero".into()],
            confidence: 0.9,
        }];
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].file_paths.len(), 1);
        assert!(groups[0].file_paths[0].contains("/hero/"));
    }

    #[test]
    fn valid_regex_matches_relative_path() {
        // Regex applies to the project-relative path. Pattern only
        // hits the .fbx file, not the .png.
        let s = scan(
            "/p",
            &["/p/SM_Sword.fbx", "/p/T_Hero_BaseColor.png"],
        );
        let rules = vec![LearnedRule::FilenameRegex {
            pattern: r"^SM_.*\.fbx$".into(),
            tags: vec!["static-mesh".into()],
            confidence: 0.95,
        }];
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "static-mesh");
        assert_eq!(groups[0].file_paths.len(), 1);
        assert!(groups[0].file_paths[0].ends_with("SM_Sword.fbx"));
        assert!(groups[0].hint.contains("regex"));
    }

    #[test]
    fn prompt_example_regex_is_alive_in_subdirectories() {
        // The learning system prompt's filename_regex example must produce a
        // rule that matches files inside subdirectories. The old example
        // `^SM_.*\.fbx$` anchored to the START OF THE PATH — models imitating
        // it emitted rules that never matched anything below the project
        // root ("dead rules" the review panel's syntax check can't catch).
        let s = scan(
            "/p",
            &[
                "/p/SM_Root.fbx",
                "/p/Props/Rocks/SM_Rock.fbx",
                "/p/Props/XSM_NotAMatch.fbx",
            ],
        );
        let rules = vec![LearnedRule::FilenameRegex {
            pattern: r"(^|/)SM_[^/]*\.fbx$".into(),
            tags: vec!["static-mesh".into()],
            confidence: 0.9,
        }];
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 1);
        let mut hits = groups[0].file_paths.clone();
        hits.sort();
        assert_eq!(hits.len(), 2, "root + subdirectory files must both match");
        assert!(hits[0].ends_with("Props/Rocks/SM_Rock.fbx"));
        assert!(hits[1].ends_with("SM_Root.fbx"));

        // And the prompt's JSON example must carry the live pattern — guard
        // against a regression back to the dead string-start anchor. (The
        // prose may still MENTION "^SM_" as the counter-example; only the
        // example rule's pattern value matters, since that's what models
        // imitate.)
        let prompt = crate::llm::prompts::SYSTEM_PROMPT_LEARNING;
        assert!(prompt.contains(r#""pattern": "(^|/)SM_[^/]*\\.fbx$""#));
        assert!(!prompt.contains(r#""pattern": "^SM_"#));
    }

    #[test]
    fn invalid_regex_is_skipped_and_reported_other_rules_still_fire() {
        // A malformed regex should NOT poison the whole call — it's
        // skipped at compile time, the remaining rules carry on. But the
        // skip has to be visible: the user saved that rule and would
        // otherwise just see one fewer group, with no way to tell whether
        // the rule is broken or simply matched nothing.
        let s = scan("/p", &["/p/SM_Sword.fbx", "/p/T_Hero.png"]);
        let rules = vec![
            LearnedRule::FilenameRegex {
                pattern: "[unbalanced(".into(),
                tags: vec!["broken".into()],
                confidence: 0.95,
            },
            LearnedRule::FilenameToken {
                pattern: "Hero".into(),
                tags: vec!["hero".into()],
                confidence: 0.99,
            },
        ];
        let suggester = RuleSuggester::new(rules);
        let groups = suggester.suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "hero");

        assert_eq!(suggester.warnings.len(), 1);
        match &suggester.warnings[0] {
            RuleWarning::InvalidPattern { pattern, detail } => {
                assert_eq!(pattern, "[unbalanced(");
                assert!(!detail.is_empty(), "the compile error explains the fix");
            }
            other => panic!("expected InvalidPattern, got {other:?}"),
        }
    }

    #[test]
    fn a_rule_set_that_compiles_reports_nothing() {
        let rules = vec![
            LearnedRule::FilenameRegex {
                pattern: r"(^|/)SM_[^/]*\.fbx$".into(),
                tags: vec!["static-mesh".into()],
                confidence: 0.9,
            },
            LearnedRule::FilenameToken {
                pattern: "Hero".into(),
                tags: vec!["hero".into()],
                confidence: 0.99,
            },
        ];
        assert!(RuleSuggester::new(rules).warnings.is_empty());
    }

    /// A pattern JS `RegExp` accepts but Rust's engine does not. This is the
    /// path a real dead rule takes: the review panel's syntax check passes
    /// it, the user saves it, and it matches nothing forever.
    #[test]
    fn a_lookahead_survives_review_and_is_reported_here() {
        let rules = vec![LearnedRule::FilenameRegex {
            pattern: r"^(?=.*Hero).*\.png$".into(),
            tags: vec!["hero".into()],
            confidence: 0.9,
        }];
        let warnings = RuleSuggester::new(rules).warnings;
        assert_eq!(warnings.len(), 1, "look-around must not compile here");
        assert!(matches!(
            warnings[0],
            RuleWarning::InvalidPattern { .. }
        ));
    }

    #[test]
    fn a_corrupt_rules_file_falls_back_to_heuristics_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("tidycraft.ai.toml"),
            "this is not = = valid toml",
        )
        .unwrap();

        let root = crate::scanner::path_to_string(dir.path());
        let out = load_or_fallback(&heuristic_friendly_scan(&root), dir.path());

        assert_eq!(out.warnings.len(), 1);
        assert!(matches!(
            out.warnings[0],
            RuleWarning::RulesUnreadable { .. }
        ));
        // Fallback still ran — the panel is not left empty.
        assert!(!out.groups.is_empty());
    }

    #[test]
    fn no_rules_file_falls_back_without_a_warning() {
        // The overwhelmingly common case: the user has never run AI
        // Learning. Nothing is wrong, so nothing may be reported.
        let dir = tempfile::tempdir().unwrap();
        let root = crate::scanner::path_to_string(dir.path());
        assert!(load_or_fallback(&heuristic_friendly_scan(&root), dir.path())
            .warnings
            .is_empty());
    }

    /// `AITagPanel.tsx` mirrors this enum by hand and branches on the exact
    /// `kind` strings — there is no codegen between the two sides. Renaming
    /// a variant without touching the mirror would take the wrong branch and
    /// render an interpolated `undefined`, which is how the warning would
    /// end up as unreadable as the stderr line it replaced.
    #[test]
    fn warning_wire_shape_matches_the_frontends_mirror() {
        let unreadable = serde_json::to_value(RuleWarning::RulesUnreadable {
            detail: "expected `=`".into(),
        })
        .unwrap();
        assert_eq!(unreadable["kind"], "rules_unreadable");
        assert_eq!(unreadable["detail"], "expected `=`");

        let invalid = serde_json::to_value(RuleWarning::InvalidPattern {
            pattern: "[unbalanced(".into(),
            detail: "unclosed character class".into(),
        })
        .unwrap();
        assert_eq!(invalid["kind"], "invalid_pattern");
        assert_eq!(invalid["pattern"], "[unbalanced(");
        assert_eq!(invalid["detail"], "unclosed character class");

        let envelope = serde_json::to_value(TagSuggestions {
            groups: Vec::new(),
            warnings: Vec::new(),
        })
        .unwrap();
        assert!(envelope["groups"].is_array());
        assert!(envelope["warnings"].is_array());
    }

    /// Enough files sharing a filename token to clear `MIN_TOKEN_HITS`, so
    /// the heuristic fallback actually produces a group to assert on.
    fn heuristic_friendly_scan(root: &str) -> ScanResult {
        let paths: Vec<String> = ["Hero", "Villain", "Rock", "Tree"]
            .iter()
            .map(|n| format!("{root}/T_{n}_BaseColor.png"))
            .collect();
        scan(root, &paths.iter().map(|p| p.as_str()).collect::<Vec<_>>())
    }

    #[test]
    fn groups_sorted_by_confidence_desc() {
        let s = scan(
            "/p",
            &["/p/a_BaseColor.png", "/p/b_Normal.png", "/p/c_Roughness.png"],
        );
        let rules = vec![
            LearnedRule::FilenameToken {
                pattern: "BaseColor".into(),
                tags: vec!["diffuse".into()],
                confidence: 0.95,
            },
            LearnedRule::FilenameToken {
                pattern: "Normal".into(),
                tags: vec!["normal".into()],
                confidence: 0.99,
            },
            LearnedRule::FilenameToken {
                pattern: "Roughness".into(),
                tags: vec!["roughness".into()],
                confidence: 0.7,
            },
        ];
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].name, "normal");
        assert_eq!(groups[1].name, "diffuse");
        assert_eq!(groups[2].name, "roughness");
    }
}
