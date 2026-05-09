//! OpenAI Chat Completions provider with vision content.
//!
//! API reference (verified 2026-05-08):
//!   POST https://api.openai.com/v1/chat/completions
//!   Auth: Authorization: Bearer <key>
//!   Body: { model, messages: [...], response_format: {type:"json_object"} }
//!   Image content block: { type: "image_url", image_url: { url: "data:image/png;base64,...", detail: "low" } }
//!   Response: choices[0].message.content + usage.{prompt_tokens,completion_tokens}
//!
//! `response_format: json_object` instructs the model to emit valid JSON
//! at the top level — saves the tier-2 markdown-fence parser most of
//! the time. Tier-2 still exists as defense in depth (some snapshots
//! still wrap output despite the directive).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    cost, project_meta::ProjectMeta, prompts, suggest_with_cache, AssetInput, CostEstimate,
    ExistingTagContext, LLMError, LLMProvider, ProviderConfig, TagRequest, TagResponse, Usage,
};

pub const DEFAULT_MODEL: &str = "gpt-5.4-mini";

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const REQUEST_TIMEOUT_SECS: u64 = 120;

pub struct OpenAIProvider {
    config: ProviderConfig,
}

impl OpenAIProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

// ---- Request body ----

#[derive(Serialize)]
struct OpenAIRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAIMessage>,
    response_format: ResponseFormat,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct OpenAIMessage {
    role: &'static str,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    /// System message: plain string.
    Text(String),
    /// User message: array of typed content blocks (so we can
    /// interleave images and text).
    Blocks(Vec<ContentBlock>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
    /// "low" / "high" / "auto". We use "low" because thumbnails are
    /// 256×256 and the high-detail tile budget would just inflate
    /// cost without quality gains at that resolution. Matches the
    /// 85-token rule baked into `cost.rs`.
    detail: &'static str,
}

// ---- Response body ----

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIChoiceMessage,
}

#[derive(Deserialize)]
struct OpenAIChoiceMessage {
    content: String,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

// ---- Request building ----

fn build_user_content_blocks(
    assets: &[AssetInput],
    project_ctx: Option<&ProjectMeta>,
    existing_tags: &[ExistingTagContext],
    include_thumbnails: bool,
) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(assets.len() + 1);
    if include_thumbnails {
        for a in assets {
            if let Some(b64) = &a.thumbnail_base64 {
                blocks.push(ContentBlock::ImageUrl {
                    image_url: ImageUrl {
                        // OpenAI requires the `data:<mime>;base64,<payload>`
                        // URI form for inline images. Bare base64 is rejected.
                        url: format!("data:image/png;base64,{b64}"),
                        detail: "low",
                    },
                });
            }
        }
    }
    blocks.push(ContentBlock::Text {
        text: prompts::build_user_prompt(assets, project_ctx, existing_tags, include_thumbnails),
    });
    blocks
}

// ---- Response → TagResponse ----

fn extract_response(parsed: OpenAIResponse) -> Result<TagResponse, LLMError> {
    let text = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| LLMError::ParseError("OpenAI response had no choices".into()))?;
    let suggestions = super::parse_suggestions(&text)?;
    Ok(TagResponse {
        suggestions,
        usage: Usage {
            input_tokens: parsed.usage.prompt_tokens,
            output_tokens: parsed.usage.completion_tokens,
            cached: false,
        },
    })
}

// ---- HTTP call ----

