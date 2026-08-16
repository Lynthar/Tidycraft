import { create } from "zustand";
import { useProjectStore, registerRecentsBridge } from "./projectStore";
import { basename } from "../lib/pathUtils";
import type { ProjectType, ProjectPathStatus, UnavailableStatus } from "../types/asset";

/// Recently-opened projects, persisted per-machine so the ProjectSwitcher can
/// offer them again after they're closed (and after an app restart). Distinct
/// from `sessionStore`, which restores the projects that were *open* last
/// session; a project can be "recent" long after it's been closed.

const STORAGE_KEY = "tidycraft-recents";
const MAX_RECENTS = 12;

export interface RecentProject {
  path: string;
  name: string;
  engine: ProjectType | null;
  lastOpenedAt: number;
  /// Set by the startup pre-check; deliberately NOT persisted — a stale
  /// verdict written to localStorage would outlive the condition it describes.
  unavailable?: UnavailableStatus | null;
}

interface RecentsState {
  recents: RecentProject[];
  /// Upsert a project to the front (most-recent), deduped by path, capped.
  record: (project: Omit<RecentProject, "lastOpenedAt">) => void;
  remove: (path: string) => void;
  clear: () => void;
  /// Stamp health onto the entries the startup pre-check covered. Entries the
  /// map says nothing about are left untouched, not cleared.
  markHealth: (health: Map<string, ProjectPathStatus>) => void;
}

function load(): RecentProject[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    // Keep only well-shaped entries so a corrupt/old payload can't crash the
    // switcher render.
    return parsed
      .filter(
        (e): e is RecentProject =>
          e &&
          typeof e.path === "string" &&
          typeof e.name === "string" &&
          typeof e.lastOpenedAt === "number"
      )
      .slice(0, MAX_RECENTS);
  } catch {
    return [];
  }
}

function persist(recents: RecentProject[]) {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify(
        recents.map(({ path, name, engine, lastOpenedAt }) => ({
          path,
          name,
          engine,
          lastOpenedAt,
        }))
      )
    );
  } catch {
    // Storage full / unavailable — recents are a convenience, not critical.
  }
}

export const useRecentsStore = create<RecentsState>((set, get) => ({
  recents: load(),
  record: ({ path, name, engine }) => {
    const without = get().recents.filter((r) => r.path !== path);
    const next = [
      { path, name, engine, lastOpenedAt: Date.now() },
      ...without,
    ].slice(0, MAX_RECENTS);
    persist(next);
    set({ recents: next });
  },
  remove: (path) => {
    const next = get().recents.filter((r) => r.path !== path);
    persist(next);
    set({ recents: next });
  },
  clear: () => {
    persist([]);
    set({ recents: [] });
  },
  markHealth: (health) => {
    const next = get().recents.map((r) => {
      const status = health.get(r.path);
      return status ? { ...r, unavailable: status.kind === "ok" ? null : status } : r;
    });
    set({ recents: next });
    // Not persisted — see the field comment.
  },
}));

// Record the active project once it has a scan result (so name + engine are
// known). Guard on the path so we upsert once per activation, not on every
// unrelated projectStore tick (watcher patches, git refreshes, …). Switching
// away and back re-records, bumping it to the front.
let lastRecordedPath: string | null = null;
useProjectStore.subscribe((state) => {
  const path = state.projectPath;
  const scan = state.scanResult;
  if (!path || !scan) return;
  if (path === lastRecordedPath) return;
  lastRecordedPath = path;
  useRecentsStore.getState().record({
    path,
    name: basename(path) || path,
    engine: scan.project_type ?? null,
  });
});

registerRecentsBridge({
  remove: (path: string) => useRecentsStore.getState().remove(path),
});
