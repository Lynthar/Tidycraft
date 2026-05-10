//! Anthropic Claude provider — Messages API with vision content blocks.
//!
//! API reference (verified 2026-05-08):
//!   POST https://api.anthropic.com/v1/messages
//!   Headers: x-api-key, anthropic-version, content-type
//!   Body: { model, max_tokens, system, messages: [{ role, content: [...] }] }
//!   Image content block: { type: "image", source: { type: "base64", media_type, data } }
//!   Text content block:  { type: "text", text }
//!   Response.content is an array of typed blocks; we read the first
//!   `text` block as the model's reply.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    cost, learning, parse_json_lenient, project_meta::ProjectMeta, prompts, suggest_with_cache,
    AssetInput, CostEstimate, ExistingTagContext, LLMError, LLMProvider, ProviderConfig,
    TagRequest, TagResponse, Usage,
};

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_OUTPUT_TOKENS: u32 = 4096;
const REQUEST_TIMEOUT_SECS: u64 = 120;

pub struct ClaudeProvider {
    config: ProviderConfig,
}

impl ClaudeProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

// ---- Request body shape ----

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: Vec<ContentBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Image { source: ImageSource },
    Text { text: String },
}

#[derive(Serialize)]
struct ImageSource {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'static str,
    data: String,
}

// ---- Response body shape ----

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ResponseContent>,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct ResponseContent {
    /// Present for `type: "text"` blocks; absent for tool_use / image
    /// blocks the model might emit (we ignore those).
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: usize,
    output_tokens: usize,
}

// ---- Request building ----