async fn call_openai(
    api_key: &str,
    model: &str,
    endpoint: &str,
    request: TagRequest,
) -> Result<TagResponse, LLMError> {
    let body = OpenAIRequest {
        model,
        messages: vec![
            OpenAIMessage {
                role: "system",
                content: MessageContent::Text(prompts::SYSTEM_PROMPT.to_string()),
            },
            OpenAIMessage {
                role: "user",
                content: MessageContent::Blocks(build_user_content_blocks(
                    &request.assets,
                    request.project_ctx.as_ref(),
                    &request.existing_tags,
                    request.include_thumbnails,
                )),
            },
        ],
        response_format: ResponseFormat {
            kind: "json_object",
        },
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| LLMError::Network(e.to_string()))?;

    let resp = client
        .post(endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                LLMError::Network("request timed out".into())
            } else {
                LLMError::Network(e.to_string())
            }
        })?;

    let status = resp.status();
    if !status.is_success() {
        let body_preview = resp.text().await.unwrap_or_default();
        return Err(match status.as_u16() {
            401 | 403 => LLMError::NoApiKey("openai".into()),
            429 => LLMError::RateLimit,
            500..=599 => LLMError::Network(format!("OpenAI {status}: {body_preview}")),
            _ => LLMError::Other(format!("OpenAI {status}: {body_preview}")),
        });
    }

    let parsed: OpenAIResponse = resp
        .json()
        .await
        .map_err(|e| LLMError::ParseError(format!("OpenAI JSON: {e}")))?;
    extract_response(parsed)
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn estimate_cost(&self, request: &TagRequest) -> CostEstimate {
        cost::estimate_cost(request)
    }

    async fn suggest_tags(&self, request: &TagRequest) -> Result<TagResponse, LLMError> {
        let api_key = self
            .config
            .api_key
            .clone()
            .ok_or_else(|| LLMError::NoApiKey("openai".into()))?;
        let model = self.config.model.clone();
        // Endpoint override supports OpenAI-compatible proxies (Azure
        // OpenAI, OpenRouter, etc.) without code changes.
        let endpoint = self
            .config
            .endpoint
            .clone()
            .unwrap_or_else(|| ENDPOINT.to_string());

        suggest_with_cache(self.id(), request, move |miss_request| async move {
            call_openai(&api_key, &model, &endpoint, miss_request).await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::TagCategory;

    fn parse_response_json(json: &str) -> Result<TagResponse, LLMError> {
        let parsed: OpenAIResponse =
            serde_json::from_str(json).map_err(|e| LLMError::ParseError(e.to_string()))?;
        extract_response(parsed)
    }

    #[test]
    fn parses_clean_response() {
        let json = r#"{
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "{\"suggestions\":[{\"asset_path\":\"hero.png\",\"tags\":[{\"label\":\"character\",\"category\":\"type\",\"confidence\":0.92}]}]}"
                }
            }],
            "usage": { "prompt_tokens": 200, "completion_tokens": 50 }
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.suggestions.len(), 1);
        assert_eq!(r.suggestions[0].asset_path, "hero.png");
        assert!(matches!(r.suggestions[0].tags[0].category, TagCategory::Type));
        assert_eq!(r.usage.input_tokens, 200);
        assert_eq!(r.usage.output_tokens, 50);
        assert!(!r.usage.cached);
    }

    #[test]
    fn rejects_empty_choices() {
        let json = r#"{"choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0}}"#;
        match parse_response_json(json) {
            Err(LLMError::ParseError(msg)) => assert!(msg.contains("no choices"), "got: {msg}"),
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn parses_markdown_wrapped_response() {
        // Even with response_format=json_object, some preview models
        // wrap their output. Tier-2 must still rescue it.
        let json = r#"{
            "choices":[{"message":{"role":"assistant","content":"```json\n{\"suggestions\":[{\"asset_path\":\"a.png\",\"tags\":[]}]}\n```"}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1}
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.suggestions.len(), 1);
    }

    #[test]
    fn build_user_content_blocks_text_only_skips_images() {
        let assets = vec![AssetInput {
            path: "a.fbx".into(),
            filename: "a.fbx".into(),
            thumbnail_base64: None,
            metadata_hint: None,
        }];
        let blocks = build_user_content_blocks(&assets, None, &[], false);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn image_url_uses_data_uri_prefix() {
        // The OpenAI vision API rejects bare base64 — it must be
        // wrapped in `data:<mime>;base64,<payload>`.
        let assets = vec![AssetInput {
            path: "a.png".into(),
            filename: "a.png".into(),
            thumbnail_base64: Some("RAW_BYTES".into()),
            metadata_hint: None,
        }];
        let blocks = build_user_content_blocks(&assets, None, &[], true);
        let url = match &blocks[0] {
            ContentBlock::ImageUrl { image_url } => image_url.url.clone(),
            _ => panic!("expected ImageUrl as first block"),
        };
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.contains("RAW_BYTES"));
    }
}
