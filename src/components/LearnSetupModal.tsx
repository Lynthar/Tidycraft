import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles, Loader2, X, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalShell } from "./ModalShell";
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

/// AI Learning launch modal. Reads `[project]` from tidycraft.toml to pre-fill the
/// theme and goal inputs, then invokes `learn_project_conventions` and swaps to
/// LearnReviewPanel on success.
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
  const [costError, setCostError] = useState(false);
  const [estimateAttempt, setEstimateAttempt] = useState(0);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const provider = aiActiveProvider;
  const config = provider ? aiProviders[provider] : null;

  // Reset transient state on close, and pull project meta on open.
  useEffect(() => {
    if (!open) {
      setError(null);
      setCostError(false);
      setRunning(false);
      return;
    }
    if (!activeProjectId) return;
    invoke<ProjectMeta>("read_project_meta", { projectId: activeProjectId })
      .then((m) => setMeta(m))
      .catch((e) => console.warn("[LearnSetup] read_project_meta failed:", e));
  }, [open, activeProjectId]);

  // Cost preview via the dedicated learning estimator: the backend builds the SAME
  // prompt the run would send (same sampler, seed and builder) and prices that,
  // plus a bounded single-document output budget.
  useEffect(() => {
    if (!open || !provider || !config || !activeProjectId) return;
    let cancelled = false;
    setLoadingCost(true);
    // Cleared up front so a failure never leaves a stale estimate from the
    // previous parameters looking current.
    setCost(null);
    setCostError(false);
    invoke<CostEstimate>("estimate_learning_cost", {
      projectId: activeProjectId,
      provider,
      model: config.model,
      samplingDepth: depth,
    })
      .then((c) => {
        if (!cancelled) setCost(c);
      })
      .catch((e) => {
        if (cancelled) return;
        console.warn("[LearnSetup] estimate failed:", e);
        setCostError(true);
      })
      .finally(() => {
        if (!cancelled) setLoadingCost(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, provider, config?.model, depth, activeProjectId, estimateAttempt]);

  const handleContinue = async () => {
    if (!provider || !config || !activeProjectId || running) return;
    setRunning(true);
    setError(null);
    try {
      /// Persist theme and goal first so the learning call reads the same
      /// tidycraft.toml the user just edited. Always writes, keeping "Continue =
      /// save + run" one mental model. A failure here aborts the run.
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
      // If the user switched projects while the call was in flight, do NOT re-open
      // the review panel over the new project: its Save resolves the active project
      // at click time. The staged rules sit in the original project's pending set.
      if (useProjectStore.getState().activeProjectId !== activeProjectId) return;
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
      else if (msg.includes("truncated"))
        // Learning is one big reply, so the output cap is reachable on huge
        // projects — the lever here is a lower sampling depth.
        setError(t("learnSetup.errTruncated"));
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
    <ModalShell
      onClose={() => setOpen(false)}
      ariaLabel={t("learnSetup.title")}
      disabled={running}
    >
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
                  <div
                    className="text-base font-medium"
                    style={{ color: "var(--text)" }}
                  >
                    {provider === "ollama"
                      ? t("aiAnalyze.costFree")
                      : `≈ $${dollarsString}`}
                  </div>
                ) : costError && provider !== "ollama" ? (
                  <div className="text-sm flex items-center justify-between gap-2">
                    <span style={{ color: "var(--err)" }}>
                      {t("aiAnalyze.estimateFailed")}
                    </span>
                    <button
                      onClick={() => setEstimateAttempt((n) => n + 1)}
                      className="text-xs underline shrink-0"
                      style={{ color: "var(--text-2)" }}
                    >
                      {t("aiAnalyze.estimateRetry")}
                    </button>
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
            {/* Same fail-closed gate as AIAnalyzeModal: a cloud run needs a
                successful cost estimate first; Ollama is local and free. */}
            <button
              onClick={handleContinue}
              disabled={
                running ||
                !activeProjectId ||
                (provider !== "ollama" && cost === null)
              }
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
    </ModalShell>
  );
}
