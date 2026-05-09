//! LLM-backed asset tagging.
//!
//! Plumbing for the "AI Tag" flow described in `docs/ai-tagging-plan.md`.
//! The user picks an asset selection in the UI; the frontend invokes
//! `llm_estimate_cost` to render a confirm modal, then `llm_suggest_tags`
//! once the user accepts. Each provider (Claude / OpenAI / Ollama) implements
//! the same trait so swapping is a config change, not a code branch.
//!
//! Day 1 scope (this commit): trait + schemas + cache + cost + prompts +
//! placeholder providers that return `LLMError::NotImplemented`. Day 2 wires
//! the actual HTTP calls.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod cache;
pub mod claude;
pub mod cost;
pub mod ollama;
pub mod openai;
pub mod project_meta;
pub mod prompts;

// ============ Data schemas ============
//
// Mirrors the TS-side shapes the frontend sends/expects via `invoke()`.
// Keep field names and snake_case in sync with `src/types/asset.ts` and
// the `aiAnalyze`/`aiResult` UI components when those land.

/// One LLM tagging call. The provider receives this and returns a
/// `TagResponse` covering every asset in the same order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRequest {
    pub assets: Vec<AssetInput>,
    /// Bumping `PROMPT_VERSION` invalidates every cache entry — see
    /// `prompts.rs`. Stored on the cache key so an old cached response
    /// from a stale prompt cannot be served.
    pub prompt_version: u32,
    /// Provider-specific model id (e.g. "claude-sonnet-4-6", "gpt-5.4-mini",
    /// "qwen2.5vl:32b"). Cached separately per model so users can
    /// upgrade their default without losing prior runs.
    pub model: String,
    /// When false, providers must skip image content and use filename +
    /// path only. Used for text-only fallback and for users who haven't
    /// consented to thumbnail upload.
    #[serde(default = "default_true")]
    pub include_thumbnails: bool,
    /// Optional project framing pulled from `tidycraft.toml [project]`.
    /// `None` (or both fields empty) → prompt builder skips the
    /// project-context block to save tokens. Defaults to None on
    /// deserialization for backward-compat with older request shapes.
    #[serde(default)]
    pub project_ctx: Option<project_meta::ProjectMeta>,
    /// User's existing tag system. The LLM is instructed to prefer
    /// these labels over inventing new ones. Empty vec → no
    /// existing-tag block emitted.
    #[serde(default)]
    pub existing_tags: Vec<ExistingTagContext>,
}

/// Per-tag context fed to the LLM so it can match existing project tags.
///
/// `description` is the user's TagManager-supplied semantic blurb (when
/// available). `sample_paths` are up to 5 asset paths where this tag is
/// currently applied — they let the LLM infer the tag's intent from
/// usage even when no description is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistingTagContext {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_paths: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// One asset's input to the LLM. `thumbnail_base64` is None for non-image
/// assets or when the user opted out of thumbnail upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInput {
    /// Project-relative path. Used in the prompt for context (folder
    /// structure often hints at asset purpose) and as part of the cache key.
    pub path: String,
    pub filename: String,
    pub thumbnail_base64: Option<String>,
    /// Optional one-liner like "1024×1024 texture / 5k vertex model" so
    /// the LLM doesn't have to infer technical details we already know.
    pub metadata_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagResponse {
    pub suggestions: Vec<TagSuggestion>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSuggestion {
    /// Echoes `AssetInput.path` so the UI can match suggestions to the
    /// original asset rows even if the provider reorders the response.
    pub asset_path: String,
    pub tags: Vec<SuggestedTag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedTag {
    pub label: String,
    pub category: TagCategory,
    /// 0.0..=1.0; the prompt instructs models to skip tags below 0.6.
    pub confidence: f32,
    /// Marks whether the LLM matched this tag against the user's
    /// existing tag system or invented a new label. Frontend uses this
    /// to skip the `(AI)` suffix and color-by-existing-tag for
    /// `Existing` chips. Defaults to `New` on deserialization so
    /// older cached responses (which lack the field) load cleanly.
    #[serde(default)]
    pub source: TagSource,
}

/// Whether a `SuggestedTag` matches an existing project tag or is a
/// brand-new label coined by the LLM.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TagSource {
    /// LLM coined this label — frontend will create a new tag with the
    /// `(AI)` suffix for disambiguation.
    #[default]
    New,
    /// Matches an existing project tag's `name` (case-sensitive).
    /// Frontend resolves it to the existing tag id and applies directly,
    /// no new tag created.
    Existing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TagCategory {
    /// What the asset depicts (character / vehicle / prop / scene / ui / vfx / weapon / nature).
    Type,
    /// Visual approach (cartoon / realistic / cyberpunk / pixel-art / lowpoly / hand-painted).
    Style,
    /// Emotional register (dark / bright / dramatic / playful).
    Mood,
    /// Free-form noun more specific than `type` (e.g. "rusty-metal", "wolf").
    Subject,
    /// Anything that doesn't fit the four buckets above.
    Other,
}

/// Precomputed cost preview shown to the user before they confirm a call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostEstimate {
    pub input_tokens: usize,
    pub output_tokens_estimate: usize,
    /// Cents (rounded up to whole cents) so the UI can render `$0.12`
    /// without floating-point dollars.
    pub usd_cents: u32,
}

/// Actual usage returned alongside `TagResponse`. `cached=true` means the
/// entire response came from disk cache — no provider was called.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cached: bool,
}

