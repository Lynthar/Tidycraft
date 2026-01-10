import { create } from "zustand";

interface SettingsState {
  // Git display settings
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;

  // Actions
  setShowGitStatusIndicators: (show: boolean) => void;
  setShowBranchInfo: (show: boolean) => void;
  setShowAheadBehind: (show: boolean) => void;
}

const STORAGE_KEY = "tidycraft-settings";

interface StoredSettings {
  showGitStatusIndicators: boolean;
  showBranchInfo: boolean;
  showAheadBehind: boolean;
}

const getStoredSettings = (): StoredSettings => {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return JSON.parse(stored);
    }
  } catch (e) {
    console.error("Failed to load settings:", e);
  }
  // Default values
  return {
    showGitStatusIndicators: true,
    showBranchInfo: true,
    showAheadBehind: true,
  };
};

const saveSettings = (settings: StoredSettings) => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
};

export const useSettingsStore = create<SettingsState>((set, get) => {
  const initial = getStoredSettings();

  return {
    showGitStatusIndicators: initial.showGitStatusIndicators,
    showBranchInfo: initial.showBranchInfo,
    showAheadBehind: initial.showAheadBehind,

    setShowGitStatusIndicators: (show: boolean) => {
      set({ showGitStatusIndicators: show });
      saveSettings({
        showGitStatusIndicators: show,
        showBranchInfo: get().showBranchInfo,
        showAheadBehind: get().showAheadBehind,
      });
    },

    setShowBranchInfo: (show: boolean) => {
      set({ showBranchInfo: show });
      saveSettings({
        showGitStatusIndicators: get().showGitStatusIndicators,
        showBranchInfo: show,
        showAheadBehind: get().showAheadBehind,
      });
    },

    setShowAheadBehind: (show: boolean) => {
      set({ showAheadBehind: show });
      saveSettings({
        showGitStatusIndicators: get().showGitStatusIndicators,
        showBranchInfo: get().showBranchInfo,
        showAheadBehind: show,
      });
    },
  };
});
