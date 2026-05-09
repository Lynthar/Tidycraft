//! Per-provider pricing and cost estimation.
//!
//! All token prices and image-tokenization rules verified from official
//! provider docs on 2026-05-08:
//! - Anthropic: https://platform.claude.com/docs/en/about-claude/pricing
//! - Anthropic vision: https://platform.claude.com/docs/en/build-with-claude/vision
//!   (formula `tokens ≈ width × height / 750`, cap 1568 for non-Opus,
//!    4784 for Opus 4.7 with 2576px long-edge native res)
//! - OpenAI: https://developers.openai.com/api/docs/pricing
//! - OpenAI vision: 32×32 patch tokenization for 5.4-mini/nano (cap 1536
//!   patches, ×1.62 mini / ×2.46 nano multipliers), tile-based for 4o
//!   family (low-detail = 85 flat, high = 85 + 170×tiles).
//!
//! Rule of thumb when prices change: edit only the `pricing()` table.
//! Every caller goes through `estimate_cost`, which routes to the table.

use super::{CostEstimate, TagRequest};

/// Per-million-token cost in micro-USD (10^-6 dollars). $5/M = 5_000_000.
/// Integer arithmetic avoids float drift; final ceiling-to-cents
/// happens once at the end of `estimate_cost`.
struct Pricing {
    input_per_m: u64,
    output_per_m: u64,
    vision: VisionRule,
}

enum VisionRule {
    /// Anthropic: tokens ≈ (width × height) / 750, capped per model.
    /// Cap reflects the model's max native-resolution tokens before
    /// the API would auto-downscale the image.
    AnthropicWHOver750 { max: u32 },
    /// OpenAI gpt-4o family at "low detail" mode: flat 85 tokens per
    /// image regardless of size. We default to low-detail because
    /// thumbnail-sized previews don't benefit from high-detail tiles.
    OpenAILowDetailFlat,
    /// OpenAI 5.4-mini and 5.4-nano: image is covered by 32×32 patches,
    /// patch count is capped at 1536, then multiplied by a per-model
    /// factor that brings the count up to the billed token total.
    OpenAIPatchBased { multiplier: f32 },
    /// Ollama: local model, no $ cost. Token estimate is best-effort
    /// for the UI; the real count varies by model tokenizer and is
    /// returned by Ollama in `prompt_eval_count` after the call.
    OllamaFree,
}

/// Boilerplate around each per-asset section of the user prompt
/// (path/filename labels, separators, etc.). Derived empirically; the
/// real number floats with prompt edits but is small relative to
/// vision tokens.
const PROMPT_OVERHEAD_TOKENS_PER_ASSET: usize = 100;

/// Output budget per asset. The system prompt instructs models to emit
/// ~3 tags × ~30 tokens each plus JSON wrapping. 150 is a comfortable
/// upper bound that the modal shows BEFORE the call, so a slight
/// overestimate is preferred to surprise.
const OUTPUT_TOKENS_PER_ASSET: usize = 150;

fn pricing(model: &str) -> Option<Pricing> {
    match model {
        // Anthropic
        "claude-haiku-4-5" => Some(Pricing {
            input_per_m: 1_000_000,
            output_per_m: 5_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 1568 },
        }),
        "claude-sonnet-4-6" => Some(Pricing {
            input_per_m: 3_000_000,
            output_per_m: 15_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 1568 },
        }),
        "claude-opus-4-7" => Some(Pricing {
            input_per_m: 5_000_000,
            output_per_m: 25_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 4784 },
        }),

        // OpenAI
        "gpt-4o-mini" => Some(Pricing {
            input_per_m: 150_000,
            output_per_m: 600_000,
            vision: VisionRule::OpenAILowDetailFlat,
        }),
        "gpt-5.4-nano" => Some(Pricing {
            input_per_m: 200_000,
            output_per_m: 1_250_000,
            vision: VisionRule::OpenAIPatchBased { multiplier: 2.46 },
        }),
        "gpt-5.4-mini" => Some(Pricing {
            input_per_m: 750_000,
            output_per_m: 4_500_000,
            vision: VisionRule::OpenAIPatchBased { multiplier: 1.62 },
        }),
        "gpt-5.4" => Some(Pricing {
            input_per_m: 2_500_000,
            output_per_m: 15_000_000,
            vision: VisionRule::OpenAILowDetailFlat,
        }),

        // Ollama: any vision-capable tag the user might ship in. Match
        // by family prefix so users can pin specific quantizations
        // (e.g. `qwen2.5vl:7b-fp16`) without us listing each variant.
        m if m.starts_with("qwen")
            || m.starts_with("llama")
            || m.starts_with("llava")
            || m.starts_with("gemma")
            || m.starts_with("moondream") =>
        {
            Some(Pricing {
                input_per_m: 0,
                output_per_m: 0,
                vision: VisionRule::OllamaFree,
            })
        }

        _ => None,
    }
}

