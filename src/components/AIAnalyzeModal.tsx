import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles, AlertTriangle, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUiStore, type AiTagResponse } from "../stores/uiStore";
import { useSettingsStore, type AiProviderId } from "../stores/settingsStore";
import { useProjectStore } from "../stores/projectStore";

/// Mirrors the backend `llm::CostEstimate` struct.
interface CostEstimate {
  input_tokens: number;
  output_tokens_estimate: number;
  usd_cents: number;
}

const PROVIDER_LABEL_KEYS: Record<AiProviderId, string> = {
  claude: "settings.aiProviderClaude",
  openai: "settings.aiProviderOpenAI",
  ollama: "settings.aiProviderOllama",
};

/// Cost-preview + consent modal that gates `llm_suggest_tags`. Triggered
/// from AssetList multi-select bar and ContextMenu via the global
/// `uiStore.aiAnalyzeOpen` flag (paths come along on the open call).
///
/// Flow:
///   1. Open with paths → call `llm_estimate_cost` to populate the
///      preview (pure function, no network).
///   2. User confirms (and ticks the consent box for cloud providers
///      on first call) → call `llm_suggest_tags`. Backend handles per-
///      asset cache lookup so a partial-cache run only bills the misses.
///   3. On success, swap to `AIResultPanel` via `setAiResultOpen`.
///   4. Errors are mapped to friendly i18n messages by string-matching
///      the LLMError variants the backend serializes.
export function AIAnalyzeModal() {
  const { t } = useTranslation();
  const aiAnalyzeOpen = useUiStore((s) => s.aiAnalyzeOpen);
  const paths = useUiStore((s) => s.aiAnalyzePaths);
  const setAiAnalyzeOpen = useUiStore((s) => s.setAiAnalyzeOpen);
  const setAiResultOpen = useUiStore((s) => s.setAiResultOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);

  const aiActiveProvider = useSettingsStore((s) => s.aiActiveProvider);
  const aiProviders = useSettingsStore((s) => s.aiProviders);
  const aiPrivacyConsented = useSettingsStore((s) => s.aiPrivacyConsented);
  const setAiPrivacyConsent = useSettingsStore((s) => s.setAiPrivacyConsent);

  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const [cost, setCost] = useState<CostEstimate | null>(null);
  const [loadingCost, setLoadingCost] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // `consentLocal` is the unticked-this-call state. Once Continue runs
  // we promote it to persisted `aiPrivacyConsented[provider]` so future
  // calls skip the checkbox altogether.
  const [consentLocal, setConsentLocal] = useState(false);
  // Whether to upload thumbnails as part of the prompt. Default OFF
  // because for game assets, filenames + paths usually carry the
  // semantic signal — normal/roughness/AO maps and 3D models don't
  // gain anything from a thumbnail (and 3D models have no thumbnail
  // anyway). Only flip on for diffuse/albedo textures or icon-style
  // flat assets where the visual matters. Vision tokens are 60-80%
  // of the total prompt cost, so OFF is also the cheap default.
  const [uploadThumbnails, setUploadThumbnails] = useState(false);

  const provider = aiActiveProvider;
  const config = provider ? aiProviders[provider] : null;
  const consented = provider ? aiPrivacyConsented[provider] : false;
  // Cloud providers (claude/openai) need explicit consent before
  // thumbnails leave the machine. Ollama is local — skip the gate.
  // Also skip when uploadThumbnails is off: text-only filenames don't
  // raise the same privacy concern (filenames + paths are still sent,
  // but the user agreed to that by configuring a cloud provider at all;
  // thumbnails are the meaningful "leaves the machine" delta).
  const needsConsentGate =
    provider !== null && provider !== "ollama" && uploadThumbnails;
  const canContinue =
    !!provider &&
    !!config &&
    !running &&
    !loadingCost &&
    (!needsConsentGate || consented || consentLocal);

  // Reset transient state when modal closes/reopens. Without this,
  // a previous error / cost number would briefly flash on the next open.
  useEffect(() => {
    if (!aiAnalyzeOpen) {
      setCost(null);
      setError(null);
      setConsentLocal(false);
      setRunning(false);
    }
  }, [aiAnalyzeOpen]);

  // Cost estimate fetch. Depends only on provider + model + asset count
  // + thumbnail toggle (cost.rs ignores actual paths). Using
  // `paths.length` instead of `paths` itself keeps re-renders cheap when
  // a different selection of the same size opens the modal.
  useEffect(() => {
    if (!aiAnalyzeOpen || !provider || !config || paths.length === 0) return;
    let cancelled = false;
    setLoadingCost(true);
    invoke<CostEstimate>("llm_estimate_cost", {
      provider,
      model: config.model,
      assetCount: paths.length,
      // Recompute when thumbnail toggle flips so the cost preview
      // reflects what Continue will actually cost.
      hasThumbnails: uploadThumbnails,
    })
      .then((c) => {
        if (cancelled) return;
        setCost(c);
      })
      .catch((err) => {
        if (cancelled) return;
        console.error("[AIAnalyzeModal] estimate failed:", err);
      })
      .finally(() => {
        if (!cancelled) setLoadingCost(false);
      });
    return () => {
      cancelled = true;
    };
  }, [aiAnalyzeOpen, provider, config?.model, paths.length, uploadThumbnails]);

  const handleContinue = async () => {
    if (!provider || !config || running) return;

    // Persist consent on first successful confirm (cloud only).
    if (needsConsentGate && !consented && consentLocal) {
      setAiPrivacyConsent(provider, true);
    }

    setRunning(true);
    setError(null);
    try {
      const response = await invoke<AiTagResponse>("llm_suggest_tags", {
        // Pass the active project's id so the backend can load this
        // project's tag system + tidycraft.toml [project] theme/goal as
        // prompt context (existing-tag reuse + project-aware suggestions).
        // Empty string makes the backend silently fall back to no context.
        projectId: activeProjectId ?? "",
        assetPaths: paths,
        provider,
        model: config.model,
        apiKey: config.apiKey ?? null,
        endpoint: config.endpoint ?? null,
        uploadThumbnails,
      });
      setAiResultOpen(true, response, paths);
      setAiAnalyzeOpen(false);
    } catch (err) {
      const msg = String(err);
      console.error("[AIAnalyzeModal] suggest failed:", err);
      // String-match the LLMError Display impls. Brittle but cheap;
      // serializing the error variant tag would require a wider backend
      // change. Keep the substring checks aligned with mod.rs.
      if (
        msg.includes("API key not configured") ||
        msg.includes("not implemented")
      ) {
        setError(
          msg.includes("not implemented")
            ? t("aiAnalyze.errGeneric", { reason: msg })
            : t("aiAnalyze.errNoApiKey")
        );
      } else if (msg.includes("Rate limit") || msg.includes("quota")) {
        setError(t("aiAnalyze.errRateLimit"));
      } else if (
        msg.includes("Network") ||
        msg.includes("Could not reach") ||
        msg.includes("timed out")
      ) {
        setError(t("aiAnalyze.errNetwork"));
      } else if (msg.includes("parse")) {
        setError(t("aiAnalyze.errParse"));
      } else {
        setError(t("aiAnalyze.errGeneric", { reason: msg }));
      }
    } finally {
      setRunning(false);
    }
  };

  if (!aiAnalyzeOpen) return null;

  const dollarsString = cost ? (cost.usd_cents / 100).toFixed(2) : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        className="rounded-lg shadow-xl w-full max-w-md"
        style={{
          background: "var(--panel)",
          border: "1px solid var(--line)",
        }}
      >
        <div
          className="flex items-center justify-between px-4 py-3"
          style={{ borderBottom: "1px solid var(--line)" }}
        >
          <div className="flex items-center gap-2">
            <Sparkles size={16} style={{ color: "var(--primary)" }} />
            <h2 className="text-sm font-semibold">{t("aiAnalyze.title")}</h2>
          </div>
          <button
            onClick={() => setAiAnalyzeOpen(false)}
            disabled={running}
            className="disabled:opacity-50"
            style={{ color: "var(--text-3)" }}
          >
            <X size={18} />
          </button>
        </div>

        <div className="p-4 space-y-3">
          {!provider ? (
            <>
              <p className="text-sm" style={{ color: "var(--text-2)" }}>
                {t("aiAnalyze.noProvider")}
              </p>
              <button
                onClick={() => {
                  setAiAnalyzeOpen(false);
                  setSettingsOpen(true);
                }}
                className="px-3 py-1.5 text-sm rounded"
                style={{
                  background: "var(--primary)",
                  color: "var(--on-primary, white)",
                }}
              >
                {t("settings.title")}
              </button>
            </>
          ) : (
            <>
              <div
                className="text-sm flex items-center justify-between"
                style={{ color: "var(--text-2)" }}
              >
                <span>
                  {t("aiAnalyze.providerLine", {
                    name: t(PROVIDER_LABEL_KEYS[provider]),
                    model: config?.model ?? "",
                  })}
                </span>
              </div>

              <div className="text-sm" style={{ color: "var(--text-2)" }}>
                {t("aiAnalyze.assetsCount", { count: paths.length })}
              </div>

              <div
                className="rounded p-3"
                style={{
                  background: "var(--panel-2)",
                  border: "1px solid var(--line)",
                }}
              >
                <div
                  className="text-xs uppercase tracking-wide mb-1"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("aiAnalyze.estimatedCost")}
                </div>
                {loadingCost ? (
                  <div className="text-sm flex items-center gap-2" style={{ color: "var(--text-3)" }}>
                    <Loader2 size={12} className="animate-spin" />
                    …
                  </div>
                ) : cost ? (
                  <div className="text-sm space-y-0.5">
                    <div style={{ color: "var(--text-3)" }}>
                      {t("aiAnalyze.tokensInput", { count: cost.input_tokens })}
                    </div>
                    <div style={{ color: "var(--text-3)" }}>
                      {t("aiAnalyze.tokensOutput", {
                        count: cost.output_tokens_estimate,
                      })}
                    </div>
                    <div className="text-base font-medium pt-1">
                      {provider === "ollama"
                        ? t("aiAnalyze.continueLocal")
                        : `≈ $${dollarsString}`}
                    </div>
                  </div>
                ) : null}
              </div>

              {/* Thumbnail upload toggle. Default off — see useState
                  comment for the rationale. The cost preview above
                  reactively recomputes when this flips. */}
              <label className="flex items-start gap-2 text-xs cursor-pointer">
                <input
                  type="checkbox"
                  checked={uploadThumbnails}
                  onChange={(e) => setUploadThumbnails(e.target.checked)}
                  className="mt-0.5"
                  disabled={running}
                />
                <span style={{ color: "var(--text-2)" }}>
                  {t("aiAnalyze.uploadThumbnails")}
                  <span
                    className="block mt-0.5"
                    style={{ color: "var(--text-3)", fontSize: 10 }}
                  >
                    {t("aiAnalyze.uploadThumbnailsHint")}
                  </span>
                </span>
              </label>

              {needsConsentGate && !consented && uploadThumbnails && (
                <label className="flex items-start gap-2 text-xs cursor-pointer">
                  <input
                    type="checkbox"
                    checked={consentLocal}
                    onChange={(e) => setConsentLocal(e.target.checked)}
                    className="mt-0.5"
                  />
                  <span style={{ color: "var(--text-2)" }}>
                    {t("aiAnalyze.consentLabel", {
                      provider: t(PROVIDER_LABEL_KEYS[provider]),
                    })}
                  </span>
                </label>
              )}

              {error && (
                <div
                  className="text-sm px-3 py-2 rounded flex items-start gap-2"
                  style={{
                    color: "var(--err)",
                    background: "color-mix(in oklch, var(--err) 8%, transparent)",
                    border:
                      "1px solid color-mix(in oklch, var(--err) 22%, transparent)",
                  }}
                >
                  <AlertTriangle size={14} className="shrink-0 mt-0.5" />
                  <span>{error}</span>
                </div>
              )}

              {running && (
                <div
                  className="flex items-center gap-2 text-sm"
                  style={{ color: "var(--text-2)" }}
                >
                  <Loader2 size={14} className="animate-spin" />
                  <span>{t("aiAnalyze.analyzing", { count: paths.length })}</span>
                </div>
              )}
            </>
          )}
        </div>

        {provider && (
          <div
            className="flex justify-end gap-2 px-4 py-3"
            style={{ borderTop: "1px solid var(--line)" }}
          >
            <button
              onClick={() => setAiAnalyzeOpen(false)}
              disabled={running}
              className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
              style={{
                border: "1px solid var(--line)",
                color: "var(--text-2)",
              }}
            >
              {t("aiAnalyze.cancel")}
            </button>
            <button
              onClick={handleContinue}
              disabled={!canContinue}
              className="px-3 py-1.5 text-sm rounded disabled:opacity-50"
              style={{
                background: "var(--primary)",
                color: "var(--on-primary, white)",
              }}
            >
              {running
                ? "…"
                : provider === "ollama"
                ? t("aiAnalyze.continueLocal")
                : dollarsString
                ? t("aiAnalyze.continueWithCost", { cost: dollarsString })
                : t("aiAnalyze.continue")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
