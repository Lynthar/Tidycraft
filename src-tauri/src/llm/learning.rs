//! Learning-mode schemas (sampling + rule generation).
//!
//! Distinct from per-asset tagging (`TagRequest` / `TagResponse`) because:
//!   - Input is whole-project metadata (samples + tag system + project
//!     framing), not a list of specific assets.
//!   - Output includes inferred conventions, tag-gap suggestions, and
//!     LEARNED RULES (filename pattern → tags) that drive a local
//!     RuleSuggester — the LLM is teacher, not labeler.
//!   - Lives in its own file so the per-asset path stays readable.
//!
//! Persistence: serialized into `<project>/tidycraft.ai.toml` after the
//! user reviews and accepts in LearnReviewPanel (Day 7+). Re-running
//! learning overwrites this file; the toml comment header warns users.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{project_meta::ProjectMeta, ExistingTagContext, TagCategory, Usage};

/// Cache-busting version for the learning prompt + output schema.
/// Bump on any change to `SYSTEM_PROMPT_LEARNING` semantics or
/// `LearningResult` shape so older `tidycraft.ai.toml` files can be
/// detected as stale (frontend nudges user to re-learn).
pub const LEARNING_PROMPT_VERSION: u32 = 1;

// ============ Inputs ============

/// One sampled file fed to the LLM. `asset_type` is the scanner's
/// classification (texture/model/audio/etc.); the model uses it to
/// disambiguate naming patterns that overlap across types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleFile {
    pub filename: String,
    pub extension: String,
    pub asset_type: String,
}

/// All files sampled from one directory. The directory path is given
/// relative to the project root so the LLM can reason about taxonomy
/// without ingesting absolute filesystem paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorySample {
    /// Project-relative directory path (forward-slash separators).
    /// Empty string = project root.
    pub rel_path: String,
    /// Total file count in the directory (not just sample size) — gives
    /// the model a sense of which directories are "deep" and likely to
    /// have a stable convention vs. catch-all dirs with mixed content.
    pub total_files: usize,
    pub files: Vec<SampleFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnRequest {
    pub samples: Vec<DirectorySample>,
    /// Theme/goal from `tidycraft.toml [project]`. Same shape as the
    /// per-asset path uses.
    #[serde(default)]
    pub project_meta: Option<ProjectMeta>,
    /// User's existing tag system. Even more important here than in
    /// per-asset tagging because the whole point of learning is to
    /// build rules that target THESE tags.
    #[serde(default)]
    pub existing_tags: Vec<ExistingTagContext>,
    pub model: String,
    /// User-controlled sampler depth (3-30). Stored on the request so
    /// it lands in the result metadata for tidycraft.ai.toml.
    pub sampling_depth: usize,
    /// `LEARNING_PROMPT_VERSION` at request time. Echoed in the result
    /// so we can detect stale cached files later.
    pub prompt_version: u32,
}

// ============ Output ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferredConventions {
    /// Free-form: e.g. "PascalCase with type prefix (T_/SM_/M_)".
    /// Empty when the model couldn't extract a stable convention.
    pub naming: String,
    /// Free-form: e.g. "Organized by faction; Characters/{name}/...".
    pub directories: String,
    /// Per-existing-tag explanation of how the model interprets it.
    /// Keys are tag names (matching `ExistingTagContext.name` in the
    /// request). Values are the model's understanding sentence-level.
    /// Useful for surfacing "the AI thinks `hero` means X" so the user
    /// can correct misunderstandings via tag descriptions.
    #[serde(default)]
    pub existing_tag_meanings: HashMap<String, String>,
}

/// Tags assigned to one sample file. Categorized by source the same
/// way per-asset suggestions are.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleTagSet {
    /// Project-relative path to the sampled file (matches some
    /// `DirectorySample.files[i]` filename joined with rel_path).
    pub asset_path: String,
    /// Existing tag names that match (match by name; frontend resolves
    /// to tag IDs).
    #[serde(default)]
    pub matched_existing: Vec<String>,
    /// Brand-new tag suggestions for this file. Each is a (label,
    /// category, confidence) triple — same shape as per-asset
    /// `SuggestedTag` minus the `source` field (always implicitly new).
    #[serde(default)]
    pub suggested_new: Vec<NewTagHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTagHint {
    pub label: String,
    pub category: TagCategory,
    pub confidence: f32,
}

/// A tag the model thinks the user's vocabulary is missing. Promoted
/// to actual tag creation in LearnReviewPanel (Day 7+) — by default
/// auto-created (per #6 decision), with the panel showing "AI added
/// N new tags" so the user can revoke.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagGap {
    pub label: String,
    pub category: TagCategory,
    /// Why the model thinks this gap exists. Surfaced in the review UI
    /// so the user can accept/reject with informed judgment.
    pub reason: String,
}

