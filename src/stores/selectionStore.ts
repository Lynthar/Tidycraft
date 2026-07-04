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

// Prune selected paths that vanished from the active project's scan — deleted
// through the app or externally (the watcher rebuilds `scanResult` on FS
// changes). Without this the batch toolbar keeps counting files that no longer
// exist and batch ops fail on them one by one. We prune against the FULL scan
// (not the filtered view) so a search / type filter never drops a still-valid
// selection. Runs after the activeProjectId-clear above on project switch, by
// which point the selection is already empty, so the two never conflict.
useProjectStore.subscribe((state, prev) => {
  if (state.scanResult === prev.scanResult) return;
  // A null scanResult is the forced-rescan in-flight window (openProject
  // clears the mirror while the scan runs), NOT "every file was deleted" —
  // pruning against it would wipe the whole selection on Ctrl+R. Wait for
  // the fresh result; this subscription fires again when it lands.
  if (state.scanResult === null) return;
  const sel = useSelectionStore.getState().selectedPaths;
  if (sel.size === 0) return;
  const present = new Set(state.scanResult.assets.map((a) => a.path));
  const stale = Array.from(sel).filter((p) => !present.has(p));
  if (stale.length > 0) useSelectionStore.getState().removePaths(stale);
});
