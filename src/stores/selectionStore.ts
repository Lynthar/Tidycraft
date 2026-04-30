import { create } from "zustand";
import { useProjectStore } from "./projectStore";

/// Multi-selection state for the asset list / gallery, lifted out of
/// `AssetList.tsx` so other components (AITagPanel's Preview action,
/// CommandPalette future "select all matching" commands) can drive it
/// without prop drilling. Selection is *not* persisted across sessions
/// and is automatically cleared on active-project change so paths from a
/// previous project don't leak into the new one.

interface SelectionState {
  selectedPaths: Set<string>;
  setSelectedPaths: (paths: Iterable<string>) => void;
  togglePath: (path: string) => void;
  addPaths: (paths: Iterable<string>) => void;
  removePaths: (paths: Iterable<string>) => void;
  clearSelection: () => void;
}

export const useSelectionStore = create<SelectionState>((set, get) => ({
  selectedPaths: new Set(),
  setSelectedPaths: (paths) => set({ selectedPaths: new Set(paths) }),
  togglePath: (path) => {
    const next = new Set(get().selectedPaths);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    set({ selectedPaths: next });
  },
  addPaths: (paths) => {
    const next = new Set(get().selectedPaths);
    for (const p of paths) next.add(p);
    set({ selectedPaths: next });
  },
  removePaths: (paths) => {
    const next = new Set(get().selectedPaths);
    for (const p of paths) next.delete(p);
    set({ selectedPaths: next });
  },
  clearSelection: () => set({ selectedPaths: new Set() }),
}));

useProjectStore.subscribe((state, prev) => {
  if (state.activeProjectId !== prev.activeProjectId) {
    useSelectionStore.getState().clearSelection();
  }
});
