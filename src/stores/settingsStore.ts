import { create } from "zustand";

// ============ AI Tagging types ============

/**
 * Stable provider id strings — must match the strings the Rust backend
 * routes through `make_provider`. Adding a new provider here requires
 * adding the Rust counterpart in `src-tauri/src/llm/`.
 */
export type AiProviderId = "claude" | "openai" | "ollama";

/**
 * Per-provider configuration. `apiKey` lives in plaintext localStorage —
 * a deliberate trade-off for a local single-user tool, disclosed in
 * README → "Privacy & Data"; the first save flashes a toast to remind
 * the user. Empty `apiKey` for cloud providers means "configured but
 * not credentialed" — `llm_suggest_tags` returns NoApiKey when called
 * against such a state.
 */
export interface AiProviderConfig {
  apiKey?: string;
  /**
   * Override URL. Always used for Ollama (default `http://localhost:11434`).
   * Optional for OpenAI (Azure / OpenRouter / corporate proxies). Unused
   * for Claude.
   */
  endpoint?: string;
  model: string;
}

/**
 * Defaults must match the `DEFAULT_MODEL` constants in
 * `src-tauri/src/llm/{claude,openai,ollama}.rs`. The backend command
 * `llm_default_models` exposes these so we COULD fetch them at boot,
 * but mirroring as constants here avoids the async startup race and
 * keeps the TS types self-documenting. If you bump a backend default,
 * mirror it here.
 */
const DEFAULT_AI_PROVIDERS: Record<AiProviderId, AiProviderConfig> = {
  claude: { model: "claude-sonnet-4-6" },
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
  /**
   * Which provider's `suggest_tags` is invoked when the user clicks
   * "AI Tag". `null` means AI tagging is disabled entirely (the
   * "Disabled" radio in the Settings panel).
   */
  aiActiveProvider: AiProviderId | null;
  /**
   * All providers' configs are kept side-by-side, so switching the
   * active provider doesn't lose the credentials of the previous one.
   */
  aiProviders: Record<AiProviderId, AiProviderConfig>;
  /**
   * Per-provider thumbnail-upload consent. The cost confirm modal
   * gates the first call until the corresponding flag flips to `true`.
   * Settings → "Reset consent" lets the user revoke per provider.
   */
  aiPrivacyConsented: Record<AiProviderId, boolean>;
  /**
   * Toggles the "AI Tag (directly)" entry points on AssetList multi-
   * select bar and right-click menu. Off by default — Learning mode
   * (sampling + rule generation) is recommended; per-asset vision
   * calls are ~50× more expensive and should be opt-in.
   */
  aiPerAssetModeEnabled: boolean;

  /**
   * When true (default), the scanner honors `.gitignore` / `.ignore`
   * files and skips hidden directories like `.git/` / `.vscode/`. Set
   * false to scan everything — useful for projects whose actual
   * assets live under gitignored paths (e.g. a vendored `Library/`).
   * Toggling this on a project triggers a full rescan on the next
   * `openProject` so the cache prunes the now-out-of-scope files.
   */
  respectGitignore: boolean;

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
  /**
   * Toggles the "AI Tag (directly)" entry points on AssetList multi-select
   * and the right-click menu. Off by default — Learning mode (sampling +
   * rule generation) is the recommended path; per-asset vision calls are
   * 50× more expensive and should be opt-in for users who actually need
   * thumbnail-level analysis.
   */
  aiPerAssetModeEnabled: boolean;
  /**
   * Per-machine setting. See `SettingsState.respectGitignore` for
   * full docs. Defaults to `true` for new installs; older shapes
   * (pre-feature) merge to the default cleanly.
   */
  respectGitignore: boolean;
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
};

/**
 * Deep-merge stored settings with current defaults. Older shapes
 * (pre-AI-tagging) lack `aiProviders` entirely; partial shapes (a user
 * who has used Claude but never configured OpenAI/Ollama) need each
 * missing provider filled from defaults so the UI never sees an
 * undefined config.
 */
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
  };
});
