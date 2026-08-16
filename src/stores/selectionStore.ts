import { create } from "zustand";
import {
  useProjectStore,
  registerSelectionSyncBridge,
  renamedTargetFor,
} from "./projectStore";

/// Multi-selection state for the asset list and gallery, lifted out of
/// `AssetList.tsx` so other components can drive it without prop drilling. Not
/// persisted, and cleared on active-project change.

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

// Follow externally renamed files/folders: applyFsChange calls this BEFORE
// swapping scanResult, so by the time the prune subscription below runs, the
// selection already holds the new paths and nothing is dropped as stale.
registerSelectionSyncBridge({
  applyRenames: (renamed) => {
    const sel = useSelectionStore.getState().selectedPaths;
    if (sel.size === 0) return;
    let changed = false;
    const next = new Set<string>();
    for (const p of sel) {
      const to = renamedTargetFor(p, renamed);
      if (to) changed = true;
      next.add(to ?? p);
    }
    if (changed) useSelectionStore.getState().setSelectedPaths(next);
  },
});

// Prune selected paths that vanished from the active project's scan, so the batch
// toolbar stops counting files that no longer exist. Pruned against the FULL
// scan, so a search or type filter never drops a still-valid selection.
useProjectStore.subscribe((state, prev) => {
  if (state.scanResult === prev.scanResult) return;
  // A null scanResult is the forced-rescan in-flight window, NOT "every file was
  // deleted" — pruning against it would wipe the selection on Ctrl+R. This
  // subscription fires again when the fresh result lands.
  if (state.scanResult === null) return;
  const sel = useSelectionStore.getState().selectedPaths;
  if (sel.size === 0) return;
  const present = new Set(state.scanResult.assets.map((a) => a.path));
  const stale = Array.from(sel).filter((p) => !present.has(p));
  if (stale.length > 0) useSelectionStore.getState().removePaths(stale);
});
