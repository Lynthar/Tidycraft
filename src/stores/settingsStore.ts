import { create } from "zustand";

// ============ AI Tagging types ============

/** Stable provider id strings — must match what the Rust backend routes through
 *  `make_provider`. A new provider needs its counterpart in `src-tauri/src/llm/`. */
export type AiProviderId = "claude" | "openai" | "ollama";

/** Per-provider configuration. `apiKey` lives in plaintext localStorage, as the
 *  README's "Privacy & Data" section states. An empty `apiKey` for a cloud
 *  provider means "configured but not credentialed". */
export interface AiProviderConfig {
  apiKey?: string;
  /** Override URL. Always used for Ollama (default `http://localhost:11434`),
   *  optional for OpenAI (Azure / OpenRouter / proxies), unused for Claude. */
  endpoint?: string;
  model: string;
}

/** The model each provider starts on — the authoritative default, since the
 *  backend uses whatever the request carries. Keep in step with `MODEL_OPTIONS`
 *  in SettingsModal.tsx and the pricing table in `src-tauri/src/llm/cost.rs`. */
const DEFAULT_AI_PROVIDERS: Record<AiProviderId, AiProviderConfig> = {
  claude: { model: "claude-sonnet-5" },
  openai: { model: "gpt-5.4-mini" },
  ollama: { model: "qwen2.5vl:32b", endpoint: "http://localhost:11434" },
};

const DEFAULT_AI_PRIVACY_CONSENTED: Record<AiProviderId, boolean> = {
  claude: false,
  openai: false,
  ollama: false,
};

// ============ Store shape ============

interface SettingsState {
  // Git display settings
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;

  // External editor mappings: extension (with leading dot, lowercase) →
  // absolute path of an editor binary / .app bundle / .desktop entry.
  // Empty map = no mappings configured.
  externalEditors: Record<string, string>;

  // ----- AI tagging -----
  /** Which provider's `suggest_tags` runs on "AI Tag". `null` disables AI tagging
   *  entirely (the "Disabled" radio in the Settings panel). */
  aiActiveProvider: AiProviderId | null;
  /** All providers' configs are kept side by side, so switching the active
   *  provider does not lose the previous one's credentials. */
  aiProviders: Record<AiProviderId, AiProviderConfig>;
  /** Per-provider thumbnail-upload consent. The cost confirm modal gates the
   *  first call until the flag flips; Settings → "Reset consent" revokes it. */
  aiPrivacyConsented: Record<AiProviderId, boolean>;
  /** Toggles the "AI Tag (directly)" entry points. Off by default — Learning mode
   *  is recommended, and per-asset vision calls are ~50× more expensive. */
  aiPerAssetModeEnabled: boolean;

  /** When true (default), the scanner honors `.gitignore` / `.ignore` and skips
   *  hidden directories. Toggling it triggers a full rescan on the next
   *  `openProject` so the cache prunes now-out-of-scope files. */
  respectGitignore: boolean;

  /** Row caps for the HTML report's issue and asset tables; `0` is unlimited. The
   *  report is one self-contained file, so unbounded rows on a 100k-file project
   *  produce a very large document. */
  htmlReportIssueLimit: number;
  htmlReportAssetLimit: number;

  // ----- Actions -----
  setShowGitStatusIndicators: (show: boolean) => void;
  setShowBranchInfo: (show: boolean) => void;
  setShowAheadBehind: (show: boolean) => void;
  setExternalEditor: (extension: string, editorPath: string) => void;
  removeExternalEditor: (extension: string) => void;

  setAiActiveProvider: (id: AiProviderId | null) => void;
  /** Partial update — only the fields you pass are touched. */
  setAiProviderConfig: (id: AiProviderId, patch: Partial<AiProviderConfig>) => void;
  setAiPrivacyConsent: (id: AiProviderId, consented: boolean) => void;
  resetAiPrivacyConsent: (id: AiProviderId) => void;
  setAiPerAssetModeEnabled: (enabled: boolean) => void;
  setRespectGitignore: (respect: boolean) => void;
  setHtmlReportIssueLimit: (limit: number) => void;
  setHtmlReportAssetLimit: (limit: number) => void;
}