// ============ Errors ============

#[derive(Error, Debug)]
pub enum LLMError {
    #[error("API key not configured for provider {0}")]
    NoApiKey(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Rate limit or quota exceeded")]
    RateLimit,
    #[error("Failed to parse provider response: {0}")]
    ParseError(String),
    #[error("Provider {0} not enabled in settings")]
    ProviderDisabled(String),
    /// Day 1 placeholder — every provider returns this until Day 2 wires
    /// the real HTTP calls. UI shows it as a friendly "AI tagging is in
    /// development" message.
    #[error("Provider not implemented yet — Day 2 work")]
    NotImplemented,
    #[error("LLM error: {0}")]
    Other(String),
}

// Tauri commands return `Result<T, String>`. The boundary conversion
// lives here so providers can `?` LLMError up to the command without
// each command re-mapping it.
impl From<LLMError> for String {
    fn from(e: LLMError) -> String {
        e.to_string()
    }
}

// ============ Provider trait ============

#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Stable identifier used as a cache-key component and as the
    /// `aiActiveProvider` value persisted in `settingsStore`. Must be
    /// lowercase ASCII (e.g. "claude", "openai", "ollama").
    fn id(&self) -> &str;

    /// Estimate input/output tokens and USD cents for `request`. Pure
    /// function — no network. Drives the per-call confirm modal.
    fn estimate_cost(&self, request: &TagRequest) -> CostEstimate;

    /// Make the actual API call. Day 1 stubs return
    /// `LLMError::NotImplemented`; Day 2 fills in the HTTP work.
    async fn suggest_tags(&self, request: &TagRequest) -> Result<TagResponse, LLMError>;
}

// ============ Factory ============

/// What providers need to construct themselves from frontend settings.
pub struct ProviderConfig {
    /// None for Ollama (local, no auth). Required for cloud providers
    /// at call time — `suggest_tags` returns `LLMError::NoApiKey` if
    /// it's None when the cloud provider runs.
    pub api_key: Option<String>,
    /// Custom endpoint override. Ollama always uses this; OpenAI uses
    /// it for proxy/Azure deployments; Claude rarely overrides.
    pub endpoint: Option<String>,
    /// The model id selected by the user.
    pub model: String,
}

pub fn make_provider(id: &str, config: ProviderConfig) -> Result<Box<dyn LLMProvider>, LLMError> {
    match id {
        "claude" => Ok(Box::new(claude::ClaudeProvider::new(config))),
        "openai" => Ok(Box::new(openai::OpenAIProvider::new(config))),
        "ollama" => Ok(Box::new(ollama::OllamaProvider::new(config))),
        _ => Err(LLMError::ProviderDisabled(id.to_string())),
    }
}

// ============ Shared response parser ============

