//! Ollama provider — local HTTP, no auth.
//!
//! API reference (verified 2026-05-08):
//!   POST {endpoint}/api/chat   (default endpoint: http://localhost:11434)
//!   Body: { model, messages: [{role, content, images?: [b64]}], stream:false, format:"json" }
//!   Response: { message: {role,content}, prompt_eval_count, eval_count, done, ... }
//!
//! Differences vs Anthropic / OpenAI:
//!   - `images` lives on the message, NOT inside the `content` array
//!     (Ollama's content is a plain string).
//!   - `format: "json"` instructs Ollama to emit valid JSON.
//!   - No 401 / 429 — local server. Connection failures surface as
//!     `Network` errors with a hint pointing at the endpoint.
//!   - Ollama has no $ cost; cost.rs already short-circuits to zero
//!     for Ollama-family model ids.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    cost, learning, parse_json_lenient, project_meta::ProjectMeta, prompts, suggest_with_cache,
    AssetInput, CostEstimate, ExistingTagContext, LLMError, LLMProvider, ProviderConfig,
    TagRequest, TagResponse, Usage,
};

pub const DEFAULT_MODEL: &str = "qwen2.5vl:32b";
pub const DEFAULT_ENDPOINT: &str = "http://localhost:11434";

/// Local-model inference is dramatically slower than cloud calls,
/// especially for 32B-class vision models on consumer GPUs. 5 minutes
/// gives even a slow batch a chance to finish before we abort.
const REQUEST_TIMEOUT_SECS: u64 = 300;

pub struct OllamaProvider {
    config: ProviderConfig,
}

impl OllamaProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

// ---- Request body ----

#[derive(Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage>,
    stream: bool,
    format: &'static str,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: &'static str,
    content: String,
    /// Image bytes are attached at message level (NOT inside content),
    /// as a list of bare base64 strings. Empty for system/text-only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
}

// ---- Response body ----

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaResponseMessage,
    /// Tokens spent reading the prompt (system + user). May be absent
    /// on errors or very-old Ollama versions; defaults to 0.
    #[serde(default)]
    prompt_eval_count: usize,
    /// Tokens emitted by the model. Same fallback rule.
    #[serde(default)]
    eval_count: usize,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

// ---- Request building ----

fn build_messages(
    assets: &[AssetInput],
    project_ctx: Option<&ProjectMeta>,
    existing_tags: &[ExistingTagContext],
    include_thumbnails: bool,
) -> Vec<OllamaMessage> {
    let mut images: Vec<String> = Vec::new();
    if include_thumbnails {
        for a in assets {
            if let Some(b64) = &a.thumbnail_base64 {
                images.push(b64.clone());
            }
        }
    }
    vec![
        OllamaMessage {
            role: "system",
            content: prompts::SYSTEM_PROMPT.to_string(),
            images: Vec::new(),
        },
        OllamaMessage {
            role: "user",
            content: prompts::build_user_prompt(
                assets,
                project_ctx,
                existing_tags,
                include_thumbnails,
            ),
            images,
        },
    ]
}

// ---- Response → TagResponse ----

fn extract_response(parsed: OllamaResponse) -> Result<TagResponse, LLMError> {
    let suggestions = super::parse_suggestions(&parsed.message.content)?;
    Ok(TagResponse {
        suggestions,
        usage: Usage {
            input_tokens: parsed.prompt_eval_count,
            output_tokens: parsed.eval_count,
            cached: false,
        },
    })
}

// ---- HTTP call ----

/// Map a non-2xx Ollama response to an error family. Local server: no
/// auth (401) or metering (429) to speak of — but 404 is the classic
/// "model not installed" and must NOT surface as a network problem: the
/// frontend routes any message mentioning Network / timed out to a
/// "check your connection" hint, which points the user in exactly the
/// wrong direction. 5xx stays Network (server-side breakage).
fn map_http_status(status: u16, url: &str, model: &str, body_preview: &str) -> LLMError {
    match status {
        404 => LLMError::Other(format!(
            "Ollama 404 at {url}: model \"{model}\" is not installed there \
             (run `ollama pull {model}`), or the endpoint path is wrong. \
             Server said: {body_preview}"
        )),
        500..=599 => LLMError::Network(format!("Ollama {status}: {body_preview}")),
        _ => LLMError::Other(format!("Ollama {status}: {body_preview}")),
    }
}