const STORAGE_KEY = "tidycraft-settings";

interface StoredSettings {
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;
  externalEditors: Record<string, string>;
  aiActiveProvider: AiProviderId | null;
  aiProviders: Record<AiProviderId, AiProviderConfig>;
  aiPrivacyConsented: Record<AiProviderId, boolean>;
  /** Toggles the "AI Tag (directly)" entry points on the multi-select bar and the
   *  right-click menu. Off by default — Learning mode is the recommended path,
   *  and per-asset vision calls cost about 50× more. */
  aiPerAssetModeEnabled: boolean;
  /** Per-machine setting; see `SettingsState.respectGitignore`. Defaults to
   *  `true`, and older stored shapes merge to that default cleanly. */
  respectGitignore: boolean;
  /** See `SettingsState` — HTML report row caps, 0 = unlimited. */
  htmlReportIssueLimit: number;
  htmlReportAssetLimit: number;
}

const DEFAULT_SETTINGS: StoredSettings = {
  showGitStatusIndicators: true,
  showBranchInfo: true,
  showAheadBehind: true,
  externalEditors: {},
  aiActiveProvider: null,
  aiProviders: DEFAULT_AI_PROVIDERS,
  aiPrivacyConsented: DEFAULT_AI_PRIVACY_CONSENTED,
  aiPerAssetModeEnabled: false,
  respectGitignore: true,
  // Historical backend defaults, kept as the out-of-box caps.
  htmlReportIssueLimit: 100,
  htmlReportAssetLimit: 500,
};

/** Deep-merge stored settings with current defaults. Older shapes lack
 *  `aiProviders` entirely, and partial ones need each missing provider filled
 *  from defaults so the interface never sees an undefined config. */
function mergeStored(parsed: Partial<StoredSettings>): StoredSettings {
  return {
    ...DEFAULT_SETTINGS,
    ...parsed,
    aiProviders: {
      claude: {
        ...DEFAULT_AI_PROVIDERS.claude,
        ...(parsed.aiProviders?.claude ?? {}),
      },
      openai: {
        ...DEFAULT_AI_PROVIDERS.openai,
        ...(parsed.aiProviders?.openai ?? {}),
      },
      ollama: {
        ...DEFAULT_AI_PROVIDERS.ollama,
        ...(parsed.aiProviders?.ollama ?? {}),
      },
    },
    aiPrivacyConsented: {
      ...DEFAULT_AI_PRIVACY_CONSENTED,
      ...(parsed.aiPrivacyConsented ?? {}),
    },
  };
}

const getStoredSettings = (): StoredSettings => {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return mergeStored(JSON.parse(stored));
    }
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
  return DEFAULT_SETTINGS;
};

const saveSettings = (settings: StoredSettings) => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
};

/// Normalize a user-supplied extension token: lowercase, leading dot,
/// stripped whitespace. `"PNG"` / `".PNG"` / `" png "` all collapse to
/// `".png"`. Empty input returns empty string — caller must reject.
const normalizeExtension = (raw: string): string => {
  const trimmed = raw.trim().toLowerCase();
  if (!trimmed) return "";
  return trimmed.startsWith(".") ? trimmed : `.${trimmed}`;
};

