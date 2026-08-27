//! LLM-backed asset tagging. The frontend calls `llm_estimate_cost` for the
//! confirm modal, then `llm_suggest_tags`. Each provider (Claude / OpenAI /
//! Ollama) implements the same trait, so swapping one is a config change.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod cache;
pub mod claude;
pub mod cost;
pub mod learning;
pub mod ollama;
pub mod openai;
pub mod project_meta;
pub mod prompts;
pub mod rule_store;
pub mod sampler;

// ============ Data schemas ============
// Mirrors the TS-side shapes the frontend sends and expects via `invoke()`.
// Keep field names and snake_case in sync with `src/types/asset.ts`.

/// One LLM tagging call. The provider receives this and returns a
/// `TagResponse` covering every asset in the same order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRequest {
    pub assets: Vec<AssetInput>,
    /// Bumping `PROMPT_VERSION` invalidates every cache entry — see
    /// `prompts.rs`. Stored on the cache key so an old cached response
    /// from a stale prompt cannot be served.
    pub prompt_version: u32,
    /// Provider-specific model id (e.g. "claude-sonnet-5", "gpt-5.4-mini",
    /// "qwen2.5vl:32b"). Cached separately per model so users can
    /// upgrade their default without losing prior runs.
    pub model: String,
    /// When false, providers must skip image content and use filename +
    /// path only. Used for text-only fallback and for users who haven't
    /// consented to thumbnail upload.
    #[serde(default = "default_true")]
    pub include_thumbnails: bool,
    /// Optional project framing pulled from `tidycraft.toml [project]`. `None`
    /// (or both fields empty) makes the prompt builder skip the project-context
    /// block. Defaults to None for older request shapes.
    #[serde(default)]
    pub project_ctx: Option<project_meta::ProjectMeta>,
    /// User's existing tag system. The LLM is instructed to prefer
    /// these labels over inventing new ones. Empty vec → no
    /// existing-tag block emitted.
    #[serde(default)]
    pub existing_tags: Vec<ExistingTagContext>,
}

/// Per-tag context fed to the LLM so it can match existing project tags.
/// `description` is the user's TagManager blurb; `sample_paths` are up to 5 paths
/// where the tag is applied, letting the model infer intent from usage.
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
    /// Whether the model matched an existing project tag or invented a new label.
    /// The frontend uses it to skip the `(AI)` suffix. Defaults to `New` on
    /// deserialization so older cached responses still load.
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
    /// Anything outside the four buckets above. `serde(other)` makes this the
    /// catch-all, so one out-of-vocabulary category cannot fail the whole
    /// already-paid response.
    #[serde(other)]
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
    /// The model hit its output-token cap mid-reply, so the JSON is cut off.
    /// Distinct from ParseError because the actionable fix is a smaller request,
    /// and the input tokens were billed either way.
    #[error("Response truncated: the model hit its output-token limit before finishing; try a smaller request")]
    Truncated,
    #[error("Provider {0} not enabled in settings")]
    ProviderDisabled(String),
    /// Default trait fallback for an optional method a provider does not
    /// implement.
    #[error("Provider not implemented yet — Day 2 work")]
    NotImplemented,
    #[error("LLM error: {0}")]
    Other(String),
}

/// Map a cloud provider's non-2xx status to an error family. `provider` is the id
/// `NoApiKey` carries; `label` is what the user sees. Ollama keeps its own
/// (`ollama::map_http_status`) — a local server has no auth or metering.
pub(crate) fn map_cloud_http_status(
    provider: &str,
    label: &str,
    status: u16,
    body_preview: &str,
) -> LLMError {
    match status {
        401 | 403 => LLMError::NoApiKey(provider.to_string()),
        429 => LLMError::RateLimit,
        // 529 is Anthropic's `overloaded_error`, 503 the generic equivalent: the
        // provider is busy and waiting is the fix. Kept out of `Network`, which the
        // frontend renders as "check your connection".
        503 | 529 => LLMError::Other(format!(
            "{label} is temporarily overloaded ({status}) — wait a few seconds and try again"
        )),
        500..=599 => LLMError::Network(format!("{label} {status}: {body_preview}")),
        _ => LLMError::Other(format!("{label} {status}: {body_preview}")),
    }
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

    /// Make the actual API call to the provider's tagging endpoint.
    async fn suggest_tags(&self, request: &TagRequest) -> Result<TagResponse, LLMError>;

    /// Learning-mode call: sends the project samples, tag system and project meta,
    /// and parses a `LearningResult`. The default returns `NotImplemented` so a
    /// provider can be added without an immediate learn impl.
    async fn learn_project(
        &self,
        _request: &learning::LearnRequest,
    ) -> Result<learning::LearningResult, LLMError> {
        Err(LLMError::NotImplemented)
    }
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

/// Three-tier JSON parser: parse the text directly, retry after stripping a
/// markdown fence, else `LLMError::ParseError` carrying the original text so the
/// interface can show what the model actually said.
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

/// Generic 2-tier parser for LLM JSON output: parse directly, then retry after
/// stripping a markdown fence. On both failures returns
/// `LLMError::ParseError(original_text)`.
pub fn parse_json_lenient<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, LLMError> {
    if let Ok(v) = serde_json::from_str::<T>(text) {
        return Ok(v);
    }
    if let Some(stripped) = strip_markdown_fence(text) {
        if let Ok(v) = serde_json::from_str::<T>(stripped) {
            return Ok(v);
        }
    }
    Err(LLMError::ParseError(text.to_string()))
}

/// Pull the body out of a fenced code block, `None` when there is no fence. The
/// optional language tag must be alphanumeric; arbitrary text on the opening line
/// is treated as content, not a tag.
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

/// Pair fresh suggestions with the cache keys of the assets they answer, matching
/// by `asset_path` — NOT by position. The model may skip assets and is never told
/// to preserve order. Paths matching no request pair with nothing and are dropped.
fn pair_suggestions_with_keys<'a>(
    suggestions: &'a [TagSuggestion],
    miss_assets: &[AssetInput],
    miss_keys: &'a [String],
) -> Vec<(&'a TagSuggestion, &'a str)> {
    // This zip IS by index — but assets and keys were built together in
    // suggest_with_cache's miss loop, so their alignment is structural,
    // unlike the model's output order.
    let key_by_path: std::collections::HashMap<&str, &'a str> = miss_assets
        .iter()
        .zip(miss_keys)
        .map(|(a, k)| (a.path.as_str(), k.as_str()))
        .collect();
    suggestions
        .iter()
        .filter_map(|s| key_by_path.get(s.asset_path.as_str()).map(|&k| (s, k)))
        .collect()
}