/// Three-tier JSON parser per plan §5.3.
///
/// 1. `serde_json::from_str` directly on the text.
/// 2. Strip a ```json``` (or bare ```...```) markdown fence and retry.
/// 3. Surface `LLMError::ParseError` with the original text so the UI
///    can show "model output couldn't be auto-applied, here's what it
///    said" instead of pretending nothing happened.
pub fn parse_suggestions(text: &str) -> Result<Vec<TagSuggestion>, LLMError> {
    #[derive(Deserialize)]
    struct Wrapped {
        suggestions: Vec<TagSuggestion>,
    }

    if let Ok(w) = serde_json::from_str::<Wrapped>(text) {
        return Ok(w.suggestions);
    }

    if let Some(stripped) = strip_markdown_fence(text) {
        if let Ok(w) = serde_json::from_str::<Wrapped>(stripped) {
            return Ok(w.suggestions);
        }
    }

    Err(LLMError::ParseError(text.to_string()))
}

/// Pull the body out of a ```json ... ``` (or bare ``` ... ```) fence.
/// Returns None if no fence found. The optional language tag must be
/// alphanumeric (e.g. `json`); arbitrary text on the opening line is
/// treated as content, not a tag.
fn strip_markdown_fence(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let start = trimmed.find("```")?;
    let after_open = &trimmed[start + 3..];
    let after_lang = if let Some(nl) = after_open.find('\n') {
        let prefix = &after_open[..nl];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_alphanumeric()) {
            &after_open[nl + 1..]
        } else {
            after_open
        }
    } else {
        after_open
    };
    let end = after_lang.rfind("```")?;
    Some(after_lang[..end].trim())
}

// ============ Shared cache + fetcher orchestration ============

