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

/// Load AI-derived rules from `<root>/tidycraft.ai.toml` and run them.
/// Falls back to `HeuristicSuggester` when no rules file is present or
/// the rules list is empty (i.e. user hasn't run AI Learning yet).
/// Errors propagate only for genuinely corrupt rule files; the caller
/// in `lib.rs::suggest_tags` logs + falls back further so the UI never
/// breaks.
pub fn load_or_fallback(
    scan: &ScanResult,
    project_root: &std::path::Path,
) -> Result<Vec<TagGroup>, String> {
    let doc = crate::llm::rule_store::AiRulesDoc::load(project_root)?;
    match doc {
        Some(d) if !d.rules.is_empty() => Ok(RuleSuggester::new(d.rules).suggest(scan)),
        _ => Ok(super::tag_suggest::HeuristicSuggester.suggest(scan)),
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
}

impl RuleSuggester {
    pub fn new(rules: Vec<LearnedRule>) -> Self {
        let compiled = rules.into_iter().map(compile_one).collect();
        Self { rules: compiled }
    }
}

/// Compile one rule. For `FilenameRegex`, attempts `Regex::new`; on
/// failure logs a one-shot warning and stores `None` so the rule
/// silent-skips at match time. We deliberately do NOT propagate the
/// error — a single malformed pattern shouldn't poison the whole rule
/// set when the rest are usable. The LearnReviewPanel runs a similar
/// validity check on the UI side via JS `RegExp` — close enough for
/// the simple patterns the LLM emits, but the dialects diverge both
/// ways: JS accepts what this engine rejects (backreferences `\1`,
/// look-around `(?=`), and those land here and silent-skip; while this
/// engine accepts what JS rejects (`(?P<name>...)` named groups), so
/// the panel can warn about a pattern that compiles fine here.
fn compile_one(rule: LearnedRule) -> CompiledRule {
    let regex = match &rule {
        LearnedRule::FilenameRegex { pattern, .. } => match Regex::new(pattern) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!(
                    "[rule_suggest] skipping invalid regex pattern {pattern:?}: {e}"
                );
                None
            }
        },
        _ => None,
    };
    CompiledRule { rule, regex }
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
    fn invalid_regex_silently_skipped_other_rules_still_fire() {
        // A malformed regex should NOT poison the whole call — it's
        // skipped at compile time, the remaining rules carry on.
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
        let groups = RuleSuggester::new(rules).suggest(&s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "hero");
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
