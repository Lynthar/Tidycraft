import { useEffect, useState } from "react";
import { X, GitBranch, Palette, Wrench, Trash2, Image as ImageIcon, FileCode, ExternalLink, Sparkles, AlertTriangle, Filter } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useSettingsStore, type AiProviderId } from "../stores/settingsStore";
import { useThemeStore, type ThemePreference } from "../stores/themeStore";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

// Curated model dropdown lists. Keep first entry as the recommended
// default (same as backend `DEFAULT_MODEL` constants in
// src-tauri/src/llm/{claude,openai,ollama}.rs). Entries beyond the
// first are alternative options exposed to the user.
const MODEL_OPTIONS: Record<AiProviderId, string[]> = {
  openai: ["gpt-5.4-mini", "gpt-4o-mini", "gpt-5.4", "gpt-5.4-nano"],
  claude: ["claude-sonnet-4-6", "claude-haiku-4-5", "claude-opus-4-7"],
  ollama: [
    "qwen2.5vl:32b",
    "qwen2.5vl:7b",
    "qwen2.5vl:3b",
    "llama3.2-vision:11b",
    "llama3.2-vision:90b",
    "llava:13b",
  ],
};

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

function ToggleSwitch({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <label className="flex items-start gap-3 cursor-pointer group">
      <div className="pt-0.5">
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          onClick={() => onChange(!checked)}
          className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 ${
            checked ? "bg-primary" : "bg-gray-600"
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              checked ? "translate-x-4" : "translate-x-0"
            }`}
          />
        </button>
      </div>
      <div className="flex-1">
        <span className="text-sm font-medium text-text-primary group-hover:text-primary transition-colors">
          {label}
        </span>
        {description && (
          <p className="text-xs text-text-secondary mt-0.5">{description}</p>
        )}
      </div>
    </label>
  );
}

function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  label,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  label: string;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-sm font-medium text-text-primary">{label}</span>
      <div
        className="inline-flex p-0.5 rounded-md"
        style={{ background: "var(--panel-2)", border: "1px solid var(--line)" }}
      >
        {options.map((opt) => {
          const active = value === opt.value;
          return (
            <button
              key={opt.value}
              type="button"
              onClick={() => onChange(opt.value)}
              className="px-3 py-1 text-xs rounded transition-colors"
              style={{
                background: active ? "var(--panel)" : "transparent",
                color: active ? "var(--text)" : "var(--text-3)",
                fontWeight: active ? 500 : 400,
                border: 0,
                cursor: "pointer",
                boxShadow: active ? "0 1px 2px oklch(0% 0 0 / 0.05)" : "none",
              }}
            >
              {opt.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/// Editable list of (extension → editor path) mappings. Reads/writes
/// `settingsStore.externalEditors`; persisting is immediate (each row
/// edit calls `setExternalEditor` / `removeExternalEditor`). The "draft"
/// row holds the in-progress new mapping so the user can pick path via
/// `Browse…` before the mapping appears in the live list — keeps the
/// store from churning on incomplete keystrokes.
function ExternalEditorsSection() {
  const { t } = useTranslation();
  const externalEditors = useSettingsStore((s) => s.externalEditors);
  const setExternalEditor = useSettingsStore((s) => s.setExternalEditor);
  const removeExternalEditor = useSettingsStore((s) => s.removeExternalEditor);

  const [draft, setDraft] = useState<{ ext: string; path: string } | null>(null);

  // Single-file picker without filters — the user might point at any
  // launcher shape (.exe, .app bundle, shell script, .desktop entry).
  // Validation happens at launch time via tauri-plugin-opener.
  const pickEditorPath = async (): Promise<string | null> => {
    try {
      const selected = await open({ multiple: false });
      return typeof selected === "string" ? selected : null;
    } catch (err) {
      console.error("Failed to pick editor:", err);
      return null;
    }
  };

  const entries = Object.entries(externalEditors).sort(([a], [b]) =>
    a.localeCompare(b)
  );

  const draftReady =
    draft !== null &&
    draft.ext.trim().length > 0 &&
    draft.path.trim().length > 0;

  return (
    <div className="space-y-2 pl-6">
      {entries.length === 0 && !draft && (
        <p className="text-xs italic" style={{ color: "var(--text-3)" }}>
          {t("settings.noExternalEditors")}
        </p>
      )}

      {entries.map(([ext, editorPath]) => (
        <div key={ext} className="flex items-center gap-2">
          <code
            className="text-xs px-2 py-1 rounded font-mono shrink-0"
            style={{
              background: "var(--panel-2)",
              border: "1px solid var(--line)",
              minWidth: 60,
              textAlign: "center",
            }}
          >
            {ext}
          </code>
          <span
            className="text-xs flex-1 truncate"
            style={{ color: "var(--text-3)" }}
            title={editorPath}
          >
            {editorPath}
          </span>
          <button
            onClick={async () => {
              const newPath = await pickEditorPath();
              if (newPath) setExternalEditor(ext, newPath);
            }}
            className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors shrink-0"
          >
            {t("settings.editorBrowse")}
          </button>
          <button
            onClick={() => removeExternalEditor(ext)}
            className="p-1 rounded hover:bg-background transition-colors shrink-0"
            title={t("settings.editorRemove")}
            style={{ color: "var(--text-3)" }}
          >
            <Trash2 size={12} />
          </button>
        </div>
      ))}

      {draft && (
        <div
          className="flex items-center gap-2 p-2 rounded"
          style={{
            background: "var(--panel-2)",
            border: "1px dashed var(--line)",
          }}
        >
          <input
            type="text"
            value={draft.ext}
            onChange={(e) => setDraft({ ...draft, ext: e.target.value })}
            placeholder={t("settings.editorExtensionPlaceholder")}
            autoFocus
            className="text-xs px-2 py-1 rounded font-mono"
            style={{
              background: "var(--panel)",
              border: "1px solid var(--line)",
              color: "var(--text)",
              width: 70,
            }}
          />
          <span
            className="text-xs flex-1 truncate"
            style={{ color: draft.path ? "var(--text-3)" : "var(--text-4)" }}
            title={draft.path || ""}
          >
            {draft.path || t("settings.editorPathPlaceholder")}
          </span>
          <button
            onClick={async () => {
              const newPath = await pickEditorPath();
              if (newPath) setDraft({ ...draft, path: newPath });
            }}
            className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors shrink-0"
          >
            {t("settings.editorBrowse")}
          </button>
          <button
            onClick={() => {
              if (!draftReady || !draft) return;
              setExternalEditor(draft.ext, draft.path);
              setDraft(null);
            }}
            disabled={!draftReady}
            className="px-3 py-1 text-xs rounded transition-colors shrink-0"
            style={{
              background: draftReady ? "var(--primary)" : "var(--panel-2)",
              color: draftReady ? "var(--on-primary)" : "var(--text-4)",
              border: "1px solid var(--line)",
              cursor: draftReady ? "pointer" : "not-allowed",
            }}
          >
            ✓
          </button>
          <button
            onClick={() => setDraft(null)}
            className="p-1 rounded hover:bg-background text-text-secondary transition-colors shrink-0"
          >
            <X size={12} />
          </button>
        </div>
      )}

      {!draft && (
        <button
          onClick={() => setDraft({ ext: "", path: "" })}
          className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors inline-flex items-center gap-1"
        >
          + {t("settings.addEditor")}
        </button>
      )}
    </div>
  );
}

/// AI Tagging configuration. Provider is exposed as a "Disabled / OpenAI /
/// Claude / Ollama" segmented control. When a provider is active, the
/// per-provider config (model select, API key, endpoint, consent reset)
/// renders below. The provider's settings persist even when switched
/// inactive — so flipping back later doesn't ask the user to re-enter
/// their API key.
///
/// First-save warning: if the active provider's API key was empty before
/// this keystroke and is now non-empty, we flash an inline warning
/// reminding the user that the value lives in plaintext localStorage.
/// Auto-clears after 5s.
function AiTaggingSection() {
  const { t } = useTranslation();
  const aiActiveProvider = useSettingsStore((s) => s.aiActiveProvider);
  const aiProviders = useSettingsStore((s) => s.aiProviders);
  const aiPrivacyConsented = useSettingsStore((s) => s.aiPrivacyConsented);
  const setAiActiveProvider = useSettingsStore((s) => s.setAiActiveProvider);
  const setAiProviderConfig = useSettingsStore((s) => s.setAiProviderConfig);
  const resetAiPrivacyConsent = useSettingsStore((s) => s.resetAiPrivacyConsent);
  const aiPerAssetModeEnabled = useSettingsStore((s) => s.aiPerAssetModeEnabled);
  const setAiPerAssetModeEnabled = useSettingsStore(
    (s) => s.setAiPerAssetModeEnabled
  );

  // password-input toggle and key-stored warning
  const [showApiKey, setShowApiKey] = useState(false);
  const [keyWarning, setKeyWarning] = useState<string | null>(null);

  // Live Ollama model list — fetched from `/api/tags` on the user's
  // local daemon. Hardcoded MODEL_OPTIONS.ollama is a fallback only.
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaError, setOllamaError] = useState<string | null>(null);
  const [loadingOllamaModels, setLoadingOllamaModels] = useState(false);

  useEffect(() => {
    if (!keyWarning) return;
    const handle = setTimeout(() => setKeyWarning(null), 5000);
    return () => clearTimeout(handle);
  }, [keyWarning]);

  // SegmentedControl is generic over `T extends string`, so we
  // translate the null state to a sentinel "null" string for the UI
  // and convert back at the boundary.
  const providerOptions: { value: string; label: string }[] = [
    { value: "null", label: t("settings.aiProviderDisabled") },
    { value: "openai", label: t("settings.aiProviderOpenAI") },
    { value: "claude", label: t("settings.aiProviderClaude") },
    { value: "ollama", label: t("settings.aiProviderOllama") },
  ];

  const handleProviderChange = (v: string) => {
    setAiActiveProvider(v === "null" ? null : (v as AiProviderId));
  };

  const activeId = aiActiveProvider;
  const config = activeId ? aiProviders[activeId] : null;
  const ollamaEndpoint = aiProviders.ollama.endpoint || "http://localhost:11434";

  // Re-fetch installed Ollama models whenever the user activates Ollama
  // or changes its endpoint. The /api/tags call is local, fast, and
  // free; no need to debounce keystrokes — endpoint typing finishes
  // quickly and a stale request just gets superseded.
  useEffect(() => {
    if (activeId !== "ollama") return;
    let cancelled = false;
    setLoadingOllamaModels(true);
    setOllamaError(null);
    invoke<string[]>("llm_ollama_models", { endpoint: ollamaEndpoint })
      .then((list) => {
        if (cancelled) return;
        setOllamaModels(list);
      })
      .catch((err) => {
        if (cancelled) return;
        setOllamaError(String(err));
        setOllamaModels([]);
      })
      .finally(() => {
        if (!cancelled) setLoadingOllamaModels(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeId, ollamaEndpoint]);

  /** Compute the dropdown options for the active provider's model select.
   *  - Cloud providers: static curated list.
   *  - Ollama, daemon reachable: real `/api/tags` list, with the user's
   *    currently-selected model appended if it's not in the list (so a
   *    stale config doesn't lose the value silently).
   *  - Ollama, daemon unreachable: fall back to MODEL_OPTIONS.ollama so
   *    the user can still pick something — the call itself will fail
   *    later with a clear error if the daemon is genuinely down. */
  const modelOptionsFor = (id: AiProviderId, currentModel: string): string[] => {
    if (id !== "ollama") return MODEL_OPTIONS[id];
    if (ollamaError || ollamaModels.length === 0) {
      // Network failure path → curated suggestions. Empty-but-no-error
      // (user has Ollama running but installed nothing) goes through
      // here too; the empty list itself is handled by the hint UI.
      return ollamaError ? MODEL_OPTIONS.ollama : [];
    }
    if (currentModel && !ollamaModels.includes(currentModel)) {
      return [...ollamaModels, currentModel];
    }
    return ollamaModels;
  };

  const handleApiKeyChange = (v: string) => {
    if (!activeId) return;
    const wasEmpty = !config?.apiKey;
    setAiProviderConfig(activeId, { apiKey: v });
    if (wasEmpty && v.trim().length > 0) {
      setKeyWarning(t("settings.aiApiKeyWarning"));
    }
  };

  return (
    <div className="space-y-3 pl-6">
      <p className="text-xs" style={{ color: "var(--text-3)" }}>
        {t("settings.aiTaggingDesc")}
      </p>

      <SegmentedControl
        value={activeId ?? "null"}
        onChange={handleProviderChange}
        options={providerOptions}
        label={t("settings.aiProvider")}
      />

      {activeId && config && (
        <div
          className="space-y-3 mt-3 pl-3"
          style={{ borderLeft: "2px solid var(--line)" }}
        >
          {/* Model select */}
          <div>
            <label className="text-xs block mb-1" style={{ color: "var(--text-3)" }}>
              {t("settings.aiModel")}
            </label>
            {(() => {
              const options = modelOptionsFor(activeId, config.model);
              const isOllama = activeId === "ollama";
              const noModelsInstalled =
                isOllama && !loadingOllamaModels && !ollamaError && options.length === 0;
              return (
                <>
                  <select
                    value={config.model}
                    onChange={(e) =>
                      setAiProviderConfig(activeId, { model: e.target.value })
                    }
                    disabled={noModelsInstalled}
                    className="w-full px-2 py-1 text-sm rounded disabled:opacity-50"
                    style={{
                      background: "var(--panel)",
                      border: "1px solid var(--line)",
                      color: "var(--text)",
                    }}
                  >
                    {options.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                  {/* Live-list status hints — only shown for Ollama since
                      the cloud lists are fully static. */}
                  {isOllama && loadingOllamaModels && (
                    <p className="text-xs mt-1" style={{ color: "var(--text-3)" }}>
                      {t("settings.aiOllamaLoadingModels")}
                    </p>
                  )}
                  {isOllama && ollamaError && (
                    <p className="text-xs mt-1" style={{ color: "var(--err)" }}>
                      {t("settings.aiOllamaUnreachable")}
                    </p>
                  )}
                  {noModelsInstalled && (
                    <p
                      className="text-xs mt-1"
                      style={{ color: "var(--text-3)", fontStyle: "italic" }}
                    >
                      {t("settings.aiOllamaNoModels")}
                    </p>
                  )}
                </>
              );
            })()}
          </div>

          {/* API key — Ollama is local, no auth */}
          {activeId !== "ollama" && (
            <div>
              <label className="text-xs block mb-1" style={{ color: "var(--text-3)" }}>
                {t("settings.aiApiKey")}
              </label>
              <div className="flex gap-1">
                <input
                  type={showApiKey ? "text" : "password"}
                  value={config.apiKey ?? ""}
                  onChange={(e) => handleApiKeyChange(e.target.value)}
                  placeholder={t("settings.aiApiKeyPlaceholder")}
                  className="flex-1 px-2 py-1 text-sm rounded font-mono"
                  style={{
                    background: "var(--panel)",
                    border: "1px solid var(--line)",
                    color: "var(--text)",
                  }}
                />
                <button
                  onClick={() => setShowApiKey(!showApiKey)}
                  className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
                  style={{ minWidth: 56 }}
                >
                  {showApiKey
                    ? t("settings.aiApiKeyHide")
                    : t("settings.aiApiKeyShow")}
                </button>
              </div>
              {keyWarning && (
                <div
                  className="text-xs mt-2 px-2 py-1.5 rounded flex items-start gap-1.5"
                  style={{
                    color: "var(--warn, var(--err))",
                    background: "color-mix(in oklch, var(--err) 8%, transparent)",
                    border:
                      "1px solid color-mix(in oklch, var(--err) 22%, transparent)",
                  }}
                >
                  <AlertTriangle size={12} className="shrink-0 mt-0.5" />
                  <span>{keyWarning}</span>
                </div>
              )}
            </div>
          )}

          {/* Endpoint — required for Ollama, optional for OpenAI proxy/Azure */}
          {(activeId === "ollama" || activeId === "openai") && (
            <div>
              <label className="text-xs block mb-1" style={{ color: "var(--text-3)" }}>
                {t("settings.aiEndpoint")}
              </label>
              <input
                type="text"
                value={config.endpoint ?? ""}
                onChange={(e) =>
                  setAiProviderConfig(activeId, { endpoint: e.target.value })
                }
                placeholder={t("settings.aiEndpointPlaceholder")}
                className="w-full px-2 py-1 text-sm rounded font-mono"
                style={{
                  background: "var(--panel)",
                  border: "1px solid var(--line)",
                  color: "var(--text)",
                }}
              />
              {activeId === "openai" && (
                <p className="text-xs mt-1" style={{ color: "var(--text-3)" }}>
                  {t("settings.aiOpenaiEndpointHint")}
                </p>
              )}
            </div>
          )}

          {/* Privacy consent state + reset */}
          <div className="flex items-center justify-between gap-3">
            <span className="text-xs flex-1" style={{ color: "var(--text-3)" }}>
              {aiPrivacyConsented[activeId]
                ? t("settings.aiConsentGiven")
                : t("settings.aiConsentNotYet")}
            </span>
            <button
              onClick={() => resetAiPrivacyConsent(activeId)}
              disabled={!aiPrivacyConsented[activeId]}
              className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {t("settings.aiResetConsent")}
            </button>
          </div>

          {/* Per-asset (direct) AI Tag mode — opt-in advanced flow.
              Off by default because Learning mode is the recommended
              path; per-asset vision calls are dramatically more
              expensive (~50× per project). */}
          <div
            className="rounded p-2.5 mt-1"
            style={{
              background: "var(--panel-2)",
              border: "1px dashed var(--line)",
            }}
          >
            <ToggleSwitch
              checked={aiPerAssetModeEnabled}
              onChange={setAiPerAssetModeEnabled}
              label={t("settings.aiPerAssetMode")}
              description={t("settings.aiPerAssetModeDesc")}
            />
          </div>
        </div>
      )}
    </div>
  );
}

export function SettingsModal({ isOpen, onClose }: SettingsModalProps) {
  const { t, i18n } = useTranslation();
  const {
    showGitStatusIndicators,
    showBranchInfo,
    showAheadBehind,
    setShowGitStatusIndicators,
    setShowBranchInfo,
    setShowAheadBehind,
    respectGitignore,
    setRespectGitignore,
  } = useSettingsStore();
  const preference = useThemeStore((s) => s.preference);
  const setPreference = useThemeStore((s) => s.setPreference);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const hasCustomConfig = useProjectStore((s) => s.hasCustomConfig);
  const setHasCustomConfig = useProjectStore((s) => s.setHasCustomConfig);
  const undoHistory = useProjectStore((s) => s.undoHistory);
  const refreshUndoState = useProjectStore((s) => s.refreshUndoState);
  const clearUndoHistory = useProjectStore((s) => s.clearUndoHistory);

  const [thumbCacheBytes, setThumbCacheBytes] = useState<number | null>(null);
  const [llmCacheBytes, setLlmCacheBytes] = useState<number | null>(null);
  const [clearingCache, setClearingCache] = useState(false);
  const [clearingLlmCache, setClearingLlmCache] = useState(false);
  const [clearingUndo, setClearingUndo] = useState(false);
  const [editingRules, setEditingRules] = useState(false);
  const [rulesError, setRulesError] = useState<string | null>(null);

  // Pull both cache sizes whenever the modal opens. Each is a single
  // readdir+stat pass; running them in parallel avoids serializing two
  // small IPC calls behind each other.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    (async () => {
      const [thumb, llm] = await Promise.all([
        invoke<number>("get_thumbnail_cache_size").catch((err) => {
          console.error("Failed to read thumb cache size:", err);
          return null;
        }),
        invoke<number>("llm_cache_size").catch((err) => {
          console.error("Failed to read LLM cache size:", err);
          return null;
        }),
      ]);
      if (cancelled) return;
      setThumbCacheBytes(thumb);
      setLlmCacheBytes(llm);
    })();
    if (activeProjectId) refreshUndoState();
    return () => {
      cancelled = true;
    };
  }, [isOpen, activeProjectId, refreshUndoState]);

  const setLanguage = (lang: string) => {
    i18n.changeLanguage(lang);
    localStorage.setItem("language", lang);
  };

  const handleClearThumbCache = async () => {
    setClearingCache(true);
    try {
      await invoke<number>("clear_thumbnail_cache");
      setThumbCacheBytes(0);
    } catch (err) {
      console.error("Failed to clear thumbnail cache:", err);
    } finally {
      setClearingCache(false);
    }
  };

  const handleClearLlmCache = async () => {
    setClearingLlmCache(true);
    try {
      await invoke<number>("llm_clear_cache");
      setLlmCacheBytes(0);
    } catch (err) {
      console.error("Failed to clear LLM cache:", err);
    } finally {
      setClearingLlmCache(false);
    }
  };

  const handleClearUndoHistory = async () => {
    if (!activeProjectId) return;
    setClearingUndo(true);
    try {
      await clearUndoHistory();
    } catch (err) {
      console.error("Failed to clear undo history:", err);
    } finally {
      setClearingUndo(false);
    }
  };

  // Open the project's tidycraft.toml in the OS default editor. Backend
  // creates the file from a commented template if it doesn't exist yet,
  // so the user always has something to start from.
  const handleEditRules = async () => {
    if (!activeProjectId) return;
    setEditingRules(true);
    setRulesError(null);
    try {
      const path = await invoke<string>("ensure_project_config", {
        projectId: activeProjectId,
      });
      setHasCustomConfig(true);
      await invoke("open_with_default_app", { path });
    } catch (err) {
      console.error("Failed to open rules editor:", err);
      setRulesError(String(err));
    } finally {
      setEditingRules(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      {/* `max-h-[90vh]` + flex column lets the body scroll when content
          exceeds viewport height. Header + footer stay pinned. Without
          this, opening Settings on a non-fullscreen window (or after we
          add more sections) clips the bottom (Maintenance / Done button
          becomes unreachable). */}
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-full max-w-md flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border shrink-0">
          <h2 className="text-lg font-semibold">{t("settings.title")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content — scrolls when overflowing the viewport-bound modal. */}
        <div className="p-4 space-y-6 overflow-y-auto flex-1 min-h-0">
          {/* Appearance Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Palette size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.appearanceSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <SegmentedControl<ThemePreference>
                value={preference}
                onChange={setPreference}
                label={t("settings.theme")}
                options={[
                  { value: "dark", label: t("settings.themeDark") },
                  { value: "light", label: t("settings.themeLight") },
                  { value: "system", label: t("settings.themeSystem") },
                ]}
              />
              <SegmentedControl<string>
                value={i18n.language}
                onChange={setLanguage}
                label={t("settings.language")}
                options={[
                  { value: "en", label: t("settings.english") },
                  { value: "zh", label: t("settings.chinese") },
                ]}
              />
            </div>
          </div>

          {/* Git Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <GitBranch size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.gitSection")}
              </h3>
            </div>
            <div className="space-y-4 pl-6">
              <ToggleSwitch
                checked={showBranchInfo}
                onChange={setShowBranchInfo}
                label={t("settings.showBranchInfo")}
                description={t("settings.showBranchInfoDesc")}
              />
              <ToggleSwitch
                checked={showAheadBehind}
                onChange={setShowAheadBehind}
                label={t("settings.showAheadBehind")}
                description={t("settings.showAheadBehindDesc")}
              />
              <ToggleSwitch
                checked={showGitStatusIndicators}
                onChange={setShowGitStatusIndicators}
                label={t("settings.showGitStatusIndicators")}
                description={t("settings.showGitStatusIndicatorsDesc")}
              />
            </div>
          </div>

          {/* Analysis Rules Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <FileCode size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.analysisRulesSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-medium text-text-primary">
                    {t("settings.analysisRulesEdit")}
                  </span>
                  <p className="text-xs text-text-secondary mt-0.5">
                    {!activeProjectId
                      ? t("settings.analysisRulesNoProject")
                      : hasCustomConfig
                      ? t("settings.analysisRulesCustom")
                      : t("settings.analysisRulesDefault")}
                  </p>
                  {rulesError && (
                    <p className="text-xs mt-1" style={{ color: "var(--err)" }}>
                      {rulesError}
                    </p>
                  )}
                </div>
                <button
                  onClick={handleEditRules}
                  disabled={!activeProjectId || editingRules}
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors inline-flex items-center gap-1"
                >
                  <ExternalLink size={11} />
                  {editingRules ? t("settings.opening") : t("settings.analysisRulesEditButton")}
                </button>
              </div>
            </div>
          </div>

          {/* External Editors Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <ExternalLink size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.externalEditorsSection")}
              </h3>
            </div>
            <p
              className="text-xs pl-6 mb-3"
              style={{ color: "var(--text-3)" }}
            >
              {t("settings.externalEditorsHint")}
            </p>
            <ExternalEditorsSection />
          </div>

          {/* AI Tagging Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Sparkles size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.aiTaggingSection")}
              </h3>
            </div>
            <AiTaggingSection />
          </div>

          {/* Scanning Section — controls scanner behavior, not the
              user's per-project tidycraft.toml [ignore] patterns
              (those still come from the rules editor). */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Filter size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.scanningSection")}
              </h3>
            </div>
            <div className="pl-6">
              <ToggleSwitch
                checked={respectGitignore}
                onChange={setRespectGitignore}
                label={t("settings.respectGitignore")}
                description={t("settings.respectGitignoreDesc")}
              />
            </div>
          </div>

          {/* Maintenance Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Wrench size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.maintenanceSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-2 flex-1 min-w-0">
                  <ImageIcon
                    size={14}
                    className="text-text-secondary mt-0.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-text-primary">
                      {t("settings.thumbnailCache")}
                    </span>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {thumbCacheBytes === null
                        ? t("settings.cacheSizeUnknown")
                        : t("settings.cacheSize", {
                            size: formatFileSize(thumbCacheBytes),
                          })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClearThumbCache}
                  disabled={
                    clearingCache ||
                    thumbCacheBytes === null ||
                    thumbCacheBytes === 0
                  }
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {clearingCache
                    ? t("settings.clearing")
                    : t("settings.clear")}
                </button>
              </div>

              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-2 flex-1 min-w-0">
                  <Sparkles
                    size={14}
                    className="text-text-secondary mt-0.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-text-primary">
                      {t("settings.llmCache")}
                    </span>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {llmCacheBytes === null
                        ? t("settings.cacheSizeUnknown")
                        : t("settings.cacheSize", {
                            size: formatFileSize(llmCacheBytes),
                          })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClearLlmCache}
                  disabled={
                    clearingLlmCache ||
                    llmCacheBytes === null ||
                    llmCacheBytes === 0
                  }
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {clearingLlmCache
                    ? t("settings.clearing")
                    : t("settings.clear")}
                </button>
              </div>

              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-2 flex-1 min-w-0">
                  <Trash2
                    size={14}
                    className="text-text-secondary mt-0.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-text-primary">
                      {t("settings.undoHistory")}
                    </span>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {!activeProjectId
                        ? t("settings.undoNoProject")
                        : t("settings.undoEntries", {
                            count: undoHistory.length,
                          })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClearUndoHistory}
                  disabled={
                    clearingUndo ||
                    !activeProjectId ||
                    undoHistory.length === 0
                  }
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {clearingUndo
                    ? t("settings.clearing")
                    : t("settings.clear")}
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end px-4 py-3 border-t border-border shrink-0">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium bg-primary text-white rounded hover:bg-primary/90 transition-colors"
          >
            {t("common.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
