//! System prompt + user-prompt builder.
//!
//! Version-bump policy: increment `PROMPT_VERSION` whenever the SYSTEM_PROMPT
//! changes meaning (new categories, different output schema, stricter
//! confidence cutoff, etc.). The version is part of the cache key, so a
//! bump invalidates every previously cached suggestion. Cosmetic edits
//! (typo fixes, whitespace, reordering equivalent phrases) do NOT
//! warrant a bump.

use std::fmt::Write;

use super::{project_meta::ProjectMeta, AssetInput, ExistingTagContext};

/// Cache-busting prompt version — see module-level doc.
///
/// History:
/// - v1: initial prompt; per-asset analysis only.
/// - v2 (2026-05-09): adds project-context + existing-tag blocks and
///   instructs the LLM to mark suggestions as `existing` vs `new`.
pub const PROMPT_VERSION: u32 = 2;

/// Instructs the model on output schema, allowed categories, and rules.
/// Kept in English: the LLM ecosystem is overwhelmingly English-native
/// and switching to Chinese here measurably degrades response quality
/// even when the user's UI is zh.
pub const SYSTEM_PROMPT: &str = r#"You are an art-asset taxonomist for game development. Look at each provided asset (thumbnail + filename + path) and suggest tags that capture content and style — NOT format or technical metadata (the user already sees that).

You MAY be given two extra context blocks before the asset list:
- A "Project context" block with the project's theme and goal — use it to disambiguate styles and pick subject terms that fit the project.
- An "Existing project tags" block listing tags the user has already defined, with optional descriptions and example asset paths. PREFER these labels whenever a suggestion fits — match the user's vocabulary instead of inventing synonyms. Only invent a new label when no existing tag captures the meaning.

Output strict JSON:
{
  "suggestions": [
    {
      "asset_path": "...",
      "tags": [
        { "label": "character", "category": "type", "confidence": 0.95, "source": "existing" },
        { "label": "cyberpunk", "category": "style", "confidence": 0.78, "source": "new" }
      ]
    }
  ]
}

Categories: type | style | mood | subject | other
- type: what the asset depicts (character/vehicle/prop/scene/ui/vfx/weapon/nature)
- style: visual approach (cartoon/realistic/cyberpunk/fantasy/pixel-art/lowpoly/anime/hand-painted)
- mood: emotional register (dark/bright/dramatic/playful)
- subject: free-form noun if more specific than `type` ("rusty-metal", "wolf", "spaceship")
- other: anything else

`source` field:
- "existing" → the `label` matches one of the names in the Existing project tags block (case-sensitive).
- "new" → no existing tag fit; you coined this label.

Rules:
- ONE tag per category at most. If unsure, omit the category.
- Confidence: 0.9+ = obvious, 0.6-0.9 = likely, below 0.6 = skip the tag entirely.
- Don't invent. If a thumbnail isn't provided, only use filename + path.
- Don't repeat extension/dimension info ("png", "1024px") — user has those.
- Output JSON only. No commentary."#;

