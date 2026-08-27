//! Per-provider pricing and cost estimation. Token prices and image-tokenization
//! rules come from the official provider docs. When prices change, edit only the
//! `pricing()` table — every caller goes through `estimate_cost`, which reads it.

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
    /// OpenAI gpt-4o family at "low detail": a flat count per image regardless of
    /// size. Flat per *size*, not per model — `gpt-4o` bills 85 while
    /// `gpt-4o-mini` bills 2833 for the identical image.
    OpenAILowDetailFlat { tokens: usize },
    /// OpenAI 5.4-mini and 5.4-nano: image is covered by 32×32 patches,
    /// patch count is capped at 1536, then multiplied by a per-model
    /// factor that brings the count up to the billed token total.
    OpenAIPatchBased { multiplier: f32 },
    /// Ollama: local model, no $ cost. Token estimate is best-effort
    /// for the UI; the real count varies by model tokenizer and is
    /// returned by Ollama in `prompt_eval_count` after the call.
    OllamaFree,
}

/// Boilerplate around each per-asset section of the user prompt. Derived
/// empirically; small relative to vision tokens.
const PROMPT_OVERHEAD_TOKENS_PER_ASSET: usize = 100;

/// Output budget per asset: roughly 3 tags × 30 tokens plus JSON wrapping. A
/// comfortable upper bound, since the modal shows it before the call.
const OUTPUT_TOKENS_PER_ASSET: usize = 150;

/// Rough chars→tokens divisor for the learning estimator. ~4 chars/token is the
/// standard heuristic for ASCII paths and English prose; path-heavy text
/// tokenizes denser, which errs the estimate high.
const CHARS_PER_TOKEN: usize = 4;

/// Learning output budget. One call returns ONE document: fixed sections plus
/// terms that scale with what the prompt echoes back — every user tag, and one
/// entry per sampled file.
const LEARNING_OUTPUT_BASE_TOKENS: usize = 600;
const LEARNING_OUTPUT_TOKENS_PER_TAG: usize = 30;
const LEARNING_OUTPUT_TOKENS_PER_SAMPLE: usize = 40;

