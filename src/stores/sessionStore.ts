import { create } from "zustand";
import { persist } from "zustand/middleware";
import { useProjectStore, checkPaths } from "./projectStore";
import { useRecentsStore } from "./recentsStore";

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

// Not exported: the store is an implementation detail of `restoreSession` and
// the persistence hooks below. Nothing outside this file subscribes to it.
const useSessionStore = create<SessionState>()(
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

/// Keep the session store in lockstep with projectStore's open-project set. The
/// quick check skips the zustand `set` when nothing meaningful changed, e.g.
/// scan-progress updates that rewrite the `projects` Map without adding entries.
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

/// Replay the persisted session on app launch. Call once from App.tsx. Two
/// phases: register every persisted path as a stub in parallel, then fully open
/// the active project (or the first remaining one), which hydrates it lazily.
export async function restoreSession(): Promise<void> {
  const session = useSessionStore.getState();
  if (session.restored) return;
  session.markRestored();

  const { openProjectPaths, activeProjectPath } = session;

  // One batch for everything the switcher can show: last session's projects plus
  // the recents list. Stubs are never scanned, so without this a project whose
  // folder is gone looks healthy.
  //
  // Started here but deliberately NOT awaited: the check stats every path, and a
  // single unreachable network mount holds that for tens of seconds — long enough
  // that awaiting it means the window sits empty until an unrelated NAS answers.
  // The marks it produces only grey out rows in the switcher, so they can land
  // after the interface is up; anything the user actually opens is path-checked
  // by openProject on its own way in.
  const recentPaths = useRecentsStore.getState().recents.map((r) => r.path);
  const healthPromise = checkPaths([
    ...new Set([...openProjectPaths, ...recentPaths]),
  ]);
  const applyHealth = () =>
    void healthPromise.then((health) => {
      useRecentsStore.getState().markHealth(health);
      useProjectStore.getState().markProjectHealth(health);
    });

  if (openProjectPaths.length === 0) {
    applyHealth();
    return;
  }

  const store = useProjectStore.getState();

  // Phase 1: stubs for every project except the one about to open fully.
  // Parallel, since each call is just two Map inserts with no IO.
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

  applyHealth();
}