/// Upper bound on assets per provider request. At 150 output tokens per asset the
/// 4096-token cap is met around 27 assets, and a truncated request loses the whole
/// batch while its input is still billed. Chunks run and cache sequentially.
const MAX_ASSETS_PER_REQUEST: usize = 20;

/// Wraps a provider's call with the per-asset cache: splits assets into hits and
/// misses, fetches each chunk of at most [`MAX_ASSETS_PER_REQUEST`] misses,
/// persists them, then merges. `fetcher` is never called on a full cache hit.
pub async fn suggest_with_cache<F, Fut>(
    provider_id: &str,
    request: &TagRequest,
    mut fetcher: F,
) -> Result<TagResponse, LLMError>
where
    F: FnMut(TagRequest) -> Fut + Send,
    Fut: std::future::Future<Output = Result<TagResponse, LLMError>> + Send,
{
    let mut hits: Vec<TagSuggestion> = Vec::new();
    let mut miss_assets: Vec<AssetInput> = Vec::new();
    let mut miss_keys: Vec<String> = Vec::new();

    // The project framing and existing-tag context is identical for every asset in
    // the batch, so hash it once and fold it into each key — editing a tag or
    // `[project]` then invalidates entries generated under the old context.
    let context_hash = cache::hash_context(request.project_ctx.as_ref(), &request.existing_tags);

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
            &context_hash,
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

    let mut fresh_suggestions: Vec<TagSuggestion> = Vec::new();
    let mut input_tokens = 0usize;
    let mut output_tokens = 0usize;

    for (chunk_assets, chunk_keys) in miss_assets
        .chunks(MAX_ASSETS_PER_REQUEST)
        .zip(miss_keys.chunks(MAX_ASSETS_PER_REQUEST))
    {
        let chunk_request = TagRequest {
            assets: chunk_assets.to_vec(),
            prompt_version: request.prompt_version,
            model: request.model.clone(),
            include_thumbnails: request.include_thumbnails,
            // Carry the project context into every chunk so the LLM still
            // sees framing + existing tags while billing only for misses.
            project_ctx: request.project_ctx.clone(),
            existing_tags: request.existing_tags.clone(),
        };

        let fresh = fetcher(chunk_request).await?;

        // Persist this chunk immediately, paired by asset_path. A save failure is
        // non-fatal but not invisible: a permanently unwritable cache directory
        // means every run pays the provider in full, so the error is logged.
        for (s, k) in pair_suggestions_with_keys(&fresh.suggestions, chunk_assets, chunk_keys) {
            if let Err(e) = cache::save(k, s) {
                eprintln!("[llm] failed to cache a suggestion (key {k}): {e}");
            }
        }

        input_tokens += fresh.usage.input_tokens;
        output_tokens += fresh.usage.output_tokens;
        fresh_suggestions.extend(fresh.suggestions);
    }

    let mut all = hits;
    all.extend(fresh_suggestions);

    Ok(TagResponse {
        suggestions: all,
        usage: Usage {
            input_tokens,
            output_tokens,
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

    /// 529 is Anthropic's documented `overloaded_error` and 503 the generic
    /// equivalent. Both landed in `Network`, which the frontend routes to "check
    /// your connection" — pointing the user at the one thing that is not wrong.
    #[test]
    fn provider_overload_is_not_reported_as_a_network_problem() {
        for status in [503, 529] {
            let err = map_cloud_http_status("claude", "Anthropic", status, "");
            let msg = err.to_string();
            assert!(
                matches!(err, LLMError::Other(_)),
                "{status} must not be a Network error, got {err:?}"
            );
            assert!(msg.contains("overloaded"), "{status}: says why: {msg}");
            // The frontend dispatches on substrings; these would misroute it.
            for misleading in [
                "Network",
                "Could not reach",
                "timed out",
                "Rate limit",
                "quota",
            ] {
                assert!(
                    !msg.contains(misleading),
                    "{status} message must not contain {misleading:?}: {msg}"
                );
            }
        }
        // A plain 500 is still a server-side failure with no better advice.
        assert!(matches!(
            map_cloud_http_status("claude", "Anthropic", 500, ""),
            LLMError::Network(_)
        ));
        // And the rest of the mapping is unchanged.
        assert!(matches!(
            map_cloud_http_status("openai", "OpenAI", 401, ""),
            LLMError::NoApiKey(_)
        ));
        assert!(matches!(
            map_cloud_http_status("openai", "OpenAI", 429, ""),
            LLMError::RateLimit
        ));
    }

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

    // ---- cache-save pairing (#6a) ----

    fn asset(path: &str) -> AssetInput {
        AssetInput {
            path: path.into(),
            filename: path.rsplit('/').next().unwrap_or(path).into(),
            thumbnail_base64: None,
            metadata_hint: None,
        }
    }

    fn suggestion(path: &str) -> TagSuggestion {
        TagSuggestion {
            asset_path: path.into(),
            tags: vec![],
        }
    }

    #[test]
    fn cache_pairing_matches_by_path_not_position() {
        // The model answered out of order AND skipped b.png — index-zipping
        // would cache c's suggestion under a's key and a's under b's key.
        let assets = vec![asset("a/a.png"), asset("a/b.png"), asset("a/c.png")];
        let keys = vec![
            "key-a".to_string(),
            "key-b".to_string(),
            "key-c".to_string(),
        ];
        let fresh = vec![suggestion("a/c.png"), suggestion("a/a.png")];

        let pairs = pair_suggestions_with_keys(&fresh, &assets, &keys);

        assert_eq!(pairs.len(), 2);
        assert!(pairs
            .iter()
            .any(|(s, k)| s.asset_path == "a/c.png" && *k == "key-c"));
        assert!(pairs
            .iter()
            .any(|(s, k)| s.asset_path == "a/a.png" && *k == "key-a"));
        // The skipped asset must get NO cache entry (so it stays a miss and
        // is re-asked next time), rather than inheriting a neighbour's answer.
        assert!(!pairs.iter().any(|(_, k)| *k == "key-b"));
    }

    #[test]
    fn cache_pairing_drops_hallucinated_paths() {
        // A suggestion for a path that was never requested pairs with
        // nothing — it must not steal a real asset's cache slot.
        let assets = vec![asset("a/a.png")];
        let keys = vec!["key-a".to_string()];
        let fresh = vec![suggestion("ghost/never-asked.png"), suggestion("a/a.png")];

        let pairs = pair_suggestions_with_keys(&fresh, &assets, &keys);

        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0.asset_path, "a/a.png");
        assert_eq!(pairs[0].1, "key-a");
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

    #[tokio::test]
    async fn misses_are_chunked_into_bounded_requests() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        // 45 misses must go out as ceil(45 / MAX_ASSETS_PER_REQUEST) requests. A
        // uuid model string keeps these cache keys disjoint from anything real;
        // the entries are removed again below.
        let model = format!("test-chunk-{}", uuid::Uuid::new_v4().simple());
        let assets: Vec<AssetInput> = (0..45).map(|i| asset(&format!("c/a{i}.png"))).collect();
        let req = TagRequest {
            assets: assets.clone(),
            prompt_version: 1,
            model: model.clone(),
            include_thumbnails: false,
            project_ctx: None,
            existing_tags: Vec::new(),
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let sizes = Arc::new(Mutex::new(Vec::<usize>::new()));
        let fetcher = {
            let calls = calls.clone();
            let sizes = sizes.clone();
            move |r: TagRequest| {
                calls.fetch_add(1, Ordering::SeqCst);
                sizes.lock().unwrap().push(r.assets.len());
                let suggestions: Vec<TagSuggestion> =
                    r.assets.iter().map(|a| suggestion(&a.path)).collect();
                async move {
                    Ok(TagResponse {
                        suggestions,
                        usage: Usage {
                            input_tokens: 10,
                            output_tokens: 5,
                            cached: false,
                        },
                    })
                }
            }
        };

        let response = suggest_with_cache("claude", &req, fetcher).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 3, "45 misses → 3 chunks");
        assert_eq!(*sizes.lock().unwrap(), vec![20, 20, 5]);
        assert_eq!(response.suggestions.len(), 45);
        // Usage sums across chunks.
        assert_eq!(response.usage.input_tokens, 30);
        assert_eq!(response.usage.output_tokens, 15);
        assert!(!response.usage.cached);

        // Clean the entries this test wrote to the real cache dir.
        let ctx = cache::hash_context(None, &[]);
        for a in &assets {
            cache::remove(&cache::cache_key(
                None,
                &a.filename,
                &a.path,
                "claude",
                &model,
                1,
                &ctx,
            ));
        }
    }
}