fn build_content_blocks(
    assets: &[AssetInput],
    project_ctx: Option<&ProjectMeta>,
    existing_tags: &[ExistingTagContext],
    include_thumbnails: bool,
) -> Vec<ContentBlock> {
    // Anthropic recommends image-then-text. We emit every available
    // thumbnail in input order, then a single text block listing the
    // assets. The text-only case (no thumbnails) skips the loop and
    // sends one Text block.
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(assets.len() + 1);
    if include_thumbnails {
        for a in assets {
            if let Some(b64) = &a.thumbnail_base64 {
                blocks.push(ContentBlock::Image {
                    source: ImageSource {
                        kind: "base64",
                        media_type: "image/png",
                        data: b64.clone(),
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

/// Pull the first text block out of the response and parse its JSON
/// body into suggestions. Separated from the HTTP layer so unit tests
/// can feed in mock response shapes without spinning up a fake server.
fn extract_response(parsed: AnthropicResponse) -> Result<TagResponse, LLMError> {
    let text = parsed
        .content
        .into_iter()
        .find_map(|c| c.text)
        .ok_or_else(|| LLMError::ParseError("Anthropic response had no text block".into()))?;
    let suggestions = super::parse_suggestions(&text)?;
    Ok(TagResponse {
        suggestions,
        usage: Usage {
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
            cached: false,
        },
    })
}

// ---- HTTP call ----

async fn call_anthropic(
    api_key: &str,
    model: &str,
    request: TagRequest,
) -> Result<TagResponse, LLMError> {
    let body = AnthropicRequest {
        model,
        max_tokens: MAX_OUTPUT_TOKENS,
        system: prompts::SYSTEM_PROMPT,
        messages: vec![AnthropicMessage {
            role: "user",
            content: build_content_blocks(
                &request.assets,
                request.project_ctx.as_ref(),
                &request.existing_tags,
                request.include_thumbnails,
            ),
        }],
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| LLMError::Network(e.to_string()))?;

    let resp = client
        .post(ENDPOINT)
        .header("x-api-key", api_key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
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
        // Best-effort: capture the body for diagnostic value, but don't
        // surface raw provider errors to the user — map to our enum
        // categories so the UI can show a localized message.
        let body_preview = resp.text().await.unwrap_or_default();
        return Err(match status.as_u16() {
            401 | 403 => LLMError::NoApiKey("claude".into()),
            429 => LLMError::RateLimit,
            500..=599 => LLMError::Network(format!("Anthropic {status}: {body_preview}")),
            _ => LLMError::Other(format!("Anthropic {status}: {body_preview}")),
        });
    }

    let parsed: AnthropicResponse = resp
        .json()
        .await
        .map_err(|e| LLMError::ParseError(format!("Anthropic JSON: {e}")))?;
    extract_response(parsed)
}

#[async_trait]
impl LLMProvider for ClaudeProvider {
    fn id(&self) -> &str {
        "claude"
    }

    fn estimate_cost(&self, request: &TagRequest) -> CostEstimate {
        cost::estimate_cost(request)
    }

    async fn suggest_tags(&self, request: &TagRequest) -> Result<TagResponse, LLMError> {
        let api_key = self
            .config
            .api_key
            .clone()
            .ok_or_else(|| LLMError::NoApiKey("claude".into()))?;
        let model = self.config.model.clone();

        suggest_with_cache(self.id(), request, move |miss_request| async move {
            call_anthropic(&api_key, &model, miss_request).await
        })
        .await
    }

    async fn learn_project(
        &self,
        request: &learning::LearnRequest,
    ) -> Result<learning::LearningResult, LLMError> {
        let api_key = self
            .config
            .api_key
            .clone()
            .ok_or_else(|| LLMError::NoApiKey("claude".into()))?;
        let model = self.config.model.clone();
        let user = prompts::build_learning_prompt(
            &request.samples,
            request.project_meta.as_ref(),
            &request.existing_tags,
        );
        let (text, usage) =
            send_text_chat(&api_key, &model, prompts::SYSTEM_PROMPT_LEARNING, &user).await?;
        let mut result: learning::LearningResult = parse_json_lenient(&text)?;
        result.usage = usage;
        Ok(result)
    }
}

/// Text-only chat (no image content blocks). Used by `learn_project`,
/// where the LLM gets directory samples + tag context as plain text.
/// Mirrors `call_anthropic`'s scaffolding (client / headers / error
/// mapping) but with a simpler body.
async fn send_text_chat(
    api_key: &str,
    model: &str,
    system: &str,
    user: &str,
) -> Result<(String, Usage), LLMError> {
    let body = AnthropicRequest {
        model,
        max_tokens: MAX_OUTPUT_TOKENS,
        system,
        messages: vec![AnthropicMessage {
            role: "user",
            content: vec![ContentBlock::Text {
                text: user.to_string(),
            }],
        }],
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| LLMError::Network(e.to_string()))?;
    let resp = client
        .post(ENDPOINT)
        .header("x-api-key", api_key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
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
            401 | 403 => LLMError::NoApiKey("claude".into()),
            429 => LLMError::RateLimit,
            500..=599 => LLMError::Network(format!("Anthropic {status}: {body_preview}")),
            _ => LLMError::Other(format!("Anthropic {status}: {body_preview}")),
        });
    }
    let parsed: AnthropicResponse = resp
        .json()
        .await
        .map_err(|e| LLMError::ParseError(format!("Anthropic JSON: {e}")))?;
    let text = parsed
        .content
        .into_iter()
        .find_map(|c| c.text)
        .ok_or_else(|| LLMError::ParseError("Anthropic response had no text block".into()))?;
    Ok((
        text,
        Usage {
            input_tokens: parsed.usage.input_tokens,
            output_tokens: parsed.usage.output_tokens,
            cached: false,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::TagCategory;

    /// Helper: round-trip a JSON string through the response struct
    /// + extractor. Lets us simulate full API responses in unit tests
    /// without HTTP.
    fn parse_response_json(json: &str) -> Result<TagResponse, LLMError> {
        let parsed: AnthropicResponse =
            serde_json::from_str(json).map_err(|e| LLMError::ParseError(e.to_string()))?;
        extract_response(parsed)
    }

    #[test]
    fn parses_clean_response() {
        let json = r#"{
            "content": [
                {
                    "type": "text",
                    "text": "{\"suggestions\":[{\"asset_path\":\"hero.png\",\"tags\":[{\"label\":\"character\",\"category\":\"type\",\"confidence\":0.92}]}]}"
                }
            ],
            "usage": { "input_tokens": 1234, "output_tokens": 56 }
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.suggestions.len(), 1);
        assert_eq!(r.suggestions[0].asset_path, "hero.png");
        assert_eq!(r.suggestions[0].tags[0].label, "character");
        assert!(matches!(
            r.suggestions[0].tags[0].category,
            TagCategory::Type
        ));
        assert_eq!(r.usage.input_tokens, 1234);
        assert_eq!(r.usage.output_tokens, 56);
        assert!(!r.usage.cached);
    }

    #[test]
    fn parses_markdown_wrapped_response() {
        // Models sometimes wrap JSON in ```json fences despite the
        // prompt telling them not to. Tier-2 parser must catch this.
        let json = r#"{
            "content": [{
                "type": "text",
                "text": "```json\n{\"suggestions\":[{\"asset_path\":\"x.png\",\"tags\":[]}]}\n```"
            }],
            "usage": {"input_tokens":10,"output_tokens":5}
        }"#;
        let r = parse_response_json(json).unwrap();
        assert_eq!(r.suggestions.len(), 1);
        assert_eq!(r.suggestions[0].asset_path, "x.png");
    }

    #[test]
    fn rejects_response_with_no_text_block() {
        // A response with only non-text content blocks (image-out, tool_use)
        // should surface a ParseError rather than silently returning empty.
        let json = r#"{
            "content": [{"type":"image","text":null}],
            "usage": {"input_tokens":0,"output_tokens":0}
        }"#;
        match parse_response_json(json) {
            Err(LLMError::ParseError(msg)) => {
                assert!(msg.contains("no text block"), "got: {msg}");
            }
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn rejects_response_with_unparseable_text() {
        let json = r#"{
            "content": [{"type":"text","text":"I cannot help with that."}],
            "usage": {"input_tokens":10,"output_tokens":5}
        }"#;
        match parse_response_json(json) {
            Err(LLMError::ParseError(_)) => {}
            other => panic!("expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn build_content_blocks_text_only_emits_single_text_block() {
        let assets = vec![AssetInput {
            path: "a/x.fbx".into(),
            filename: "x.fbx".into(),
            thumbnail_base64: None,
            metadata_hint: None,
        }];
        let blocks = build_content_blocks(&assets, None, &[], false);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(&blocks[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn build_content_blocks_with_thumbnails_emits_image_then_text() {
        let assets = vec![
            AssetInput {
                path: "a/1.png".into(),
                filename: "1.png".into(),
                thumbnail_base64: Some("FAKE_B64_DATA".into()),
                metadata_hint: None,
            },
            AssetInput {
                path: "a/2.png".into(),
                filename: "2.png".into(),
                thumbnail_base64: Some("FAKE_B64_DATA_2".into()),
                metadata_hint: None,
            },
        ];
        let blocks = build_content_blocks(&assets, None, &[], true);
        // 2 images + 1 text
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], ContentBlock::Image { .. }));
        assert!(matches!(&blocks[1], ContentBlock::Image { .. }));
        assert!(matches!(&blocks[2], ContentBlock::Text { .. }));
    }
}