async fn call_ollama(
    endpoint: &str,
    model: &str,
    request: TagRequest,
) -> Result<TagResponse, LLMError> {
    // Endpoint comes in as base URL ("http://host:port"); we always
    // append the chat path. If the user already included `/api/chat`
    // we still trim and re-append to keep the joining unambiguous.
    let base = endpoint.trim_end_matches('/').trim_end_matches("/api/chat");
    let url = format!("{base}/api/chat");

    let body = OllamaRequest {
        model,
        messages: build_messages(
            &request.assets,
            request.project_ctx.as_ref(),
            &request.existing_tags,
            request.include_thumbnails,
        ),
        stream: false,
        format: "json",
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| LLMError::Network(e.to_string()))?;

    let resp = client.post(&url).json(&body).send().await.map_err(|e| {
        if e.is_timeout() {
            LLMError::Network(format!("Ollama request timed out after {REQUEST_TIMEOUT_SECS}s"))
        } else if e.is_connect() {
            // Most user-facing failure: Ollama isn't running. Surface
            // the endpoint we tried so the user can verify with
            // `ollama serve` or check their reverse proxy.
            LLMError::Network(format!("Could not reach Ollama at {url} ({e})"))
        } else {
            LLMError::Network(e.to_string())
        }
    })?;

    let status = resp.status();
    if !status.is_success() {
        let body_preview = resp.text().await.unwrap_or_default();
        return Err(map_http_status(status.as_u16(), &url, model, &body_preview));
    }

    let parsed: OllamaResponse = resp
        .json()
        .await
        .map_err(|e| LLMError::ParseError(format!("Ollama JSON: {e}")))?;
    extract_response(parsed)
}

#[async_trait]
impl LLMProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }

    fn estimate_cost(&self, request: &TagRequest) -> CostEstimate {
        cost::estimate_cost(request)
    }

    async fn suggest_tags(&self, request: &TagRequest) -> Result<TagResponse, LLMError> {
        let endpoint = self
            .config
            .endpoint
            .clone()
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        let model = self.config.model.clone();

        // FnMut: called once per chunk of misses, so clone per call instead
        // of moving into a one-shot async block.
        suggest_with_cache(self.id(), request, move |miss_request| {
            let endpoint = endpoint.clone();
            let model = model.clone();
            async move { call_ollama(&endpoint, &model, miss_request).await }
        })
        .await
    }

    async fn learn_project(
        &self,
        request: &learning::LearnRequest,
    ) -> Result<learning::LearningResult, LLMError> {
        let endpoint = self
            .config
            .endpoint
            .clone()
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        let model = self.config.model.clone();
        let user = prompts::build_learning_prompt(
            &request.samples,
            request.project_meta.as_ref(),
            &request.existing_tags,
        );
        let (text, usage) =
            send_text_chat(&endpoint, &model, prompts::SYSTEM_PROMPT_LEARNING, &user).await?;
        let mut result: learning::LearningResult = parse_json_lenient(&text)?;
        result.usage = usage;
        Ok(result)
    }
}

