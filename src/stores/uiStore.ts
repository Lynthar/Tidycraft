import { create } from "zustand";

/// Tracks transient UI state for app-level overlays (modals + command palette).
/// Lives in a global store rather than App.tsx local state so that any
/// component can trigger them without prop drilling — CommandPalette in
/// particular needs to open Settings / TagManager from inside an action.

interface UiState {
  cmdkOpen: boolean;
  settingsOpen: boolean;
  tagManagerOpen: boolean;
  setCmdkOpen: (open: boolean) => void;
  toggleCmdk: () => void;
  setSettingsOpen: (open: boolean) => void;
  setTagManagerOpen: (open: boolean) => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  cmdkOpen: false,
  settingsOpen: false,
  tagManagerOpen: false,
  setCmdkOpen: (open) => set({ cmdkOpen: open }),
  toggleCmdk: () => set({ cmdkOpen: !get().cmdkOpen }),
  setSettingsOpen: (open) => set({ settingsOpen: open }),
  setTagManagerOpen: (open) => set({ tagManagerOpen: open }),
}));