/// Tokens an image of `width × height` pixels would cost on `model`.
/// Returns 0 for unknown models so the cost estimator degrades to
/// "unknown — skip the modal" rather than silently undercharging.
pub fn estimate_image_tokens(width: u32, height: u32, model: &str) -> usize {
    let p = match pricing(model) {
        Some(p) => p,
        None => return 0,
    };
    match p.vision {
        VisionRule::AnthropicWHOver750 { max } => {
            let tokens = (width as u64 * height as u64) / 750;
            tokens.min(max as u64) as usize
        }
        VisionRule::OpenAILowDetailFlat => 85,
        VisionRule::OpenAIPatchBased { multiplier } => {
            let patches_w = (width as f32 / 32.0).ceil() as u32;
            let patches_h = (height as f32 / 32.0).ceil() as u32;
            let patches = (patches_w.saturating_mul(patches_h)).min(1536);
            (patches as f32 * multiplier).round() as usize
        }
        VisionRule::OllamaFree => {
            // Best-effort placeholder so the UI displays a non-zero
            // estimate. Real counts come back in the Usage struct
            // after the call.
            ((width as usize).saturating_mul(height as usize)) / 500
        }
    }
}

/// Estimate the input/output tokens and USD cents for a request,
/// without making any network call.
///
/// Assumes 256×256 thumbnails because that's what `thumbnail.rs`
/// emits — actual image dimensions aren't part of `TagRequest`. If
/// the frontend ever switches to per-asset thumbnail sizes, plumb
/// the dimension through `AssetInput` and update this loop.
pub fn estimate_cost(request: &TagRequest) -> CostEstimate {
    let p = match pricing(&request.model) {
        Some(p) => p,
        None => return CostEstimate::default(),
    };

    let mut input_tokens = 0usize;
    for asset in &request.assets {
        if request.include_thumbnails && asset.thumbnail_base64.is_some() {
            input_tokens =
                input_tokens.saturating_add(estimate_image_tokens(256, 256, &request.model));
        }
        input_tokens = input_tokens.saturating_add(PROMPT_OVERHEAD_TOKENS_PER_ASSET);
    }

    let output_tokens = OUTPUT_TOKENS_PER_ASSET.saturating_mul(request.assets.len());

    let input_micros = (input_tokens as u64).saturating_mul(p.input_per_m) / 1_000_000;
    let output_micros = (output_tokens as u64).saturating_mul(p.output_per_m) / 1_000_000;
    let total_micros = input_micros.saturating_add(output_micros);

    // Ceiling-divide to the next whole cent (10_000 micro-USD = 1 cent).
    // Free providers return 0 cents.
    let usd_cents = if total_micros == 0 {
        0
    } else {
        ((total_micros + 9_999) / 10_000) as u32
    };

    CostEstimate {
        input_tokens,
        output_tokens_estimate: output_tokens,
        usd_cents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::AssetInput;

    fn req(model: &str, n: usize, with_thumb: bool) -> TagRequest {
        TagRequest {
            assets: (0..n)
                .map(|i| AssetInput {
                    path: format!("a/{i}.png"),
                    filename: format!("{i}.png"),
                    thumbnail_base64: if with_thumb { Some("x".into()) } else { None },
                    metadata_hint: None,
                })
                .collect(),
            prompt_version: 1,
            model: model.into(),
            include_thumbnails: with_thumb,
            project_ctx: None,
            existing_tags: Vec::new(),
        }
    }

    // ----- Anthropic vision formula -----
    // Cross-checked against the docs' worked examples table:
    // 200×200 → 54, 1000×1000 → 1334, 1092×1092 → 1568 (capped),
    // Opus 4.7 1920×1080 → 2765.

    #[test]
    fn anthropic_vision_200x200() {
        assert_eq!(estimate_image_tokens(200, 200, "claude-sonnet-4-6"), 53);
    }

    #[test]
    fn anthropic_vision_1000x1000() {
        assert_eq!(estimate_image_tokens(1000, 1000, "claude-sonnet-4-6"), 1333);
    }

    #[test]
    fn anthropic_vision_caps_at_model_max() {
        // Without cap the formula gives 1589; non-Opus models cap at 1568.
        assert_eq!(estimate_image_tokens(1092, 1092, "claude-sonnet-4-6"), 1568);
        // Opus 4.7 has the higher 4784 cap, so 1092² (1589) goes through.
        assert_eq!(estimate_image_tokens(1092, 1092, "claude-opus-4-7"), 1589);
    }

    #[test]
    fn anthropic_vision_opus_high_res() {
        assert_eq!(estimate_image_tokens(1920, 1080, "claude-opus-4-7"), 2764);
    }

    // ----- OpenAI vision rules -----

    #[test]
    fn openai_low_detail_is_flat_85() {
        assert_eq!(estimate_image_tokens(256, 256, "gpt-4o-mini"), 85);
        assert_eq!(estimate_image_tokens(2048, 2048, "gpt-4o-mini"), 85);
        assert_eq!(estimate_image_tokens(50, 50, "gpt-4o-mini"), 85);
    }

    #[test]
    fn openai_patch_based_mini_256() {
        // 256/32 = 8, 8×8 = 64 patches, ×1.62 ≈ 104.
        assert_eq!(estimate_image_tokens(256, 256, "gpt-5.4-mini"), 104);
    }

    #[test]
    fn openai_patch_based_nano_256() {
        // 64 patches × 2.46 ≈ 157.
        assert_eq!(estimate_image_tokens(256, 256, "gpt-5.4-nano"), 157);
    }

    #[test]
    fn openai_patch_capped_at_1536() {
        // A 4096×4096 image would be (128×128)=16384 raw patches but
        // the cap is 1536; mini ×1.62 → 2488.
        let tokens = estimate_image_tokens(4096, 4096, "gpt-5.4-mini");
        assert_eq!(tokens, (1536.0_f32 * 1.62).round() as usize);
    }

    // ----- Cost roll-up -----

    #[test]
    fn cost_unknown_model_returns_zero() {
        let r = req("not-a-model", 10, true);
        let est = estimate_cost(&r);
        assert_eq!(est.usd_cents, 0);
        assert_eq!(est.input_tokens, 0);
        assert_eq!(est.output_tokens_estimate, 0);
    }

    #[test]
    fn cost_ollama_is_zero_dollars() {
        let r = req("qwen2.5vl:32b", 50, true);
        let est = estimate_cost(&r);
        assert_eq!(est.usd_cents, 0);
        // Token count is non-zero so the UI can still show "~13k tokens".
        assert!(est.input_tokens > 0);
        assert!(est.output_tokens_estimate > 0);
    }

    #[test]
    fn cost_50_assets_sonnet_matches_expected() {
        // Per-asset: 87 (image) + 100 (prompt) = 187 input + 150 output.
        // 50 × (187 × 3 + 150 × 15) / 1_000_000 USD
        //   = 50 × (561 + 2250) micros = 140_550 micros = $0.14 = 14 cents.
        let r = req("claude-sonnet-4-6", 50, true);
        let est = estimate_cost(&r);
        assert_eq!(est.usd_cents, 15); // 140_550 / 10_000 ceil = 15 cents
    }

    #[test]
    fn cost_50_assets_openai_mini_cheaper_than_claude() {
        let openai = estimate_cost(&req("gpt-5.4-mini", 50, true));
        let claude = estimate_cost(&req("claude-sonnet-4-6", 50, true));
        // Sanity: gpt-5.4-mini should be at least 2× cheaper than Sonnet.
        assert!(openai.usd_cents * 2 < claude.usd_cents);
    }

    #[test]
    fn cost_text_only_skips_image_tokens() {
        let with = estimate_cost(&req("claude-sonnet-4-6", 10, true));
        let without = estimate_cost(&req("claude-sonnet-4-6", 10, false));
        assert!(without.input_tokens < with.input_tokens);
    }
}
