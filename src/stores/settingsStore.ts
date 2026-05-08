import { create } from "zustand";

interface SettingsState {
  // Git display settings
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;

  // External editor mappings: extension (with leading dot, lowercase) →
  // absolute path of an editor binary / .app bundle / .desktop entry.
  // Empty map = no mappings configured.
  externalEditors: Record<string, string>;

  // Actions
  setShowGitStatusIndicators: (show: boolean) => void;
  setShowBranchInfo: (show: boolean) => void;
  setShowAheadBehind: (show: boolean) => void;
  setExternalEditor: (extension: string, editorPath: string) => void;
  removeExternalEditor: (extension: string) => void;
}

const STORAGE_KEY = "tidycraft-settings";

interface StoredSettings {
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;
  externalEditors: Record<string, string>;
}

const DEFAULT_SETTINGS: StoredSettings = {
  showGitStatusIndicators: true,
  showBranchInfo: true,
  showAheadBehind: true,
  externalEditors: {},
};

const getStoredSettings = (): StoredSettings => {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      // Merge with defaults so older shapes (no externalEditors) load
      // cleanly without losing previously persisted git toggles.
      return { ...DEFAULT_SETTINGS, ...JSON.parse(stored) };
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
  });

  return {
    showGitStatusIndicators: initial.showGitStatusIndicators,
    showBranchInfo: initial.showBranchInfo,
    showAheadBehind: initial.showAheadBehind,
    externalEditors: initial.externalEditors,

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
  };
});