export const useSettingsStore = create<SettingsState>((set, get) => {
  const initial = getStoredSettings();

  // Snapshot of the persisted shape — every setter rebuilds a full
  // StoredSettings object from `get()` and writes it back, so adding a
  // field here means updating each setter to include it.
  const snapshot = (): StoredSettings => ({
    showGitStatusIndicators: get().showGitStatusIndicators,
    showBranchInfo: get().showBranchInfo,
    showAheadBehind: get().showAheadBehind,
    externalEditors: get().externalEditors,
    aiActiveProvider: get().aiActiveProvider,
    aiProviders: get().aiProviders,
    aiPrivacyConsented: get().aiPrivacyConsented,
    aiPerAssetModeEnabled: get().aiPerAssetModeEnabled,
    respectGitignore: get().respectGitignore,
    htmlReportIssueLimit: get().htmlReportIssueLimit,
    htmlReportAssetLimit: get().htmlReportAssetLimit,
  });

  return {
    showGitStatusIndicators: initial.showGitStatusIndicators,
    showBranchInfo: initial.showBranchInfo,
    showAheadBehind: initial.showAheadBehind,
    externalEditors: initial.externalEditors,
    aiActiveProvider: initial.aiActiveProvider,
    aiProviders: initial.aiProviders,
    aiPrivacyConsented: initial.aiPrivacyConsented,
    aiPerAssetModeEnabled: initial.aiPerAssetModeEnabled,
    respectGitignore: initial.respectGitignore,
    htmlReportIssueLimit: initial.htmlReportIssueLimit,
    htmlReportAssetLimit: initial.htmlReportAssetLimit,

    setShowGitStatusIndicators: (show: boolean) => {
      set({ showGitStatusIndicators: show });
      saveSettings(snapshot());
    },

    setShowBranchInfo: (show: boolean) => {
      set({ showBranchInfo: show });
      saveSettings(snapshot());
    },

    setShowAheadBehind: (show: boolean) => {
      set({ showAheadBehind: show });
      saveSettings(snapshot());
    },

    setExternalEditor: (extension: string, editorPath: string) => {
      const ext = normalizeExtension(extension);
      if (!ext || !editorPath.trim()) return;
      set({
        externalEditors: { ...get().externalEditors, [ext]: editorPath.trim() },
      });
      saveSettings(snapshot());
    },

    removeExternalEditor: (extension: string) => {
      const ext = normalizeExtension(extension);
      if (!ext) return;
      const next = { ...get().externalEditors };
      delete next[ext];
      set({ externalEditors: next });
      saveSettings(snapshot());
    },

    setAiActiveProvider: (id: AiProviderId | null) => {
      set({ aiActiveProvider: id });
      saveSettings(snapshot());
    },

    setAiProviderConfig: (id: AiProviderId, patch: Partial<AiProviderConfig>) => {
      const current = get().aiProviders[id];
      const merged: AiProviderConfig = { ...current, ...patch };
      set({ aiProviders: { ...get().aiProviders, [id]: merged } });
      saveSettings(snapshot());
    },

    setAiPrivacyConsent: (id: AiProviderId, consented: boolean) => {
      set({
        aiPrivacyConsented: { ...get().aiPrivacyConsented, [id]: consented },
      });
      saveSettings(snapshot());
    },

    resetAiPrivacyConsent: (id: AiProviderId) => {
      set({
        aiPrivacyConsented: { ...get().aiPrivacyConsented, [id]: false },
      });
      saveSettings(snapshot());
    },

    setAiPerAssetModeEnabled: (enabled: boolean) => {
      set({ aiPerAssetModeEnabled: enabled });
      saveSettings(snapshot());
    },

    setRespectGitignore: (respect: boolean) => {
      set({ respectGitignore: respect });
      saveSettings(snapshot());
    },

    setHtmlReportIssueLimit: (limit: number) => {
      // Non-finite input (NaN from a cleared or garbled field) means "no edit",
      // NOT zero: 0 means unlimited by contract with export_to_html, so
      // persisting it would arm an unbounded report. Negatives clamp to 0.
      if (!Number.isFinite(limit)) return;
      set({ htmlReportIssueLimit: Math.max(0, Math.floor(limit)) });
      saveSettings(snapshot());
    },

    setHtmlReportAssetLimit: (limit: number) => {
      if (!Number.isFinite(limit)) return;
      set({ htmlReportAssetLimit: Math.max(0, Math.floor(limit)) });
      saveSettings(snapshot());
    },
  };
});