/// Wraps a provider's actual API call with the per-asset cache.
///
/// Splits `request.assets` into hits (already cached) and misses,
/// calls `fetcher` exactly once with a misses-only request, persists
/// the fresh suggestions, then merges hits + fresh into the final
/// `TagResponse`. The provider-level call paths only have to know how
/// to make one batched request — caching, key generation, and merging
/// live here.
///
/// `fetcher` receives an OWNED `TagRequest` (so it can ship into an
/// async block / move into a closure freely) containing only the
/// missing assets. It must return suggestions in the same order as the
/// input assets — the cache save loop pairs them by index.
///
/// Returns `Usage { cached: true, tokens: 0 }` when every asset is a
/// cache hit (fetcher is never called).
pub async fn suggest_with_cache<F, Fut>(
    provider_id: &str,
    request: &TagRequest,
    fetcher: F,
) -> Result<TagResponse, LLMError>
where
    F: FnOnce(TagRequest) -> Fut + Send,
    Fut: std::future::Future<Output = Result<TagResponse, LLMError>> + Send,
{
    let mut hits: Vec<TagSuggestion> = Vec::new();
    let mut miss_assets: Vec<AssetInput> = Vec::new();
    let mut miss_keys: Vec<String> = Vec::new();

    for a in &request.assets {
        let thumb_hash = a
            .thumbnail_base64
            .as_ref()
            .map(|s| cache::hash_bytes(s.as_bytes()));
        let key = cache::cache_key(
            thumb_hash.as_deref(),
            &a.filename,
            &a.path,
            provider_id,
            &request.model,
            request.prompt_version,
        );
        if let Some(hit) = cache::get(&key) {
            hits.push(hit);
        } else {
            miss_assets.push(a.clone());
            miss_keys.push(key);
        }
    }

    if miss_assets.is_empty() {
        return Ok(TagResponse {
            suggestions: hits,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cached: true,
            },
        });
    }

    let miss_request = TagRequest {
        assets: miss_assets,
        prompt_version: request.prompt_version,
        model: request.model.clone(),
        include_thumbnails: request.include_thumbnails,
        // Carry the project context through to the misses-only call so
        // the LLM still sees framing + existing tags when it bills only
        // for the cache misses.
        project_ctx: request.project_ctx.clone(),
        existing_tags: request.existing_tags.clone(),
    };

    let fresh = fetcher(miss_request).await?;

    // Persist fresh suggestions. A cache save failure is non-fatal —
    // worst case we'll re-bill on the next call. We pair by index
    // because the LLM is instructed to preserve input order.
    for (s, k) in fresh.suggestions.iter().zip(miss_keys.iter()) {
        let _ = cache::save(k, s);
    }

    let mut all = hits;
    all.extend(fresh.suggestions);

    Ok(TagResponse {
        suggestions: all,
        usage: Usage {
            input_tokens: fresh.usage.input_tokens,
            output_tokens: fresh.usage.output_tokens,
            // Even partial-cache responses count as a real (paid) call —
            // the UI distinguishes "everything was cached" from "some hit,
            // some paid" via the input_tokens field, not this flag.
            cached: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_category_serializes_snake_case() {
        let json = serde_json::to_string(&TagCategory::Subject).unwrap();
        assert_eq!(json, "\"subject\"");
    }

    #[test]
    fn unknown_provider_id_routes_to_disabled_error() {
        let cfg = ProviderConfig {
            api_key: None,
            endpoint: None,
            model: "x".into(),
        };
        // `Box<dyn LLMProvider>` doesn't implement Debug, so we can't use
        // `unwrap_err()`; pattern-match the Err branch directly instead.
        match make_provider("not-a-provider", cfg) {
            Err(LLMError::ProviderDisabled(id)) => assert_eq!(id, "not-a-provider"),
            Err(e) => panic!("expected ProviderDisabled, got {e:?}"),
            Ok(_) => panic!("expected error for unknown provider id"),
        }
    }

    // ---- parse_suggestions: 3-tier fallback ----

    #[test]
    fn parser_tier1_clean_json() {
        let text = r#"{
            "suggestions": [
                {
                    "asset_path": "a/b.png",
                    "tags": [
                        { "label": "character", "category": "type", "confidence": 0.95 }
                    ]
                }
            ]
        }"#;
        let s = parse_suggestions(text).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].asset_path, "a/b.png");
        assert_eq!(s[0].tags.len(), 1);
        assert_eq!(s[0].tags[0].label, "character");
    }

    #[test]
    fn parser_tier2_json_markdown_fence() {
        let text = r#"Here you go:

```json
{
  "suggestions": [
    { "asset_path": "x.png", "tags": [] }
  ]
}
```

That's it!"#;
        let s = parse_suggestions(text).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].asset_path, "x.png");
    }

    #[test]
    fn parser_tier2_bare_fence_no_lang_tag() {
        // Some models emit ``` without a language hint.
        let text = "```\n{\"suggestions\":[{\"asset_path\":\"y.png\",\"tags\":[]}]}\n```";
        let s = parse_suggestions(text).unwrap();
        assert_eq!(s[0].asset_path, "y.png");
    }

    #[test]
    fn parser_tier3_total_failure_returns_parse_error_with_raw() {
        let text = "I'm sorry, I cannot tag these images.";
        match parse_suggestions(text) {
            Err(LLMError::ParseError(raw)) => assert_eq!(raw, text),
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn parser_handles_empty_suggestions_array() {
        // Valid JSON with zero suggestions — happens when the model
        // explicitly opts out of tagging assets it can't classify.
        let s = parse_suggestions(r#"{"suggestions":[]}"#).unwrap();
        assert!(s.is_empty());
    }

    // ---- suggest_with_cache ----

    #[tokio::test]
    async fn cache_short_circuit_when_no_assets() {
        // Edge case: empty request shouldn't call the fetcher.
        let req = TagRequest {
            assets: vec![],
            prompt_version: 1,
            model: "claude-sonnet-4-6".into(),
            include_thumbnails: false,
            project_ctx: None,
            existing_tags: Vec::new(),
        };
        let mut called = false;
        let response = suggest_with_cache("claude", &req, |_r| {
            called = true;
            async {
                Ok(TagResponse {
                    suggestions: vec![],
                    usage: Usage::default(),
                })
            }
        })
        .await
        .unwrap();
        assert!(!called, "fetcher should not be called when no assets");
        assert!(response.usage.cached);
    }
}