/// One heuristic rule the model derived. Fed into `RuleSuggester`
/// (Day 7) which runs them across the full project to produce
/// `TagGroup[]` for the existing AITagPanel UI.
///
/// The four `kind`s cover the patterns that came up across the design
/// discussion. Free-form regex is included for power-user escape
/// hatches, but the model is asked to prefer the simpler kinds when
/// possible (regex is harder to edit and review).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearnedRule {
    /// Match if filename (basename) contains the literal token,
    /// case-insensitive. Most common rule type.
    FilenameToken {
        pattern: String,
        tags: Vec<String>,
        confidence: f32,
    },
    /// Match if relative path starts with the literal prefix.
    /// Used for "everything under Characters/Hero/ is a hero asset".
    PathPrefix {
        pattern: String,
        tags: Vec<String>,
        confidence: f32,
    },
    /// Match if relative path contains the literal segment as a full
    /// path component. e.g. segment="hero" matches "a/hero/b" and
    /// "hero/x" but not "a/heroic/b".
    PathSegment {
        pattern: String,
        tags: Vec<String>,
        confidence: f32,
    },
    /// Free-form regex applied to the full relative path. Power tool
    /// — use sparingly. Confidence guidance asks the model to set this
    /// only when simpler kinds don't fit.
    FilenameRegex {
        pattern: String,
        tags: Vec<String>,
        confidence: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningResult {
    pub inferred_conventions: InferredConventions,
    #[serde(default)]
    pub sample_tags: Vec<SampleTagSet>,
    #[serde(default)]
    pub tag_gaps: Vec<TagGap>,
    #[serde(default)]
    pub rules: Vec<LearnedRule>,
    #[serde(default)]
    pub usage: Usage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_rule_serializes_with_kind_tag() {
        let r = LearnedRule::FilenameToken {
            pattern: "BaseColor".into(),
            tags: vec!["diffuse-map".into()],
            confidence: 0.95,
        };
        let json = serde_json::to_string(&r).unwrap();
        // `serde(tag = "kind", rename_all = "snake_case")` should emit
        // {"kind":"filename_token", ...} — matching the LLM-output
        // schema documented in SYSTEM_PROMPT_LEARNING.
        assert!(json.contains("\"kind\":\"filename_token\""));
        assert!(json.contains("\"pattern\":\"BaseColor\""));
    }

    #[test]
    fn learning_result_round_trips() {
        let result = LearningResult {
            inferred_conventions: InferredConventions {
                naming: "Lowercase with underscores".into(),
                directories: "Flat".into(),
                existing_tag_meanings: HashMap::new(),
            },
            sample_tags: vec![],
            tag_gaps: vec![TagGap {
                label: "diffuse-map".into(),
                category: TagCategory::Type,
                reason: "Many *_BaseColor.png lack a channel-level tag".into(),
            }],
            rules: vec![LearnedRule::PathPrefix {
                pattern: "Characters/Hero/".into(),
                tags: vec!["hero".into()],
                confidence: 0.99,
            }],
            usage: Usage::default(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: LearningResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tag_gaps.len(), 1);
        assert_eq!(back.rules.len(), 1);
    }

    #[test]
    fn missing_optional_fields_default_cleanly() {
        // LLM output is instructed NOT to emit `usage` (it's API
        // metadata, filled in by the provider after parse), and older
        // tidycraft.ai.toml files may lack sample_tags / tag_gaps /
        // rules. All four must default rather than fail the load.
        let json = r#"{
            "inferred_conventions": {
                "naming": "",
                "directories": "",
                "existing_tag_meanings": {}
            }
        }"#;
        let result: LearningResult = serde_json::from_str(json).unwrap();
        assert!(result.sample_tags.is_empty());
        assert!(result.tag_gaps.is_empty());
        assert!(result.rules.is_empty());
        assert_eq!(result.usage.input_tokens, 0);
        assert_eq!(result.usage.output_tokens, 0);
        assert!(!result.usage.cached);
    }

    #[test]
    fn parses_llm_response_shape_without_usage_field() {
        // Regression for the learn_project parse path:
        // SYSTEM_PROMPT_LEARNING tells the model to emit exactly
        // {inferred_conventions, sample_tags, tag_gaps, rules} with no
        // `usage`. Deserialization must accept that shape across every
        // LearnedRule kind.
        let json = r#"{
            "inferred_conventions": {
                "naming": "PascalCase with type prefix",
                "directories": "Organized by subject",
                "existing_tag_meanings": { "hero": "main character assets" }
            },
            "sample_tags": [{
                "asset_path": "Cactus/cactus.png",
                "matched_existing": ["hero"],
                "suggested_new": [
                    { "label": "cactus", "category": "subject", "confidence": 0.98 }
                ]
            }],
            "tag_gaps": [
                { "label": "flower", "category": "subject", "reason": "no flower tag yet" }
            ],
            "rules": [
                { "kind": "filename_token", "pattern": "BaseColor",  "tags": ["texture"], "confidence": 0.95 },
                { "kind": "path_prefix",    "pattern": "Characters/","tags": ["hero"],    "confidence": 0.99 },
                { "kind": "path_segment",   "pattern": "Cactus",     "tags": ["cactus"],  "confidence": 0.99 },
                { "kind": "filename_regex", "pattern": "^model\\.obj$","tags": ["model"], "confidence": 0.99 }
            ]
        }"#;
        let result: LearningResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.sample_tags.len(), 1);
        assert_eq!(result.tag_gaps.len(), 1);
        assert_eq!(result.rules.len(), 4);
        assert_eq!(result.usage.input_tokens, 0);
    }
}