fn pricing(model: &str) -> Option<Pricing> {
    match model {
        // Anthropic. Superseded models keep their entries so a user pinned to one
        // still gets a real estimate. The `max` in each vision rule is the
        // per-image token ceiling, tracking the model's max input resolution.
        "claude-haiku-4-5" => Some(Pricing {
            input_per_m: 1_000_000,
            output_per_m: 5_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 1568 },
        }),
        // Listed at the standard rate although Sonnet 5 also carries a limited
        // introductory price: an estimate that expires silently would start
        // under-charging, and the policy here is to err high.
        "claude-sonnet-5" => Some(Pricing {
            input_per_m: 3_000_000,
            output_per_m: 15_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 4784 },
        }),
        "claude-opus-5" => Some(Pricing {
            input_per_m: 5_000_000,
            output_per_m: 25_000_000,
            vision: VisionRule::AnthropicWHOver750 { max: 4784 },
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
        // 2833, not 85: see `OpenAILowDetailFlat`. The 33× token markup is
        // the counterpart of this model's 33×-lower per-token price.
        "gpt-4o-mini" => Some(Pricing {
            input_per_m: 150_000,
            output_per_m: 600_000,
            vision: VisionRule::OpenAILowDetailFlat { tokens: 2833 },
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
        // Patch-based like its mini/nano siblings. The base model's multiplier is
        // unpublished; 1.33 is a ceiling that prices a 256×256 thumbnail
        // (64 patches) at 85 tokens.
        "gpt-5.4" => Some(Pricing {
            input_per_m: 2_500_000,
            output_per_m: 15_000_000,
            vision: VisionRule::OpenAIPatchBased { multiplier: 1.33 },
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
        VisionRule::OpenAILowDetailFlat { tokens } => tokens,
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

/// Estimate the input/output tokens and USD cents for a request, with no network
/// call. Assumes 256×256 thumbnails, matching what `thumbnail.rs` emits. The
/// shared prompt context is billed once per chunk, since every chunk re-sends it.
pub fn estimate_cost(request: &TagRequest) -> CostEstimate {
    let p = match pricing(&request.model) {
        Some(p) => p,
        None => return CostEstimate::default(),
    };

    // Shared-per-chunk part: system prompt + project framing + existing-tag
    // context. At ~450+ tokens minimum it dominates the relative error for
    // small text-only batches.
    let mut shared_tokens = super::prompts::SYSTEM_PROMPT.len() / CHARS_PER_TOKEN;
    if let Some(meta) = &request.project_ctx {
        let chars =
            meta.theme.as_deref().map_or(0, str::len) + meta.goal.as_deref().map_or(0, str::len);
        shared_tokens = shared_tokens.saturating_add(chars / CHARS_PER_TOKEN);
    }
    for tag in &request.existing_tags {
        let chars = tag.name.len()
            + tag.description.as_deref().map_or(0, str::len)
            + tag.sample_paths.iter().map(String::len).sum::<usize>();
        shared_tokens = shared_tokens.saturating_add(chars / CHARS_PER_TOKEN + 4);
    }
    // A fully-cached re-run makes zero requests, but the estimate can't see
    // the cache — one chunk minimum keeps the empty-selection case sane.
    let chunks = request
        .assets
        .len()
        .div_ceil(super::MAX_ASSETS_PER_REQUEST)
        .max(1);
    let mut input_tokens = shared_tokens.saturating_mul(chunks);

    for asset in &request.assets {
        if request.include_thumbnails && asset.thumbnail_base64.is_some() {
            input_tokens =
                input_tokens.saturating_add(estimate_image_tokens(256, 256, &request.model));
        }
        input_tokens = input_tokens.saturating_add(PROMPT_OVERHEAD_TOKENS_PER_ASSET);
    }

    let output_tokens = OUTPUT_TOKENS_PER_ASSET.saturating_mul(request.assets.len());

    finish_estimate(&p, input_tokens, output_tokens)
}

/// Price a (input, output) token pair with `p` and ceiling-round to whole
/// cents (10_000 micro-USD = 1 cent). Free providers return 0 cents.
fn finish_estimate(p: &Pricing, input_tokens: usize, output_tokens: usize) -> CostEstimate {
    let input_micros = (input_tokens as u64).saturating_mul(p.input_per_m) / 1_000_000;
    let output_micros = (output_tokens as u64).saturating_mul(p.output_per_m) / 1_000_000;
    let total_micros = input_micros.saturating_add(output_micros);

    let usd_cents = if total_micros == 0 {
        0
    } else {
        total_micros.div_ceil(10_000) as u32
    };

    CostEstimate {
        input_tokens,
        output_tokens_estimate: output_tokens,
        usd_cents,
    }
}

/// Estimate a LEARNING run — not `estimate_cost` with a fake asset count, since
/// learning is one text-only call returning one document. Input is derived from
/// the byte length of the actual prompt the run would send.
pub fn estimate_learning_cost(
    model: &str,
    prompt_chars: usize,
    sample_count: usize,
    existing_tag_count: usize,
) -> CostEstimate {
    let p = match pricing(model) {
        Some(p) => p,
        None => return CostEstimate::default(),
    };

    let input_tokens = prompt_chars / CHARS_PER_TOKEN;
    let output_tokens = LEARNING_OUTPUT_BASE_TOKENS
        .saturating_add(LEARNING_OUTPUT_TOKENS_PER_TAG.saturating_mul(existing_tag_count))
        .saturating_add(LEARNING_OUTPUT_TOKENS_PER_SAMPLE.saturating_mul(sample_count));

    finish_estimate(&p, input_tokens, output_tokens)
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

    // Cross-checked against the docs' worked examples: 200×200 → 54,
    // 1000×1000 → 1334, 1092×1092 → 1568 (capped), Opus 4.7 1920×1080 → 2765.

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

    /// Flat means size-independent, not model-independent. `gpt-4o-mini` bills a
    /// low-detail image at 2833 tokens, not 85, scaled by the same ~33× that
    /// separates the two models' per-token prices.
    #[test]
    fn openai_low_detail_is_flat_per_size_but_priced_per_model() {
        for (w, h) in [(256, 256), (2048, 2048), (50, 50)] {
            assert_eq!(estimate_image_tokens(w, h, "gpt-4o-mini"), 2833);
        }
    }

    /// gpt-5.4 is patch-based like its siblings. Its multiplier is unpublished, so
    /// 1.33 is a ceiling that prices the app's one real input — a 256×256
    /// thumbnail, 64 patches — at 85 tokens.
    #[test]
    fn openai_patch_based_gpt54_256_matches_old_flat_rate() {
        assert_eq!(estimate_image_tokens(256, 256, "gpt-5.4"), 85);
        // And it now scales with size instead of staying flat.
        assert!(estimate_image_tokens(2048, 2048, "gpt-5.4") > 85);
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

    /// Every Claude model the settings dropdown offers must be priced here: an
    /// unpriced model estimates as zero, which the interface reads as "unknown" and
    /// drops the cost line. Mirrors `MODEL_OPTIONS.claude` in SettingsModal.tsx.
    #[test]
    fn every_offered_claude_model_is_priced() {
        for model in ["claude-sonnet-5", "claude-haiku-4-5", "claude-opus-5"] {
            let p = pricing(model).unwrap_or_else(|| panic!("{model} has no pricing entry"));
            assert!(
                p.input_per_m > 0 && p.output_per_m > 0,
                "{model} priced at zero"
            );
            assert!(
                estimate_image_tokens(512, 512, model) > 0,
                "{model} has no vision rule"
            );
        }
    }

    /// Superseded models keep their entries so a pinned configuration still
    /// gets a real estimate rather than the unknown-model degradation.
    #[test]
    fn previously_offered_claude_models_stay_priced() {
        for model in ["claude-sonnet-4-6", "claude-opus-4-7"] {
            assert!(pricing(model).is_some(), "{model} lost its pricing entry");
        }
    }

    /// The high-resolution tier accepts larger images, and its per-image
    /// token ceiling is correspondingly higher — pricing one of these at the
    /// older 1568 cap would under-estimate a full-resolution image threefold.
    #[test]
    fn high_resolution_models_cap_image_tokens_higher() {
        // 4096×4096 / 750 = 22369 raw, well past either cap.
        assert_eq!(estimate_image_tokens(4096, 4096, "claude-opus-5"), 4784);
        assert_eq!(estimate_image_tokens(4096, 4096, "claude-sonnet-5"), 4784);
        assert_eq!(estimate_image_tokens(4096, 4096, "claude-haiku-4-5"), 1568);
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
        // 50 × (187 × 3 + 150 × 15) micros = 140_550 ≈ 14 cents, plus the fixed
        // system-prompt input term — still 15 cents after ceiling.
        let r = req("claude-sonnet-4-6", 50, true);
        let est = estimate_cost(&r);
        assert_eq!(est.usd_cents, 15);
    }

    #[test]
    fn estimate_includes_the_system_prompt() {
        // A text-only single-asset request is dominated by the system prompt.
        let est = estimate_cost(&req("claude-sonnet-4-6", 1, false));
        let system_tokens = crate::llm::prompts::SYSTEM_PROMPT.len() / CHARS_PER_TOKEN;
        assert!(system_tokens > 300, "system prompt unexpectedly tiny");
        assert!(est.input_tokens >= system_tokens + PROMPT_OVERHEAD_TOKENS_PER_ASSET);
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

    /// The dispatcher re-sends system prompt, project framing and tag context with
    /// every ≤20-asset chunk, so the shared part must be billed per chunk. Billing
    /// it once under-charged exactly the runs where it matters.
    #[test]
    fn estimate_bills_shared_context_once_per_chunk() {
        let mut tagged = req("claude-sonnet-4-6", 40, false); // 2 chunks of 20
        tagged.existing_tags = vec![crate::llm::ExistingTagContext {
            name: "environment".into(),
            description: Some("outdoor scenery, terrain, foliage".into()),
            sample_paths: vec!["env/rock_01.png".into(), "env/tree_02.png".into()],
        }];

        let two_chunks = estimate_cost(&tagged).input_tokens;
        tagged.assets.truncate(20); // 1 chunk
        let one_chunk = estimate_cost(&tagged).input_tokens;

        // Doubling the chunk count adds one extra copy of the shared part
        // (system prompt + tag context), on top of the per-asset terms.
        let per_asset_part = 20 * PROMPT_OVERHEAD_TOKENS_PER_ASSET;
        let shared_part = one_chunk - per_asset_part;
        assert_eq!(two_chunks, one_chunk + per_asset_part + shared_part);
        assert!(shared_part > 300, "shared part unexpectedly tiny");
    }

    // ----- Learning estimator -----

    #[test]
    fn learning_unknown_model_returns_zero() {
        let est = estimate_learning_cost("not-a-model", 100_000, 500, 10);
        assert_eq!(est.usd_cents, 0);
        assert_eq!(est.input_tokens, 0);
    }

    #[test]
    fn learning_output_scales_with_samples_and_tags_not_dirs_times_150() {
        // 800 dirs × depth 10 = 8000 samples. The tagging estimator abused with that
        // as "asset count" would budget 1.2M output tokens (~$18 on Sonnet); the
        // learning call returns ONE document.
        let est = estimate_learning_cost("claude-sonnet-4-6", 400_000, 8000, 20);
        assert!(
            est.output_tokens_estimate < 8000 * OUTPUT_TOKENS_PER_ASSET / 3,
            "learning output budget ({}) must not look like per-asset tagging",
            est.output_tokens_estimate
        );
        // And it still scales in the right direction.
        let smaller = estimate_learning_cost("claude-sonnet-4-6", 40_000, 200, 5);
        assert!(smaller.output_tokens_estimate < est.output_tokens_estimate);
        assert!(smaller.input_tokens < est.input_tokens);
    }

    #[test]
    fn learning_typical_project_is_cents_not_dollars() {
        // ~120 dirs × depth 10 ≈ 1200 samples, roughly 60k chars (≈15k tokens). On
        // Sonnet this must land in cents territory, not the ~$18 the abused tagging
        // estimator displayed.
        let est = estimate_learning_cost("claude-sonnet-4-6", 60_000, 1200, 10);
        assert!(est.usd_cents >= 1);
        assert!(
            est.usd_cents < 200,
            "expected cents-range estimate, got {} cents",
            est.usd_cents
        );
    }

    #[test]
    fn learning_input_tracks_prompt_size() {
        let est = estimate_learning_cost("claude-sonnet-4-6", 80_000, 100, 0);
        assert_eq!(est.input_tokens, 80_000 / CHARS_PER_TOKEN);
    }
}