/// Build the user-message body. Layout:
///
/// ```text
/// Project context:
/// - Theme: ...
/// - Goal: ...
///
/// Existing project tags (prefer these — match by name when applicable):
/// - "hero" — Player-controlled characters
///   used on: a/b.png, c/d.png, ...
/// - "weapon"
///   used on: ...
///
/// Analyze N asset(s):
///
/// Asset 1:
/// - path: ...
/// - filename: ...
/// - [thumbnail attached]
/// ```
///
/// Each context block is omitted entirely when its source is empty
/// (no project meta / no existing tags) so simple use-cases don't pay
/// for the framing overhead.
///
/// The thumbnail bytes themselves are attached via per-provider content-
/// block APIs (image blocks for Claude, image_url for OpenAI, images
/// array for Ollama); here we only emit the textual scaffold.
pub fn build_user_prompt(
    assets: &[AssetInput],
    project_ctx: Option<&ProjectMeta>,
    existing_tags: &[ExistingTagContext],
    include_thumbnails: bool,
) -> String {
    let mut out =
        String::with_capacity(128 + assets.len() * 96 + existing_tags.len() * 80);

    // ---- Project context block ----
    if let Some(meta) = project_ctx {
        if !meta.is_empty() {
            let _ = writeln!(out, "Project context:");
            if let Some(theme) = meta.theme.as_deref() {
                let _ = writeln!(out, "- Theme: {theme}");
            }
            if let Some(goal) = meta.goal.as_deref() {
                let _ = writeln!(out, "- Goal: {goal}");
            }
            let _ = writeln!(out);
        }
    }

    // ---- Existing tags block ----
    if !existing_tags.is_empty() {
        let _ = writeln!(
            out,
            "Existing project tags (prefer these — match by name when applicable):"
        );
        for tag in existing_tags {
            // Quote the name so single-word vs phrase tags align visually.
            if let Some(desc) = tag.description.as_deref() {
                let _ = writeln!(out, "- \"{}\" — {desc}", tag.name);
            } else {
                let _ = writeln!(out, "- \"{}\"", tag.name);
            }
            // Only include "used on" when we actually have samples.
            // 5 samples/tag is the cap; truncate longer lists with an ellipsis.
            if !tag.sample_paths.is_empty() {
                let preview = tag.sample_paths.join(", ");
                let _ = writeln!(out, "  used on: {preview}");
            }
        }
        let _ = writeln!(out);
    }

    // ---- Asset list ----
    let _ = writeln!(out, "Analyze {} asset(s):", assets.len());
    for (i, a) in assets.iter().enumerate() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Asset {}:", i + 1);
        let _ = writeln!(out, "- path: {}", a.path);
        let _ = writeln!(out, "- filename: {}", a.filename);
        if let Some(hint) = &a.metadata_hint {
            let _ = writeln!(out, "- metadata: {hint}");
        }
        if include_thumbnails && a.thumbnail_base64.is_some() {
            let _ = writeln!(out, "- [thumbnail attached]");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str, with_thumb: bool) -> AssetInput {
        AssetInput {
            path: path.into(),
            filename: path.rsplit('/').next().unwrap_or(path).into(),
            thumbnail_base64: if with_thumb { Some("xxx".into()) } else { None },
            metadata_hint: None,
        }
    }

    #[test]
    fn system_prompt_mentions_all_five_categories() {
        for cat in ["type", "style", "mood", "subject", "other"] {
            assert!(
                SYSTEM_PROMPT.contains(cat),
                "system prompt should describe `{cat}` category"
            );
        }
    }

    #[test]
    fn user_prompt_lists_every_asset_path_and_filename() {
        let assets = vec![
            asset("Characters/Hero/diffuse.png", true),
            asset("Props/barrel/wood.png", true),
        ];
        let p = build_user_prompt(&assets, None, &[], true);
        assert!(p.contains("Asset 1:"));
        assert!(p.contains("Asset 2:"));
        assert!(p.contains("Characters/Hero/diffuse.png"));
        assert!(p.contains("Props/barrel/wood.png"));
        assert!(p.contains("- filename: diffuse.png"));
        assert!(p.contains("- filename: wood.png"));
    }

    #[test]
    fn thumbnail_marker_omitted_when_disabled() {
        let assets = vec![asset("a/x.png", true)];
        let with = build_user_prompt(&assets, None, &[], true);
        let without = build_user_prompt(&assets, None, &[], false);
        assert!(with.contains("[thumbnail attached]"));
        assert!(!without.contains("[thumbnail attached]"));
    }

    #[test]
    fn thumbnail_marker_omitted_when_asset_has_no_thumbnail() {
        // Even with include_thumbnails=true, an asset that didn't
        // actually attach bytes (e.g. a non-image asset) shouldn't
        // get the "[thumbnail attached]" line.
        let assets = vec![asset("a/x.fbx", false)];
        let p = build_user_prompt(&assets, None, &[], true);
        assert!(!p.contains("[thumbnail attached]"));
    }

    #[test]
    fn metadata_hint_emitted_when_present() {
        let mut a = asset("t.png", true);
        a.metadata_hint = Some("1024×1024 albedo texture".into());
        let p = build_user_prompt(&[a], None, &[], true);
        assert!(p.contains("- metadata: 1024×1024 albedo texture"));
    }

    #[test]
    fn empty_asset_list_produces_zero_count_header() {
        let p = build_user_prompt(&[], None, &[], true);
        assert!(p.contains("Analyze 0 asset"));
    }

    // ---- New v2 context blocks ----

    #[test]
    fn project_context_block_emitted_when_meta_set() {
        let meta = ProjectMeta {
            theme: Some("Cyberpunk RPG".into()),
            goal: Some("Asset library".into()),
        };
        let assets = vec![asset("a/x.png", true)];
        let p = build_user_prompt(&assets, Some(&meta), &[], true);
        assert!(p.contains("Project context:"));
        assert!(p.contains("Theme: Cyberpunk RPG"));
        assert!(p.contains("Goal: Asset library"));
    }

    #[test]
    fn project_context_block_omitted_when_meta_empty() {
        // is_empty() should short-circuit so we don't print the header
        // for a meta object with all-None fields.
        let p = build_user_prompt(
            &[asset("a/x.png", true)],
            Some(&ProjectMeta::default()),
            &[],
            true,
        );
        assert!(!p.contains("Project context:"));
    }

    #[test]
    fn existing_tags_block_emitted_with_descriptions_and_samples() {
        let tags = vec![
            ExistingTagContext {
                name: "hero".into(),
                description: Some("Player-controlled chars".into()),
                sample_paths: vec!["Characters/Hero/diffuse.png".into()],
            },
            ExistingTagContext {
                name: "weapon".into(),
                description: None,
                sample_paths: vec!["Weapons/Sword/SM_BroadSword.fbx".into()],
            },
        ];
        let p = build_user_prompt(&[asset("a/x.png", true)], None, &tags, true);
        assert!(p.contains("Existing project tags"));
        assert!(p.contains("\"hero\" — Player-controlled chars"));
        assert!(p.contains("\"weapon\""));
        assert!(p.contains("used on: Characters/Hero/diffuse.png"));
        assert!(p.contains("used on: Weapons/Sword/SM_BroadSword.fbx"));
    }

    #[test]
    fn existing_tags_block_omitted_when_empty() {
        let p = build_user_prompt(&[asset("a/x.png", true)], None, &[], true);
        assert!(!p.contains("Existing project tags"));
    }

    #[test]
    fn both_blocks_render_before_asset_list() {
        let meta = ProjectMeta {
            theme: Some("X".into()),
            goal: None,
        };
        let tags = vec![ExistingTagContext {
            name: "hero".into(),
            description: None,
            sample_paths: vec![],
        }];
        let p = build_user_prompt(&[asset("a/x.png", true)], Some(&meta), &tags, true);
        let proj_idx = p.find("Project context:").unwrap();
        let tag_idx = p.find("Existing project tags").unwrap();
        let asset_idx = p.find("Analyze ").unwrap();
        assert!(proj_idx < tag_idx);
        assert!(tag_idx < asset_idx);
    }
}
