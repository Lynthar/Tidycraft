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
///
/// Two-phase restore:
///   1. Register every persisted path as a stub (parallel, O(1) each):
///      backend gets the project entry, sidebar shows the row, but no
///      scan runs. This used to be a serial `await openProject` loop
///      that triggered N full scans on boot — slow with many projects
///      and wasteful since non-active scans are immediately discarded
///      by the active-switch step at the end.
///   2. Fully open the active project (or fall back to the first one
///      if the previously-active path is gone). `openProject` here will
///      see the stub in the projects Map (path matches), hit the dedupe
///      branch, and route through `setActiveProject`, which detects the
///      stub state and triggers full hydration.
///
/// When the user later switches to a different stub, the same
/// `setActiveProject` lazy-hydration kicks in — they pay the scan cost
/// once, when they actually need that project.
export async function restoreSession(): Promise<void> {
  const session = useSessionStore.getState();
  if (session.restored) return;
  session.markRestored();

  const { openProjectPaths, activeProjectPath } = session;
  if (openProjectPaths.length === 0) return;

  const store = useProjectStore.getState();

  // Phase 1: stubs for every project except the one we're about to
  // open fully. Parallel because each call is just a backend HashMap
  // insert + zustand Map insert; no IO that would benefit from
  // sequencing.
  const stubPaths = openProjectPaths.filter((p) => p !== activeProjectPath);
  await Promise.all(
    stubPaths.map((path) =>
      store.registerProjectStub(path).catch((err) => {
        console.warn(
          `[sessionStore] stub registration failed for ${path}:`,
          err
        );
      })
    )
  );

  // Phase 2: hydrate the previously-active project. If that path is no
  // longer in the open list (shouldn't happen, but be defensive), fall
  // back to the first remaining path so the user lands somewhere sane.
  const target =
    (activeProjectPath && openProjectPaths.includes(activeProjectPath)
      ? activeProjectPath
      : null) ?? openProjectPaths[0] ?? null;

  if (target) {
    try {
      await store.openProject(target);
    } catch (err) {
      console.warn(`[sessionStore] failed to hydrate active ${target}:`, err);
    }
  }
}
