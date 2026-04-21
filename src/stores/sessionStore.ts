import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useProjectStore } from "./projectStore";

/// Cross-session snapshot of which projects were open and which was active.
/// Keeps only paths — the full ProjectData (scanResult, analysisResult, UI
/// state) is rebuilt by re-running openProject on each path at boot.

interface SessionState {
  openProjectPaths: string[];
  activeProjectPath: string | null;
  /** Whether session restore has already run this boot (guards against
   *  re-running if React strict mode double-mounts). */
  restored: boolean;
  setSession: (paths: string[], active: string | null) => void;
  removeFromSession: (path: string) => void;
  markRestored: () => void;
}

export const useSessionStore = create<SessionState>()(
  persist(
    (set) => ({
      openProjectPaths: [],
      activeProjectPath: null,
      restored: false,
      setSession: (paths, active) =>
        set({ openProjectPaths: paths, activeProjectPath: active }),
      removeFromSession: (path) =>
        set((state) => ({
          openProjectPaths: state.openProjectPaths.filter((p) => p !== path),
          activeProjectPath:
            state.activeProjectPath === path ? null : state.activeProjectPath,
        })),
      markRestored: () => set({ restored: true }),
    }),
    {
      name: "tidycraft-session",
      // Don't persist `restored` — it's per-launch runtime state.
      partialize: (state) => ({
        openProjectPaths: state.openProjectPaths,
        activeProjectPath: state.activeProjectPath,
      }),
    }
  )
);

/// Keep the session store in lockstep with the projectStore's open-project
/// set. Any open / close / active-project change eventually reaches here.
/// The quick-check below skips the zustand `set` when nothing meaningful
/// has changed (e.g. during scan progress updates that rewrite `projects`
/// Map but don't add or remove entries).
useProjectStore.subscribe((state) => {
  const session = useSessionStore.getState();
  const paths = Array.from(state.projects.values()).map((p) => p.projectPath);
  const active = state.activeProjectId
    ? state.projects.get(state.activeProjectId)?.projectPath ?? null
    : null;

  const sameSet =
    session.openProjectPaths.length === paths.length &&
    session.openProjectPaths.every((p, i) => p === paths[i]);

  if (sameSet && session.activeProjectPath === active) {
    return;
  }

  session.setSession(paths, active);
});

/// Replay the persisted session on app launch. Call once from App.tsx.
/// openProject is invoked serially — each call runs its scan in the
/// background via Tauri, but we still want the activeProjectId sequence
/// predictable so the restored "active" project wins at the end.
export async function restoreSession(): Promise<void> {
  const session = useSessionStore.getState();
  if (session.restored) return;
  session.markRestored();

  const { openProjectPaths, activeProjectPath } = session;
  if (openProjectPaths.length === 0) return;

  const store = useProjectStore.getState();

  for (const path of openProjectPaths) {
    try {
      await store.openProject(path);
    } catch (err) {
      // openProject already handles its own errors internally; this is a
      // belt-and-suspenders log in case of an unexpected throw.
      console.warn(`[sessionStore] failed to restore project ${path}:`, err);
    }
  }

  // Re-point active to whichever project was active before, if it's still
  // in the Map (the loop above sets activeProjectId to each project as it
  // opens, so the last-opened would otherwise win).
  if (activeProjectPath) {
    const latest = useProjectStore.getState();
    const target = Array.from(latest.projects.values()).find(
      (p) => p.projectPath === activeProjectPath
    );
    if (target && latest.activeProjectId !== target.id) {
      latest.setActiveProject(target.id);
    }
  }
}
