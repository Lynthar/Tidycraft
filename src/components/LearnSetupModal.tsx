import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles, Loader2, X, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUiStore, type AiLearningResult } from "../stores/uiStore";
import { useSettingsStore, type AiProviderId } from "../stores/settingsStore";
import { useProjectStore } from "../stores/projectStore";

interface ProjectMeta {
  theme?: string;
  goal?: string;
}

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

/// AI Learning launch modal. Reads `[project]` from tidycraft.toml to
/// pre-fill theme/goal inputs (read-only — to persist them the user
/// edits tidycraft.toml directly via Settings → Analysis Rules → Edit;
/// inline edits would risk clobbering the user's own toml comments
/// since the toml crate doesn't preserve them on round-trip).
///
/// Continue invokes `learn_project_conventions`, on success swaps to
/// LearnReviewPanel via uiStore.
export function LearnSetupModal() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.learnSetupOpen);
  const setOpen = useUiStore((s) => s.setLearnSetupOpen);
  const setLearnReviewOpen = useUiStore((s) => s.setLearnReviewOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);

  const aiActiveProvider = useSettingsStore((s) => s.aiActiveProvider);
  const aiProviders = useSettingsStore((s) => s.aiProviders);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const [meta, setMeta] = useState<ProjectMeta>({});
  const [depth, setDepth] = useState(5);
  const [cost, setCost] = useState<CostEstimate | null>(null);
  const [loadingCost, setLoadingCost] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const provider = aiActiveProvider;
  const config = provider ? aiProviders[provider] : null;

  // Reset transient state on close, and pull project meta on open.
  useEffect(() => {
    if (!open) {
      setError(null);
      setRunning(false);
      return;
    }
    if (!activeProjectId) return;
    invoke<ProjectMeta>("read_project_meta", { projectId: activeProjectId })
      .then((m) => setMeta(m))
      .catch((e) => console.warn("[LearnSetup] read_project_meta failed:", e));
  }, [open, activeProjectId]);

  // Quick cost preview using `llm_estimate_cost` as an approximation —
  // the real learning prompt size depends on sample count (depth × dirs)
  // + tag count. For now we approximate as `depth × 10` "asset-equivalent
  // calls" which matches order-of-magnitude for typical projects.
  useEffect(() => {
    if (!open || !provider || !config) return;
    let cancelled = false;
    setLoadingCost(true);
    const approxAssetCount = Math.max(depth * 10, 20);
    invoke<CostEstimate>("llm_estimate_cost", {
      provider,
      model: config.model,
      assetCount: approxAssetCount,
      hasThumbnails: false, // learning mode is text-only
    })
      .then((c) => {
        if (!cancelled) setCost(c);
      })
      .catch((e) => {
        if (!cancelled) console.warn("[LearnSetup] estimate failed:", e);
      })
      .finally(() => {
        if (!cancelled) setLoadingCost(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, provider, config?.model, depth]);

  const handleContinue = async () => {
    if (!provider || !config || !activeProjectId || running) return;
    setRunning(true);
    setError(null);
    try {
      /// Persist theme/goal first so the learning call reads the same
      /// tidycraft.toml the user just edited. Always writes — even
      /// when meta hasn't changed — to keep "Continue = save + run"
      /// a single mental model. Cost is negligible (toml_edit on a
      /// small file). Failure here aborts the run rather than
      /// proceeding with stale config that would mislead the user
      /// about which framing the model saw.
      await invoke("write_project_meta", {
        projectId: activeProjectId,
        theme: meta.theme ?? "",
        goal: meta.goal ?? "",
      });

      const result = await invoke<AiLearningResult>("learn_project_conventions", {
        projectId: activeProjectId,
        provider,
        model: config.model,
        apiKey: config.apiKey ?? null,
        endpoint: config.endpoint ?? null,
        samplingDepth: depth,
      });
      setLearnReviewOpen(true, result);
      setOpen(false);
    } catch (err) {
      const msg = String(err);
      console.error("[LearnSetup] learn failed:", err);
      if (msg.includes("API key")) setError(t("aiAnalyze.errNoApiKey"));
      else if (msg.includes("Rate limit") || msg.includes("quota"))
        setError(t("aiAnalyze.errRateLimit"));
      else if (msg.includes("Network") || msg.includes("Could not reach") || msg.includes("timed out"))
        setError(t("aiAnalyze.errNetwork"));
      else if (msg.includes("hasn't been scanned"))
        setError(t("learnSetup.errNoScan"));
      else if (msg.includes("tidycraft.toml"))
        setError(t("learnSetup.errWrite"));
      else setError(t("aiAnalyze.errGeneric", { reason: msg }));
    } finally {
      setRunning(false);
    }
  };

  if (!open) return null;

  const dollarsString = cost ? (cost.usd_cents / 100).toFixed(2) : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div
        className="rounded-lg shadow-xl w-full max-w-md"
        style={{ background: "var(--panel)", border: "1px solid var(--line)" }}
      >
        <div
          className="flex items-center justify-between px-4 py-3"
          style={{ borderBottom: "1px solid var(--line)" }}
        >
          <div className="flex items-center gap-2">
            <Sparkles size={16} style={{ color: "var(--primary)" }} />
            <h2 className="text-sm font-semibold">{t("learnSetup.title")}</h2>
          </div>
          <button
            onClick={() => setOpen(false)}
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
                  setOpen(false);
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
              <div className="text-sm" style={{ color: "var(--text-2)" }}>
                {t("aiAnalyze.providerLine", {
                  name: t(PROVIDER_LABEL_KEYS[provider]),
                  model: config?.model ?? "",
                })}
              </div>

              <div>
                <label
                  className="text-xs block mb-1"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("learnSetup.theme")}
                </label>
                <input
                  type="text"
                  value={meta.theme ?? ""}
                  onChange={(e) =>
                    setMeta((prev) => ({ ...prev, theme: e.target.value }))
                  }
                  disabled={running}
                  placeholder={t("learnSetup.themeEmpty")}
                  className="w-full px-2 py-1 text-sm rounded font-mono disabled:opacity-50"
                  style={{
                    background: "var(--panel-2)",
                    border: "1px solid var(--line)",
                    color: "var(--text-2)",
                  }}
                />
              </div>
              <div>
                <label
                  className="text-xs block mb-1"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("learnSetup.goal")}
                </label>
                <input
                  type="text"
                  value={meta.goal ?? ""}
                  onChange={(e) =>
                    setMeta((prev) => ({ ...prev, goal: e.target.value }))
                  }
                  disabled={running}
                  placeholder={t("learnSetup.goalEmpty")}
                  className="w-full px-2 py-1 text-sm rounded font-mono disabled:opacity-50"
                  style={{
                    background: "var(--panel-2)",
                    border: "1px solid var(--line)",
                    color: "var(--text-2)",
                  }}
                />
              </div>
              <p
                className="text-xs"
                style={{ color: "var(--text-3)", fontStyle: "italic" }}
              >
                {t("learnSetup.editHint")}
              </p>

              <div>
                <label
                  className="text-xs block mb-1"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("learnSetup.depth", { value: depth })}
                </label>
                <input
                  type="range"
                  min={3}
                  max={30}
                  value={depth}
                  onChange={(e) => setDepth(parseInt(e.target.value, 10))}
                  className="w-full"
                />
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
                  <div
                    className="text-sm flex items-center gap-2"
                    style={{ color: "var(--text-3)" }}
                  >
                    <Loader2 size={12} className="animate-spin" />…
                  </div>
                ) : cost ? (
                  <div className="text-base font-medium">
                    {provider === "ollama"
                      ? t("aiAnalyze.continueLocal")
                      : `≈ $${dollarsString}`}
                  </div>
                ) : null}
                <p
                  className="text-xs mt-1"
                  style={{ color: "var(--text-3)" }}
                >
                  {t("learnSetup.costHint")}
                </p>
              </div>

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
                  <span>{t("learnSetup.running")}</span>
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
              onClick={() => setOpen(false)}
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
              disabled={running || !activeProjectId}
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