/// Text-only chat for learning mode. Same scaffolding as `call_ollama`
/// but without the `images` array on the user message.
async fn send_text_chat(
    endpoint: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<(String, Usage), LLMError> {
    let base = endpoint.trim_end_matches('/').trim_end_matches("/api/chat");
    let url = format!("{base}/api/chat");

    let body = OllamaRequest {
        model,
        messages: vec![
            OllamaMessage {
                role: "system",
                content: system.to_string(),
                images: Vec::new(),
            },
            OllamaMessage {
                role: "user",
                content: user.to_string(),
                images: Vec::new(),
            },
        ],
        stream: false,
        format: "json",
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| LLMError::Network(e.to_string()))?;
    let resp = client.post(&url).json(&body).send().await.map_err(|e| {
        if e.is_timeout() {
            LLMError::Network(format!("Ollama request timed out after {REQUEST_TIMEOUT_SECS}s"))
        } else if e.is_connect() {
            LLMError::Network(format!("Could not reach Ollama at {url} ({e})"))
        } else {
            LLMError::Network(e.to_string())
        }
    })?;
    let status = resp.status();
    if !status.is_success() {
        let body_preview = resp.text().await.unwrap_or_default();
        return Err(map_http_status(status.as_u16(), &url, model, &body_preview));
    }
    let parsed: OllamaResponse = resp
        .json()
        .await
        .map_err(|e| LLMError::ParseError(format!("Ollama JSON: {e}")))?;
    Ok((
        parsed.message.content,
        Usage {
            input_tokens: parsed.prompt_eval_count,
            output_tokens: parsed.eval_count,
            cached: false,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_response_json(json: &str) -> Result<TagResponse, LLMError> {
        let parsed: OllamaResponse =
            serde_json::from_str(json).map_err(|e| LLMError::ParseError(e.to_string()))?;
        extract_response(parsed)
    }

    #[test]
    fn parses_clean_response() {
        let json = r#"{
            "model": "qwen2.5vl:32b",
            "message": {
                "role": "assistant",
                "content": "{\"suggestions\":[{\"asset_path\":\"a.png\",\"tags\":[{\"label\":\"prop\",\"category\":\"type\",\"confidence\":0.85}]}]}"
            },
            "done": true,
            "prompt_eval_count": 320,
            "eval_count": 80
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.suggestions.len(), 1);
        assert_eq!(r.suggestions[0].asset_path, "a.png");
        assert_eq!(r.usage.input_tokens, 320);
        assert_eq!(r.usage.output_tokens, 80);
        assert!(!r.usage.cached);
    }

    #[test]
    fn missing_token_counts_default_to_zero() {
        // Older Ollama snapshots omit the *_eval_count fields entirely.
        // We fall back to 0 rather than failing the whole call.
        let json = r#"{
            "message": {
                "role": "assistant",
                "content": "{\"suggestions\":[]}"
            }
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.usage.input_tokens, 0);
        assert_eq!(r.usage.output_tokens, 0);
        assert_eq!(r.suggestions.len(), 0);
    }

    #[test]
    fn rejects_unparseable_content() {
        let json = r#"{
            "message": {"role":"assistant","content":"sorry, no tags"},
            "prompt_eval_count": 10,
            "eval_count": 5
        }"#;
        match parse_response_json(json) {
            Err(LLMError::ParseError(_)) => {}
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn build_messages_attaches_images_to_user_message_only() {
        let assets = vec![
            AssetInput {
                path: "x.png".into(),
                filename: "x.png".into(),
                thumbnail_base64: Some("AAA".into()),
                metadata_hint: None,
            },
            AssetInput {
                path: "y.png".into(),
                filename: "y.png".into(),
                thumbnail_base64: Some("BBB".into()),
                metadata_hint: None,
            },
        ];
        let msgs = build_messages(&assets, None, &[], true);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].images.is_empty(), "system msg must not carry images");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].images, vec!["AAA".to_string(), "BBB".to_string()]);
    }

    #[test]
    fn build_messages_skips_images_when_disabled() {
        let assets = vec![AssetInput {
            path: "x.png".into(),
            filename: "x.png".into(),
            thumbnail_base64: Some("AAA".into()),
            metadata_hint: None,
        }];
        let msgs = build_messages(&assets, None, &[], false);
        assert!(msgs[1].images.is_empty());
    }

    #[test]
    fn http_404_maps_to_actionable_error_not_network() {
        // 404 = model not installed (or wrong endpoint path). The frontend
        // routes anything whose message mentions Network / timed out to a
        // "check your connection" hint — exactly the wrong direction here.
        let e = map_http_status(
            404,
            "http://localhost:11434/api/chat",
            "qwen2.5vl:32b",
            r#"{"error":"model 'qwen2.5vl:32b' not found, try pulling it first"}"#,
        );
        match e {
            LLMError::Other(msg) => {
                assert!(msg.contains("qwen2.5vl:32b"), "names the model: {msg}");
                assert!(msg.contains("ollama pull"), "suggests the fix: {msg}");
                assert!(!msg.contains("Network"), "must not trip the network hint: {msg}");
            }
            other => panic!("expected LLMError::Other, got {other:?}"),
        }
    }

    #[test]
    fn http_5xx_still_maps_to_network() {
        match map_http_status(502, "http://host/api/chat", "m", "bad gateway") {
            LLMError::Network(msg) => assert!(msg.contains("502")),
            other => panic!("expected LLMError::Network, got {other:?}"),
        }
    }

    #[test]
    fn http_other_4xx_maps_to_generic_error_with_body() {
        match map_http_status(400, "http://host/api/chat", "m", "invalid request") {
            LLMError::Other(msg) => {
                assert!(msg.contains("400") && msg.contains("invalid request"));
            }
            other => panic!("expected LLMError::Other, got {other:?}"),
        }
    }
}
