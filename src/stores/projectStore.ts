import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { basename, dirname } from "../lib/pathUtils";
import type { ScanResult, AssetInfo, ScanProgress, AssetType, ProjectType, AnalysisResult, UndoResult, HistoryEntry, GitInfo, GitStatusMap, GitFileStatus, FsChangeEvent, RenamedPair, DirectoryNode, ProjectWarning, ProjectPathStatus, ProjectPathReport, UnavailableStatus } from "../types/asset";
import { useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";
import i18n from "../i18n";
import { evictThumbs } from "../lib/thumbnailCache";

// Per-project filesystem-watcher unlisten handles. Kept outside the zustand
// store because function references don't belong in serialized state, and
// we need to dispose them on closeProject.
const fsWatchers = new Map<string, UnlistenFn>();
const warningWatchers = new Map<string, UnlistenFn>();

async function stopFsWatch(projectId: string) {
  const unlisten = fsWatchers.get(projectId);
  if (unlisten) {
    unlisten();
    fsWatchers.delete(projectId);
  }
  const warnUnlisten = warningWatchers.get(projectId);
  if (warnUnlisten) {
    warnUnlisten();
    warningWatchers.delete(projectId);
  }
  try {
    await invoke("stop_watching", { projectId });
  } catch (err) {
    console.error("Failed to stop watcher:", err);
  }
}

/// Ask the backend which of these paths are still usable, keyed by path so a
/// duplicated or reordered input cannot pair a verdict with the wrong project.
/// A failure returns an EMPTY map: a broken health check must not look like death.
export async function checkPaths(
  paths: string[]
): Promise<Map<string, ProjectPathStatus>> {
  if (paths.length === 0) return new Map();
  try {
    const reports = await invoke<ProjectPathReport[]>("check_project_paths", {
      paths,
    });
    return new Map(reports.map((r) => [r.path, r.status]));
  } catch (err) {
    console.error("Failed to check project paths:", err);
    return new Map();
  }
}

/// `ok` and "no answer" both mean "nothing to show" — see the ProjectData
/// field comment.
function toUnavailable(
  status: ProjectPathStatus | undefined
): UnavailableStatus | null {
  return !status || status.kind === "ok" ? null : status;
}

// Per-project debounced git-refresh timers. Each fs-change event resets the
// timer; on quiescence the branch chip and file badges re-fetch. The window sits
// above the watcher's own 500ms coalescing in `watcher.rs`.
const gitRefreshTimers = new Map<string, ReturnType<typeof setTimeout>>();
const GIT_REFRESH_DEBOUNCE_MS = 800;

function scheduleGitRefresh(projectId: string) {
  const existing = gitRefreshTimers.get(projectId);
  if (existing) clearTimeout(existing);
  const timer = setTimeout(() => {
    gitRefreshTimers.delete(projectId);
    useProjectStore.getState().refreshGitInfo(projectId).catch((err) => {
      console.error(`[gitRefresh] failed for ${projectId}:`, err);
    });
  }, GIT_REFRESH_DEBOUNCE_MS);
  gitRefreshTimers.set(projectId, timer);
}

function cancelGitRefresh(projectId: string) {
  const existing = gitRefreshTimers.get(projectId);
  if (existing) {
    clearTimeout(existing);
    gitRefreshTimers.delete(projectId);
  }
}

// Store-level memoization for getFilteredAssets, shared across components so
// they don't each re-run filter+sort on 10k+ assets. All inputs are replaced
// rather than mutated by their setters, so reference equality is a correct check.
let filterCacheInputs: readonly unknown[] | null = null;
let filterCacheResult: AssetInfo[] = [];

type ViewMode = "assets" | "issues" | "stats";

export type SortField = "name" | "type" | "size" | "dimensions" | "vertices" | "faces" | "duration" | "sampleRate" | "extension";
export type SortDirection = "asc" | "desc";

export interface AdvancedFilters {
  minSize: number | null;
  maxSize: number | null;
  minWidth: number | null;
  maxWidth: number | null;
  minHeight: number | null;
  maxHeight: number | null;
  minVertices: number | null;
  maxVertices: number | null;
  minFaces: number | null;
  maxFaces: number | null;
  minDuration: number | null;
  maxDuration: number | null;
  /** Tri-state alpha filter: null = any, true = has alpha, false = no alpha. */
  hasAlpha: boolean | null;
  /** Texture color space, e.g. "sRGB" / "Linear"; null = any. */
  colorSpace: string | null;
  extensions: string[];
  gitStatusFilter: GitFileStatus[];
}

/// The "nothing filtered" state, built fresh each call. A function rather than a
/// shared const because two of its fields are arrays: one const would hand every
/// project the same array to hold.
export function createDefaultAdvancedFilters(): AdvancedFilters {
  return {
    minSize: null,
    maxSize: null,
    minWidth: null,
    maxWidth: null,
    minHeight: null,
    maxHeight: null,
    minVertices: null,
    maxVertices: null,
    minFaces: null,
    maxFaces: null,
    minDuration: null,
    maxDuration: null,
    hasAlpha: null,
    colorSpace: null,
    extensions: [],
    gitStatusFilter: [],
  };
}

// Data for a single project
export interface ProjectData {
  id: string;
  projectPath: string;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;
  analysisResult: AnalysisResult | null;
  /// True when files changed after `analysisResult` was computed, so the analysis
  /// is showing pre-change data. IssueList surfaces a "re-run" banner. Cleared by
  /// the next successful runAnalysis.
  analysisStale: boolean;
  isAnalyzing: boolean;
  viewMode: ViewMode;
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  /// Asset-type filter as a UNION of selected types; `null` is the one
  /// canonical "no filter" state (an empty array normalizes to it in the
  /// setters — same discipline as `selectedDirectory`'s null).
  typeFilter: AssetType[] | null;
  sortField: SortField;
  sortDirection: SortDirection;
  advancedFilters: AdvancedFilters;
  gitInfo: GitInfo | null;
  gitStatuses: GitStatusMap;
  /// True when this project root contains a `tidycraft.toml`. Surfaced in
  /// the UI (Sidebar Run Analysis button) so users know whether the next
  /// analysis will use custom rules or fall back to defaults.
  hasCustomConfig: boolean;
  /// Session-lifetime degradations (watcher trouble), deduped by kind —
  /// scan-time warnings live on scanResult.warnings and are NOT copied here.
  projectWarnings: ProjectWarning[];
  /// Non-null when the project root is not usable. `ok` is stored as null, so
  /// "the path is fine" and "not checked yet" are one state. The branch predicate
  /// is `unavailable !== null`, never `kind`.
  unavailable: UnavailableStatus | null;
}

const createDefaultProjectData = (id: string, path: string): ProjectData => ({
  id,
  projectPath: path,
  scanResult: null,
  isScanning: false,
  error: null,
  scanProgress: null,
  analysisResult: null,
  analysisStale: false,
  isAnalyzing: false,
  viewMode: "assets",
  selectedDirectory: null,
  selectedAsset: null,
  searchQuery: "",
  typeFilter: null,
  sortField: "name",
  sortDirection: "asc",
  advancedFilters: createDefaultAdvancedFilters(),
  gitInfo: null,
  gitStatuses: {},
  hasCustomConfig: false,
  projectWarnings: [],
  unavailable: null,
});

/// Reset a project entry to "just learned something about this folder": the same
/// bones as createDefaultProjectData, but preserving the six observation
/// preferences that describe how the user was looking at the project.
function resetProjectData(
  project: ProjectData,
  path: string,
  unavailable: UnavailableStatus | null = null
): ProjectData {
  return {
    ...createDefaultProjectData(project.id, path),
    viewMode: project.viewMode,
    searchQuery: project.searchQuery,
    typeFilter: project.typeFilter,
    sortField: project.sortField,
    sortDirection: project.sortDirection,
    advancedFilters: project.advancedFilters,
    unavailable,
  };
}

const generateProjectId = (): string => {
  return `project_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
};

// Bridge registered by tagsStore at its module init. tagsStore already imports
// this store, so a static import back would close an ESM cycle and TDZ-crash at
// startup.
interface TagFilterBridge {
  /// Clear the active tag filters if (and only if) they would exclude
  /// `path` from the asset list.
  clearIfFiltering: (path: string) => void;
}
let tagFilterBridge: TagFilterBridge | null = null;
export const registerTagFilterBridge = (bridge: TagFilterBridge) => {
  tagFilterBridge = bridge;
};

// Same cycle constraint as the tag-filter bridge: recentsStore imports this
// store, so removing a recent has to come back through a hook it registers.
interface RecentsBridge {
  remove: (path: string) => void;
}
let recentsBridge: RecentsBridge | null = null;
export const registerRecentsBridge = (bridge: RecentsBridge) => {
  recentsBridge = bridge;
};

// Same cycle constraint, two more hooks — both fired from applyFsChange.
interface SelectionSyncBridge {
  /// Re-point batch-selected paths across recognized renames. Called BEFORE the
  /// scanResult swap: selectionStore's prune subscription fires on that swap and
  /// must already see the new paths, or it prunes them as stale.
  applyRenames: (renamed: RenamedPair[]) => void;
}
let selectionSyncBridge: SelectionSyncBridge | null = null;
export const registerSelectionSyncBridge = (bridge: SelectionSyncBridge) => {
  selectionSyncBridge = bridge;
};

interface TagsSyncBridge {
  /// The backend migrated (renames) or reaped (removals) tag bindings during
  /// an fs change — re-pull the active project's tag mirror.
  reloadTags: () => void;
}
let tagsSyncBridge: TagsSyncBridge | null = null;
export const registerTagsSyncBridge = (bridge: TagsSyncBridge) => {
  tagsSyncBridge = bridge;
};

/// New path for `path` under this batch's renames: an exact file match, or a
/// directory pair's prefix rewrite (component-boundary safe — `Tex` never
/// captures `Textures/…`). Null when the path is untouched.
export function renamedTargetFor(
  path: string,
  renamed: RenamedPair[]
): string | null {
  for (const r of renamed) {
    if (path === r.from) return r.to;
    if (r.is_dir && path.startsWith(r.from + "/")) {
      return r.to + path.slice(r.from.length);
    }
  }
  return null;
}

// True when a project entry is an unhydrated stub — registered but never scanned
// — that should be hydrated the moment it becomes active. False when `error` or
// `unavailable` is set, so a permanent failure does not re-scan on every switch.
const needsHydration = (project: ProjectData): boolean =>
  project.scanResult === null &&
  !project.isScanning &&
  !project.error &&
  !project.unavailable;

// Filtering by the project root is identical to no filter, so the root is stored
// as `null`. Every path that assigns `selectedDirectory` goes through here, not
// just the setter.
const normalizeDirectory = (path: string | null, projectPath: string | null): string | null =>
  path && path === projectPath ? null : path;

interface ProjectState {
  // Multi-project state
  projects: Map<string, ProjectData>;
  activeProjectId: string | null;

  // Global undo state (shared across projects)
  canUndo: boolean;
  undoHistory: HistoryEntry[];

  /// Monotonic timestamp bumped each time the filesystem watcher reports a
  /// change for any project. StatusBar subscribes to this to flash a
  /// "syncing" indicator. 0 = no events seen yet this session.
  watcherPulse: number;

  /// Monotonic counter bumped by every locateAsset call. The asset views scroll
  /// on [selectedAsset.path, locatePulse], so a repeat locate still scrolls
  /// without every filter change fighting the user's scroll position.
  locatePulse: number;

  /// Monotonic counter bumped only when the active project's previewed asset was
  /// dropped because its file was removed — never for any other reason a
  /// selection can go away.
  selectionRemovedPulse: number;

  // Convenience getters for active project
  projectPath: string | null;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;
  analysisResult: AnalysisResult | null;
  analysisStale: boolean;
  isAnalyzing: boolean;
  viewMode: ViewMode;
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  /// Asset-type filter as a UNION of selected types; `null` is the one
  /// canonical "no filter" state (an empty array normalizes to it in the
  /// setters — same discipline as `selectedDirectory`'s null).
  typeFilter: AssetType[] | null;
  sortField: SortField;
  sortDirection: SortDirection;
  advancedFilters: AdvancedFilters;
  gitInfo: GitInfo | null;
  gitStatuses: GitStatusMap;
  hasCustomConfig: boolean;
  projectWarnings: ProjectWarning[];
  unavailable: UnavailableStatus | null;

  // Multi-project actions
  openProject: (path: string, options?: { force?: boolean }) => Promise<void>;
  /// Register a project with the backend and add a stub `ProjectData` without
  /// scanning, so session-restored projects appear in the sidebar instantly and
  /// hydrate lazily. Idempotent for a path already in the Map. Path health is
  /// stamped separately by `markProjectHealth`, off the startup critical path.
  registerProjectStub: (rawPath: string) => Promise<void>;
  /// Stamp a batch path-health result onto projects that have nothing better to
  /// go on. Startup runs the check off the critical path, so this lands after the
  /// interface is already up.
  markProjectHealth: (health: Map<string, ProjectPathStatus>) => void;
  /// Point an existing project at a different folder. The projectId is kept, so
  /// this is "this project moved", not "close one and open another" — deleting a
  /// project is a separate action.
  relocateProject: (projectId: string, newPath: string) => Promise<void>;
  /// Close a project AND drop it from recents, so an unusable path stops
  /// coming back as a suggestion in the same menu.
  removeProject: (projectId: string) => void;
  closeProject: (projectId?: string) => void;
  setActiveProject: (projectId: string) => void;
  getProjectList: () => {
    id: string;
    name: string;
    path: string;
    isActive: boolean;
    assetCount: number | null;
    issueCount: number | null;
    engine: ProjectType | null;
    unavailable: UnavailableStatus | null;
  }[];

  // Active project actions
  cancelScan: () => Promise<void>;
  /// Cache-clearing force rescan, shared by the Header rescan button and the
  /// Ctrl+R shortcut (the button's tooltip advertises Ctrl+R, so the two must
  /// behave identically). No-op without an active project or while scanning.
  rescan: () => Promise<void>;
  clearError: () => void;
  runAnalysis: () => Promise<void>;
  /// Optimistic prune after the duplicate-group cleanup: drop that group's issue
  /// so the card disappears immediately, counts recomputed wholesale. `groupKey`
  /// is the group's first `related_paths` member. Does not touch `analysisStale`.
  pruneDuplicateGroup: (projectId: string, groupKey: string) => void;
  setViewMode: (mode: ViewMode) => void;
  setHasCustomConfig: (value: boolean) => void;
  setSelectedDirectory: (path: string | null) => void;
  setSelectedAsset: (asset: AssetInfo | null) => void;
  setSearchQuery: (query: string) => void;
  setTypeFilter: (types: AssetType[] | null) => void;
  /// Add/remove one type from the filter set (Ctrl+click on a pill, the
  /// command palette's filter entries, the advanced panel's type chips).
  toggleTypeFilter: (type: AssetType) => void;
  setSortField: (field: SortField) => void;
  toggleSortDirection: () => void;
  locateAsset: (path: string) => void;
  setAdvancedFilters: (filters: Partial<AdvancedFilters>) => void;
  resetAdvancedFilters: () => void;

  // Undo actions
  undoLastOperation: () => Promise<UndoResult | null>;
  refreshUndoState: () => Promise<void>;
  clearUndoHistory: () => Promise<void>;

  // Git actions
  refreshGitInfo: (targetProjectId?: string) => Promise<void>;

  // Computed
  getFilteredAssets: () => AssetInfo[];
}

// Helper to update active project data
const updateActiveProject = (
  state: ProjectState,
  updates: Partial<ProjectData>
): Partial<ProjectState> => {
  const { activeProjectId, projects } = state;
  if (!activeProjectId) return {};

  const project = projects.get(activeProjectId);
  if (!project) return {};

  const updatedProject = { ...project, ...updates };
  const newProjects = new Map(projects);
  newProjects.set(activeProjectId, updatedProject);

  // Return both the updated projects map and the convenience fields
  const result: Partial<ProjectState> = { projects: newProjects };

  // Update convenience fields
  if ('projectPath' in updates) result.projectPath = updates.projectPath ?? null;
  if ('scanResult' in updates) result.scanResult = updates.scanResult ?? null;
  if ('isScanning' in updates) result.isScanning = updates.isScanning ?? false;
  if ('error' in updates) result.error = updates.error ?? null;
  if ('scanProgress' in updates) result.scanProgress = updates.scanProgress ?? null;
  if ('analysisResult' in updates) result.analysisResult = updates.analysisResult ?? null;
  if ('analysisStale' in updates) result.analysisStale = updates.analysisStale ?? false;
  if ('isAnalyzing' in updates) result.isAnalyzing = updates.isAnalyzing ?? false;
  if ('viewMode' in updates) result.viewMode = updates.viewMode ?? "assets";
  if ('selectedDirectory' in updates) result.selectedDirectory = updates.selectedDirectory ?? null;
  if ('selectedAsset' in updates) result.selectedAsset = updates.selectedAsset ?? null;
  if ('searchQuery' in updates) result.searchQuery = updates.searchQuery ?? "";
  if ('typeFilter' in updates) result.typeFilter = updates.typeFilter ?? null;
  if ('sortField' in updates) result.sortField = updates.sortField ?? "name";
  if ('sortDirection' in updates) result.sortDirection = updates.sortDirection ?? "asc";
  if ('advancedFilters' in updates) result.advancedFilters = updates.advancedFilters ?? state.advancedFilters;
  if ('gitInfo' in updates) result.gitInfo = updates.gitInfo ?? null;
  if ('gitStatuses' in updates) result.gitStatuses = updates.gitStatuses ?? {};
  if ('hasCustomConfig' in updates) result.hasCustomConfig = updates.hasCustomConfig ?? false;
  if ('projectWarnings' in updates) result.projectWarnings = updates.projectWarnings ?? [];
  if ('unavailable' in updates) result.unavailable = updates.unavailable ?? null;

  return result;
};

// Helper to sync convenience fields from active project
const syncFromActiveProject = (project: ProjectData | undefined): Partial<ProjectState> => {
  if (!project) {
    return {
      projectPath: null,
      scanResult: null,
      isScanning: false,
      error: null,
      scanProgress: null,
      analysisResult: null,
      analysisStale: false,
      isAnalyzing: false,
      viewMode: "assets",
      selectedDirectory: null,
      selectedAsset: null,
      searchQuery: "",
      typeFilter: null,
      sortField: "name",
      sortDirection: "asc",
      advancedFilters: createDefaultAdvancedFilters(),
      gitInfo: null,
      gitStatuses: {},
      hasCustomConfig: false,
      projectWarnings: [],
      unavailable: null,
    };
  }

  return {
    projectPath: project.projectPath,
    scanResult: project.scanResult,
    isScanning: project.isScanning,
    error: project.error,
    scanProgress: project.scanProgress,
    analysisResult: project.analysisResult,
    analysisStale: project.analysisStale,
    isAnalyzing: project.isAnalyzing,
    viewMode: project.viewMode,
    selectedDirectory: project.selectedDirectory,
    selectedAsset: project.selectedAsset,
    searchQuery: project.searchQuery,
    typeFilter: project.typeFilter,
    sortField: project.sortField,
    sortDirection: project.sortDirection,
    advancedFilters: project.advancedFilters,
    gitInfo: project.gitInfo,
    gitStatuses: project.gitStatuses,
    hasCustomConfig: project.hasCustomConfig,
    projectWarnings: project.projectWarnings,
    unavailable: project.unavailable,
  };
};

// True when `path` names a directory node anywhere in `tree`. Keeps
// `selectedDirectory` honest against a fresh tree: filtering by a directory that
// no longer exists yields a blank view with no explanation.
function directoryExistsInTree(tree: DirectoryNode, path: string): boolean {
  if (tree.path === path) return true;
  return tree.children.some((child) => directoryExistsInTree(child, path));
}

/// Record a session-lifetime warning against a (possibly non-active) project.
/// Deduped by kind, latest payload wins. `tags_not_saved` becomes a toast rather
/// than a list entry — it belongs to the operation the user just performed.
function recordProjectWarning(projectId: string, w: ProjectWarning) {
  if (w.kind === "tags_not_saved") {
    useToastStore.getState().push({
      kind: "error",
      message: i18n.t("warnings.tags_not_saved.body", { detail: w.detail }),
    });
    return;
  }
  if (w.kind === "sidecar_not_carried") {
    // The one warning that is both: a toast, because it belongs to the rename the
    // user just performed and they should hear about broken references now — and
    // a list entry, because its sample of affected assets is what tells them
    // which files to fix, and a toast is gone before they can read five paths.
    useToastStore.getState().push({
      kind: "error",
      message: i18n.t("warnings.sidecar_not_carried.body", {
        affected: w.affected,
        detail: w.detail,
      }),
    });
  }
  const state = useProjectStore.getState();
  const target = state.projects.get(projectId);
  if (!target) return;
  const kept = target.projectWarnings.filter((p) => p.kind !== w.kind);
  const updated = { ...target, projectWarnings: [...kept, w] };
  const newMap = new Map(state.projects);
  newMap.set(projectId, updated);
  const patch: Partial<ProjectState> = { projects: newMap };
  if (state.activeProjectId === projectId) {
    patch.projectWarnings = updated.projectWarnings;
  }
  useProjectStore.setState(patch);
}

/// Drop one warning kind — a successful watcher start supersedes the
/// failure note from an earlier attempt.
function clearProjectWarning(projectId: string, kind: ProjectWarning["kind"]) {
  const state = useProjectStore.getState();
  const target = state.projects.get(projectId);
  if (!target || !target.projectWarnings.some((p) => p.kind === kind)) return;
  const updated = {
    ...target,
    projectWarnings: target.projectWarnings.filter((p) => p.kind !== kind),
  };
  const newMap = new Map(state.projects);
  newMap.set(projectId, updated);
  const patch: Partial<ProjectState> = { projects: newMap };
  if (state.activeProjectId === projectId) {
    patch.projectWarnings = updated.projectWarnings;
  }
  useProjectStore.setState(patch);
}

// Apply a filesystem-change event into the store. Targets the project the event
// was emitted for, even if the user has switched away.
function applyFsChange(projectId: string, event: FsChangeEvent) {
  const state = useProjectStore.getState();
  const target = state.projects.get(projectId);
  if (!target || !target.scanResult) return;

  const merged = new Map<string, AssetInfo>();
  for (const a of target.scanResult.assets) merged.set(a.path, a);
  for (const p of event.removed) merged.delete(p);
  for (const a of event.updated) merged.set(a.path, a);

  // Modified/removed files may have a stale thumbnail in the gallery's
  // in-memory cache (path-keyed; the backend disk cache is mtime-keyed and
  // regenerates on its own). Evict them so cards re-fetch fresh images.
  evictThumbs([...event.updated.map((a) => a.path), ...event.removed]);

  const newScanResult: ScanResult = {
    ...target.scanResult,
    assets: Array.from(merged.values()),
    directory_tree: event.directory_tree,
    total_count: event.total_count,
    total_size: event.total_size,
    type_counts: event.type_counts,
  };

  // Reconcile selectedAsset: follow it across a recognized rename, swap to
  // the fresh copy if it was re-parsed, or drop it if the file was deleted.
  let newSelectedAsset = target.selectedAsset;
  let selectionRemoved = false;
  if (newSelectedAsset) {
    const renamedTo = renamedTargetFor(newSelectedAsset.path, event.renamed);
    if (renamedTo) {
      newSelectedAsset = merged.get(renamedTo) ?? null;
    } else if (event.removed.includes(newSelectedAsset.path)) {
      newSelectedAsset = null;
      selectionRemoved = true;
    } else {
      const fresh = event.updated.find((a) => a.path === newSelectedAsset!.path);
      if (fresh) newSelectedAsset = fresh;
    }
  }

  // Reconcile selectedDirectory: follow a renamed folder, then drop the scope
  // when the selected folder is genuinely gone. The fallback is `null`, not
  // `root_path` — "entire project" is spelled `null` everywhere else.
  let newSelectedDirectory = target.selectedDirectory;
  if (newSelectedDirectory) {
    const renamedTo = renamedTargetFor(newSelectedDirectory, event.renamed);
    if (renamedTo) newSelectedDirectory = renamedTo;
  }
  if (
    newSelectedDirectory &&
    !directoryExistsInTree(event.directory_tree, newSelectedDirectory)
  ) {
    newSelectedDirectory = null;
  }

  const updated: ProjectData = {
    ...target,
    scanResult: newScanResult,
    selectedAsset: newSelectedAsset,
    selectedDirectory: newSelectedDirectory,
    // The analysis (if any) was computed against the pre-change files — flag
    // it so IssueList shows the "re-run" banner instead of silently serving
    // stale issues.
    analysisStale: target.analysisResult !== null || target.analysisStale,
  };
  const newMap = new Map(state.projects);
  newMap.set(projectId, updated);

  const patch: Partial<ProjectState> = {
    projects: newMap,
    watcherPulse: Date.now(),
  };
  if (state.activeProjectId === projectId) {
    Object.assign(patch, syncFromActiveProject(updated));
    if (selectionRemoved) {
      patch.selectionRemovedPulse = state.selectionRemovedPulse + 1;
    }
  }
  // Order matters: re-point the batch selection while the OLD scanResult is
  // still in place — selectionStore's prune subscription fires on the swap
  // below and must find the new paths already selected (see the bridge doc).
  if (event.renamed.length > 0 && state.activeProjectId === projectId) {
    selectionSyncBridge?.applyRenames(event.renamed);
  }
  useProjectStore.setState(patch);

  // Tag bindings moved (renames) or were reaped (removals) on the backend;
  // the mirror in tagsStore only reloads on project switch by itself.
  if (
    (event.renamed.length > 0 || event.removed.length > 0) &&
    state.activeProjectId === projectId
  ) {
    tagsSyncBridge?.reloadTags();
  }

  // Files changed → git status may have changed too. Debounce so that a
  // burst (e.g. batch rename, `git checkout` outside the app) collapses into
  // one refresh rather than one per file.
  scheduleGitRefresh(projectId);
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  // Multi-project initial state
  projects: new Map(),
  activeProjectId: null,

  // Global state
  canUndo: false,
  undoHistory: [],
  watcherPulse: 0,
  locatePulse: 0,
  selectionRemovedPulse: 0,

  // Initial convenience fields (no active project)
  projectPath: null,
  scanResult: null,
  isScanning: false,
  error: null,
  scanProgress: null,
  analysisResult: null,
  analysisStale: false,
  isAnalyzing: false,
  viewMode: "assets",
  selectedDirectory: null,
  selectedAsset: null,
  searchQuery: "",
  typeFilter: null,
  sortField: "name",
  sortDirection: "asc",
  advancedFilters: createDefaultAdvancedFilters(),
  gitInfo: null,
  gitStatuses: {},
  hasCustomConfig: false,
  projectWarnings: [],
  unavailable: null,

  // Multi-project actions
  openProject: async (rawPath: string, options?: { force?: boolean }) => {
    const { projects } = get();

    // Normalize path separators: the Tauri dialog returns OS-native paths, but
    // the scanner, `selectedDirectory` filtering, `convertFileSrc` and tree
    // navigation all expect forward slashes.
    const path = rawPath.replace(/\\/g, "/");

    const existingProject = Array.from(projects.values()).find(p => p.projectPath === path);

    // If the project is already open and this isn't a force-rescan, just
    // switch the active project.
    if (existingProject && !options?.force) {
      get().setActiveProject(existingProject.id);
      return;
    }

    // Don't start a second scan while one is already in flight for this
    // project. The backend rejects concurrent scans (its scan_state guard),
    // but bailing here avoids the wasted IPC and a spurious error toast.
    if (existingProject?.isScanning) {
      get().setActiveProject(existingProject.id);
      return;
    }

    // Path health before anything else: register + scan on a folder that is
    // gone produces an error the user can only "Retry" into the same wall.
    const health = toUnavailable((await checkPaths([path])).get(path));
    if (health) {
      // This branch crosses an await (the health check) before writing a Map
      // entry. A second openProject for the same path that raced ahead during
      // that await already inserted one — adopt it rather than duplicate.
      if (!existingProject) {
        const winner = Array.from(get().projects.values()).find(
          (p) => p.projectPath === path
        );
        if (winner) {
          get().setActiveProject(winner.id);
          return;
        }
      }
      // The mirror image: the entry was CLOSED while the health check ran — or
      // RELOCATED, which keeps the id and changes the path. Writing the pre-await
      // snapshot back would resurrect a project the user just removed (its backend
      // registration already gone), or drag a just-relocated project back to the
      // folder it left. The check is slow enough to make both reachable: it stats
      // every path, and a dead network mount can hold that for tens of seconds
      // while this panel — and its Locate button — stay on screen.
      const stillOurs = existingProject
        ? get().projects.get(existingProject.id)
        : undefined;
      if (existingProject && stillOurs?.projectPath !== path) return;
      const id = existingProject?.id ?? generateProjectId();
      const data: ProjectData = stillOurs
        ? resetProjectData(stillOurs, path, health)
        : { ...createDefaultProjectData(id, path), unavailable: health };
      const newMap = new Map(get().projects);
      newMap.set(id, data);
      const patch: Partial<ProjectState> = { projects: newMap, activeProjectId: id };
      set({ ...patch, ...syncFromActiveProject(data) });
      return;
    }

    // Reuse the existing projectId on force-rescan so the backend's
    // ProjectState (undo history, watcher, tags, git manager) survives.
    const projectId = existingProject?.id ?? generateProjectId();

    // Same two races as the unavailable branch above, re-checked after ITS await:
    // registering now would rebuild the backend against a path this project has
    // since left, or resurrect one that was closed.
    const current = existingProject ? get().projects.get(projectId) : undefined;
    if (existingProject && current?.projectPath !== path) {
      return;
    }

    // A project coming back from `unavailable` may still hold a watcher that no
    // longer reports anything: the OS watch does not reliably survive its root
    // being deleted or renamed (notify documents this as platform-dependent), and
    // the install below is skipped whenever `fsWatchers` already has an entry. So
    // without this teardown a recovered project runs watcher-less, and the only
    // symptom is that live updates quietly never arrive again. relocateProject
    // does the same before its own rebuild; a plain force-rescan, whose watcher is
    // healthy, deliberately keeps it.
    if (current?.unavailable && fsWatchers.has(projectId)) {
      await stopFsWatch(projectId);
    }

    // Register with the backend BEFORE flipping activeProjectId, so subscribers
    // that re-load on that change don't race an unregistered project into their
    // invoke calls. Registering an existing id with a different path rebuilds it.
    try {
      await invoke("register_project", { projectId, path });
    } catch (err) {
      // This runs before the project has a Map entry, so there is no
      // per-project error slot for the status bar to render — a console line
      // means the user clicks Open Project and watches nothing happen.
      useToastStore.getState().push({
        kind: "error",
        message: i18n.t("projects.openFailed", { reason: String(err) }),
      });
      return;
    }

    // Concurrent double-open guard: a second openProject for the same path may
    // have written its Map entry while register_project was in flight. Adopt the
    // winner; this call's backend registration is dropped best-effort.
    if (!existingProject) {
      const winner = Array.from(get().projects.values()).find(
        (p) => p.projectPath === path
      );
      if (winner) {
        void invoke("unregister_project", { projectId }).catch(() => {});
        get().setActiveProject(winner.id);
        return;
      }
    } else if (get().projects.get(projectId)?.projectPath !== path) {
      // Closed or relocated during register_project itself. That registration has
      // already rebuilt the backend against `path`, and a relocation's own
      // openProject re-registers the new root behind us — so what is left to
      // prevent is writing this call's snapshot into the store on top of it.
      return;
    }

    // For force-rescan, keep the user's UI state (view mode, filters,
    // selection, etc.) and only reset the scan-related fields.
    const projectData: ProjectData = existingProject
      ? {
          ...existingProject,
          isScanning: true,
          error: null,
          scanProgress: null,
          scanResult: null,
          // The pre-check above already confirmed the path is healthy, so a
          // prior unavailable mark (e.g. from a stub, or an earlier failed
          // open) is stale by the time we get here.
          unavailable: null,
        }
      : { ...createDefaultProjectData(projectId, path), isScanning: true };

    const newProjects = new Map(get().projects);
    newProjects.set(projectId, projectData);

    set({
      projects: newProjects,
      activeProjectId: projectId,
      ...syncFromActiveProject(projectData),
    });

    let unlisten: UnlistenFn | null = null;

    try {
      // Listen for this project's scan progress events.
      unlisten = await listen<ScanProgress>(`scan-progress-${projectId}`, (event) => {
        // Update progress against the project that owns this scan, even if
        // the user has switched the active project mid-scan.
        const state = get();
        const target = state.projects.get(projectId);
        if (!target) return;
        const updated = { ...target, scanProgress: event.payload };
        const newMap = new Map(state.projects);
        newMap.set(projectId, updated);
        const patch: Partial<ProjectState> = { projects: newMap };
        if (state.activeProjectId === projectId) {
          patch.scanProgress = event.payload;
        }
        set(patch);
      });

      // Read the user's "Respect .gitignore" setting at scan kickoff time.
      // Toggling this setting after a scan kicks off has no effect on the
      // in-flight scan — the next openProject call picks it up.
      const respectGitignore = useSettingsStore.getState().respectGitignore;

      // Use incremental scan command
      const { result } = await invoke<{ result: ScanResult; stats: { cached_files: number; rescanned_files: number } }>(
        "scan_project_incremental",
        { projectId, path, respectGitignore }
      );

      // Probe for a project-local `tidycraft.toml` so the UI can flag
      // whether the next analysis will use custom rules. Best-effort —
      // failure just means we'll fall back to defaults at analyze time.
      let hasCustomConfig = false;
      try {
        const cfg = await invoke<string | null>("read_project_config", {
          projectId,
        });
        hasCustomConfig = cfg !== null;
      } catch (err) {
        console.warn("Failed to probe tidycraft.toml:", err);
      }

      // Apply scan result to the project that owns it (not necessarily active),
      // and only while it still points at the folder this scan walked — a
      // relocation keeps the id and changes the path, so the id alone would let a
      // scan of the old root install itself as the new one's result.
      const state = get();
      const target = state.projects.get(projectId);
      if (target && target.projectPath === path) {
        // Force-rescan keeps UI state, including the directory selection. Falls
        // back to `null` (= whole project) when the directory vanished between
        // scans — never the root path, which must not be a second spelling.
        const keptDirectory =
          target.selectedDirectory &&
          target.selectedDirectory !== path &&
          directoryExistsInTree(result.directory_tree, target.selectedDirectory)
            ? target.selectedDirectory
            : null;
        const updated = {
          ...target,
          scanResult: result,
          isScanning: false,
          selectedDirectory: keptDirectory,
          selectedAsset: null,
          scanProgress: null,
          hasCustomConfig,
          // A rescan may have changed the file set under an existing
          // analysis snapshot — mark it stale (first scans have no
          // analysisResult, so this stays false for them).
          analysisStale: target.analysisResult !== null || target.analysisStale,
          // A scan that reaches this line proved the folder readable, so any
          // stale unavailable mark (from a stub or the force-rescan branch
          // above) no longer applies.
          unavailable: null,
        };
        const newMap = new Map(state.projects);
        newMap.set(projectId, updated);
        const patch: Partial<ProjectState> = { projects: newMap };
        if (state.activeProjectId === projectId) {
          Object.assign(patch, syncFromActiveProject(updated));
        }
        set(patch);
      } else {
        // Closed, or relocated while this scan ran. The result describes a folder
        // the project has left, and the git refresh and watcher install below
        // belong to whoever owns it now. `finally` still unlistens.
        return;
      }

      // refreshGitInfo patches the right entry in the Map regardless of which
      // project is active, so no "is still active" guard is needed here.
      get().refreshGitInfo(projectId);

      // Start the watcher now that the cache is populated: earlier events would
      // be no-ops backend-side. On force-rescan the watcher is already running,
      // so skip it to avoid stacking duplicate listeners.
      if (!fsWatchers.has(projectId)) {
        try {
          const fsUnlisten = await listen<FsChangeEvent>(
            `fs-change-${projectId}`,
            (event) => applyFsChange(projectId, event.payload)
          );
          fsWatchers.set(projectId, fsUnlisten);
          const warnUnlisten = await listen<ProjectWarning>(
            `project-warning-${projectId}`,
            (event) => recordProjectWarning(projectId, event.payload)
          );
          warningWatchers.set(projectId, warnUnlisten);
          await invoke("start_watching", { projectId });
          // A successful start supersedes any earlier failure note.
          clearProjectWarning(projectId, "watcher_start_failed");
        } catch (err) {
          console.error("Failed to start watcher:", err);
          recordProjectWarning(projectId, {
            kind: "watcher_start_failed",
            detail: String(err),
          });
          await stopFsWatch(projectId);
        }
      }
    } catch (err) {
      // A concurrent scan already owns this project's scan state (the backend
      // rejected this one); let that scan finish and drive isScanning /
      // scanResult — don't clobber it here. The finally block still unlistens.
      if (String(err).includes("already in progress")) return;
      const errorMessage = String(err);
      // Gone, or relocated while this scan ran: a failure on the old root says
      // nothing about the folder the project points at now, and the recheck below
      // would ask about the wrong path.
      if (get().projects.get(projectId)?.projectPath !== path) return;
      const isCancelled = errorMessage.includes("cancelled");
      // The folder can go away between the pre-check and the scan. Ask again
      // rather than pattern-matching the error prose, which breaks silently the
      // moment a message is reworded.
      const recheck = isCancelled
        ? null
        : toUnavailable((await checkPaths([path])).get(path));
      // Re-read everything after the await — the Map, this entry, and the active
      // id. Writing a stale Map back would drop a watcher batch that patched a
      // different project, and this project may have been closed meanwhile.
      const latest = get();
      const target = latest.projects.get(projectId);
      if (!target || target.projectPath !== path) return;
      // The folder disappearing invalidates everything derived from it, not just
      // scanResult, so the whole entry resets through the same `resetProjectData`
      // relocateProject and the pre-check branch use.
      const updated: ProjectData = recheck
        ? resetProjectData(target, path, recheck)
        : {
            ...target,
            isScanning: false,
            scanProgress: null,
            unavailable: null,
            error: isCancelled ? null : errorMessage,
          };
      const newMap = new Map(latest.projects);
      newMap.set(projectId, updated);
      const patch: Partial<ProjectState> = { projects: newMap };
      if (latest.activeProjectId === projectId) {
        Object.assign(patch, syncFromActiveProject(updated));
      }
      set(patch);
    } finally {
      if (unlisten) {
        unlisten();
      }
    }
  },

  closeProject: (projectId?: string) => {
    const { projects, activeProjectId } = get();
    const idToClose = projectId || activeProjectId;

    if (!idToClose) return;

    // Stop the watcher first so no events arrive for state we're about to
    // drop. Best-effort; fire-and-forget.
    stopFsWatch(idToClose).catch((err) => {
      console.error("Failed to stop watcher:", err);
    });

    // Cancel any pending git refresh for this project — refreshGitInfo
    // would silently drop the write anyway (project no longer in Map), but
    // we'd rather not spend the IPC.
    cancelGitRefresh(idToClose);

    // Tell the backend to drop its state for this project (best-effort).
    invoke("unregister_project", { projectId: idToClose }).catch((err) => {
      console.error("Failed to unregister project:", err);
    });

    const newProjects = new Map(projects);
    newProjects.delete(idToClose);

    // If closing active project, switch to another one
    let newActiveId: string | null = null;
    if (idToClose === activeProjectId && newProjects.size > 0) {
      newActiveId = newProjects.keys().next().value ?? null;
    } else if (idToClose !== activeProjectId) {
      newActiveId = activeProjectId;
    }

    const activeProject = newActiveId ? newProjects.get(newActiveId) : undefined;

    set({
      projects: newProjects,
      activeProjectId: newActiveId,
      ...syncFromActiveProject(activeProject),
    });

    // Closing the active project promotes the next one directly, bypassing
    // setActiveProject, so the same lazy-hydration rule applies here — a promoted
    // stub would otherwise render a permanently blank asset view.
    if (idToClose === activeProjectId && activeProject && needsHydration(activeProject)) {
      void get().openProject(activeProject.projectPath, { force: true });
    }
  },

  setActiveProject: (projectId: string) => {
    const { projects, activeProjectId } = get();
    if (projectId === activeProjectId) return;
    const project = projects.get(projectId);
    if (!project) return;

    // Lazy hydration: a stub gets a full openProject, whose force path replaces
    // the stub's ProjectData, wires the scan-progress and fs-change listeners,
    // runs the scan, and starts the watcher on completion.
    if (needsHydration(project)) {
      void get().openProject(project.projectPath, { force: true });
      return;
    }

    set({
      activeProjectId: projectId,
      ...syncFromActiveProject(project),
    });
    // The cached gitInfo/gitStatuses for this project may be stale
    // (e.g. user did `git checkout` while it was inactive). Re-fetch.
    get().refreshGitInfo(projectId);
  },

  registerProjectStub: async (rawPath: string) => {
    const path = rawPath.replace(/\\/g, "/");
    const { projects } = get();

    // Dedupe — a stub may already exist if restoreSession double-ran
    // (React strict mode) or if the user manually opened this project
    // before sessionStore got around to restoring it.
    const existing = Array.from(projects.values()).find(
      (p) => p.projectPath === path
    );
    if (existing) return;

    const projectId = generateProjectId();
    try {
      await invoke("register_project", { projectId, path });
    } catch (err) {
      console.error("Failed to register project stub:", err);
      return;
    }

    // Same double-entry guard as openProject: the pre-await dedupe above
    // can't see an entry that lands while register_project is in flight.
    const winner = Array.from(get().projects.values()).find(
      (p) => p.projectPath === path
    );
    if (winner) {
      void invoke("unregister_project", { projectId }).catch(() => {});
      return;
    }

    const stub = createDefaultProjectData(projectId, path);
    const newMap = new Map(get().projects);
    newMap.set(projectId, stub);
    set({ projects: newMap });
  },

  markProjectHealth: (health: Map<string, ProjectPathStatus>) => {
    const { projects, activeProjectId } = get();
    const next = new Map(projects);
    let changed = false;
    for (const [id, project] of projects) {
      const status = health.get(project.projectPath);
      if (!status) continue;
      // Fills a blank, never overwrites a verdict. Anything that has scanned, is
      // scanning, failed, or already carries a verdict learned it from its own
      // openProject — which checked the path later than this batch did, so
      // stamping this result over it would replace fresh knowledge with stale.
      if (
        project.unavailable ||
        project.scanResult ||
        project.isScanning ||
        project.error
      ) {
        continue;
      }
      const unavailable = toUnavailable(status);
      if (!unavailable) continue;
      next.set(id, { ...project, unavailable });
      changed = true;
    }
    if (!changed) return;
    const active = activeProjectId ? next.get(activeProjectId) : undefined;
    set({
      projects: next,
      ...(active ? syncFromActiveProject(active) : {}),
    });
  },

  relocateProject: async (projectId: string, rawPath: string) => {
    const newPath = rawPath.replace(/\\/g, "/");
    const project = get().projects.get(projectId);
    if (!project) return;

    // Two entries pointing at one folder is what openProject's path-based
    // dedupe has always prevented; relocation bypasses that lookup, so it
    // has to refuse the collision itself.
    const clash = Array.from(get().projects.values()).find(
      (p) => p.projectPath === newPath && p.id !== projectId
    );
    if (clash) {
      useToastStore.getState().push({
        kind: "error",
        message: i18n.t("projects.unavailable.alreadyOpen", {
          name: basename(newPath) || newPath,
        }),
      });
      return;
    }

    // The watcher map is keyed by projectId and the id does not change here.
    // openProject only installs a watcher when the map has no entry, so without
    // this teardown the relocated project would run with none.
    await stopFsWatch(projectId);

    // Everything derived from the old root goes; the six observation
    // preferences stay (none of them holds a path).
    const relocated: ProjectData = resetProjectData(project, newPath);
    const newMap = new Map(get().projects);
    newMap.set(projectId, relocated);
    set({
      projects: newMap,
      ...(get().activeProjectId === projectId
        ? syncFromActiveProject(relocated)
        : {}),
    });

    // openProject finds this entry by its (new) path, reuses the id, and its
    // register_project call rebuilds the backend state on the new root.
    await get().openProject(newPath, { force: true });

    // Relocation preserves activeProjectId, so tagsStore's reload-on-change
    // subscription never fires — the tag mirror is pushed through the bridge
    // instead. Guarded on this project still being active at write time.
    if (get().activeProjectId === projectId) {
      tagsSyncBridge?.reloadTags();
    }
  },

  removeProject: (projectId: string) => {
    const project = get().projects.get(projectId);
    get().closeProject(projectId);
    if (project) {
      recentsBridge?.remove(project.projectPath);
    }
  },

  getProjectList: () => {
    const { projects, activeProjectId } = get();
    return Array.from(projects.values()).map((p) => ({
      id: p.id,
      name: basename(p.projectPath) || "Project",
      path: p.projectPath,
      isActive: p.id === activeProjectId,
      assetCount: p.scanResult?.total_count ?? null,
      issueCount: p.analysisResult?.issue_count ?? null,
      engine: p.scanResult?.project_type ?? null,
      unavailable: p.unavailable,
    }));
  },

  // Active project actions
  cancelScan: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return;
    try {
      await invoke("cancel_scan", { projectId: activeProjectId });
    } catch (err) {
      console.error("Failed to cancel scan:", err);
    }
  },

  rescan: async () => {
    const { projectPath, isScanning } = get();
    if (!projectPath || isScanning) return;
    // Drop the on-disk scan cache so even unchanged-mtime files get reclassified,
    // then re-open with force. A failed cache clear still proceeds, so the button
    // is never a dead end.
    try {
      await invoke("clear_scan_cache", { path: projectPath });
    } catch (err) {
      console.warn("Failed to clear scan cache:", err);
    }
    await get().openProject(projectPath, { force: true });
  },

  clearError: () => {
    set(updateActiveProject(get(), { error: null }));
  },

  runAnalysis: async () => {
    const startState = get();
    const startedProjectId = startState.activeProjectId;
    if (!startedProjectId) return;

    /// Snapshot the project that owns this analysis. Every subsequent write
    /// targets THIS project's entry even if the user switches mid-flight; mirror
    /// fields only sync when it is still active at write time.
    const startedProject = startState.projects.get(startedProjectId);
    if (!startedProject) return;
    /// Re-entry guard. The Sidebar button disables on `isAnalyzing`, but the
    /// command palette and menu entries do not, so the check is collapsed here.
    if (startedProject.isAnalyzing) return;

    /// Snapshot the view the user was on at kickoff. Auto-switching to "issues"
    /// on completion happens only when the started project is still active and
    /// the view is unchanged.
    const viewModeAtStart = startState.viewMode;

    /// Patch helper: writes to the started project's entry directly,
    /// only syncing mirror fields when it's still the active project.
    const patchProject = (updates: Partial<ProjectData>) => {
      const cur = get();
      const target = cur.projects.get(startedProjectId);
      if (!target) return;
      const updated = { ...target, ...updates };
      const newMap = new Map(cur.projects);
      newMap.set(startedProjectId, updated);
      const patch: Partial<ProjectState> = { projects: newMap };
      if (cur.activeProjectId === startedProjectId) {
        Object.assign(patch, syncFromActiveProject(updated));
      }
      set(patch);
    };

    patchProject({ isAnalyzing: true });

    // Re-read config at click time so users can edit `tidycraft.toml` and re-run
    // without rescanning. A missing file is normal (`null` → defaults); a file
    // that exists but cannot be read must fail the run, as the exporters do.
    let configToml: string | null = null;
    let hasCustomConfig = false;
    try {
      configToml = await invoke<string | null>("read_project_config", {
        projectId: startedProjectId,
      });
      hasCustomConfig = configToml !== null;
    } catch (err) {
      patchProject({ error: String(err), isAnalyzing: false });
      return;
    }

    try {
      const result = await invoke<AnalysisResult>("analyze_assets", {
        projectId: startedProjectId,
        configToml,
      });
      const updates: Partial<ProjectData> = {
        analysisResult: result,
        // Fresh snapshot: the new scan result carries no accumulated staleness.
        analysisStale: false,
        isAnalyzing: false,
        hasCustomConfig,
      };
      const cur = get();
      if (
        cur.activeProjectId === startedProjectId &&
        cur.viewMode === viewModeAtStart
      ) {
        updates.viewMode = "issues";
      }
      patchProject(updates);
    } catch (err) {
      console.error("Failed to analyze:", err);
      patchProject({
        error: String(err),
        isAnalyzing: false,
        hasCustomConfig,
      });
    }
  },

  pruneDuplicateGroup: (projectId: string, groupKey: string) => {
    set((state) => {
      const project = state.projects.get(projectId);
      const prev = project?.analysisResult;
      if (!project || !prev) return {};

      const issues = prev.issues.filter(
        (i) => !(i.rule_id === "duplicate" && i.related_paths?.[0] === groupKey)
      );
      if (issues.length === prev.issues.length) return {};

      // Recompute the summary wholesale from what's left — immune to any
      // assumption about how many issues the group contributed or their
      // severity, at the cost of one linear pass.
      const by_rule: Record<string, number> = {};
      let error_count = 0;
      let warning_count = 0;
      let info_count = 0;
      for (const issue of issues) {
        by_rule[issue.rule_id] = (by_rule[issue.rule_id] ?? 0) + 1;
        if (issue.severity === "error") error_count++;
        else if (issue.severity === "warning") warning_count++;
        else info_count++;
      }
      const result: AnalysisResult = {
        issues,
        issue_count: issues.length,
        error_count,
        warning_count,
        info_count,
        by_rule,
      };

      // Patch the target project directly; mirror only when it's active
      // (same discipline as every background-write path in this store).
      const projects = new Map(state.projects);
      projects.set(projectId, { ...project, analysisResult: result });
      return state.activeProjectId === projectId
        ? { projects, analysisResult: result }
        : { projects };
    });
  },

  setViewMode: (mode: ViewMode) => {
    set(updateActiveProject(get(), { viewMode: mode }));
  },

  setHasCustomConfig: (value: boolean) => {
    set(updateActiveProject(get(), { hasCustomConfig: value }));
  },

  setSelectedDirectory: (path: string | null) => {
    const normalized = normalizeDirectory(path, get().projectPath);
    set(updateActiveProject(get(), { selectedDirectory: normalized, selectedAsset: null }));
  },

  setSelectedAsset: (asset: AssetInfo | null) => {
    set(updateActiveProject(get(), { selectedAsset: asset }));
  },

  setSearchQuery: (query: string) => {
    set(updateActiveProject(get(), { searchQuery: query }));
  },

  setTypeFilter: (types: AssetType[] | null) => {
    // [] normalizes to null so "no filter" has exactly one state — an empty
    // set would otherwise read as "hide everything".
    set(updateActiveProject(get(), {
      typeFilter: types && types.length > 0 ? types : null,
    }));
  },

  toggleTypeFilter: (type: AssetType) => {
    const current = get().typeFilter ?? [];
    const next = current.includes(type)
      ? current.filter((t) => t !== type)
      : [...current, type];
    set(updateActiveProject(get(), {
      typeFilter: next.length > 0 ? next : null,
    }));
  },

  setSortField: (field: SortField) => {
    const { sortField, sortDirection } = get();
    if (sortField === field) {
      set(updateActiveProject(get(), { sortDirection: sortDirection === "asc" ? "desc" : "asc" }));
    } else {
      set(updateActiveProject(get(), { sortField: field, sortDirection: "asc" }));
    }
  },

  toggleSortDirection: () => {
    const { sortDirection } = get();
    set(updateActiveProject(get(), { sortDirection: sortDirection === "asc" ? "desc" : "asc" }));
  },

  locateAsset: (path: string) => {
    const { scanResult } = get();
    if (!scanResult) return;

    const asset = scanResult.assets.find((a) => a.path === path);
    if (!asset) return;

    set(updateActiveProject(get(), {
      viewMode: "assets",
      selectedDirectory: normalizeDirectory(dirname(path), get().projectPath),
      selectedAsset: asset,
    }));

    // "Locate" must land on a visible row: if the current filters exclude the
    // target — checked through the real pipeline — reset them. Tag filters live
    // in tagsStore and go through the registered bridge.
    if (!get().getFilteredAssets().some((a) => a.path === path)) {
      set(updateActiveProject(get(), { searchQuery: "", typeFilter: null }));
      get().resetAdvancedFilters();
    }
    tagFilterBridge?.clearIfFiltering(path);

    // Fire last so the views scroll against the settled, post-clear list.
    set({ locatePulse: get().locatePulse + 1 });
  },

  setAdvancedFilters: (filters: Partial<AdvancedFilters>) => {
    const { advancedFilters } = get();
    set(updateActiveProject(get(), { advancedFilters: { ...advancedFilters, ...filters } }));
  },

  resetAdvancedFilters: () => {
    set(updateActiveProject(get(), {
      advancedFilters: createDefaultAdvancedFilters(),
    }));
  },

  // Undo actions (scoped to the active project). `canUndo` / `undoHistory` are
  // GLOBAL mirror fields, so every write re-checks that the project the IPC round
  // trip started for is still active. A skipped write self-heals in the Header.
  undoLastOperation: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return null;
    try {
      const result = await invoke<UndoResult>("undo_last_operation", { projectId: activeProjectId });
      const canUndo = await invoke<boolean>("can_undo", { projectId: activeProjectId });
      const history = await invoke<HistoryEntry[]>("get_undo_history", { projectId: activeProjectId });
      if (get().activeProjectId === activeProjectId) {
        set({ canUndo, undoHistory: history });
      }
      return result;
    } catch (err) {
      // Console-only left the undo button looking inert; the caller gets `null`
      // and stays quiet, so the toast has to happen here.
      console.error("Failed to undo:", err);
      useToastStore.getState().push({
        kind: "error",
        message: i18n.t("header.undoFailed", { reason: String(err) }),
      });
      return null;
    }
  },

  refreshUndoState: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) {
      set({ canUndo: false, undoHistory: [] });
      return;
    }
    try {
      const canUndo = await invoke<boolean>("can_undo", { projectId: activeProjectId });
      const history = await invoke<HistoryEntry[]>("get_undo_history", { projectId: activeProjectId });
      if (get().activeProjectId === activeProjectId) {
        set({ canUndo, undoHistory: history });
      }
    } catch (err) {
      console.error("Failed to refresh undo state:", err);
    }
  },

  clearUndoHistory: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return;
    try {
      await invoke("clear_undo_history", { projectId: activeProjectId });
      if (get().activeProjectId === activeProjectId) {
        set({ canUndo: false, undoHistory: [] });
      }
    } catch (err) {
      console.error("Failed to clear undo history:", err);
    }
  },

  // Refresh gitInfo + gitStatuses for a specific project, defaulting to the
  // active one. The target id is captured once at entry and used for every call
  // and write; mirror fields update only if it is still active, as applyFsChange.
  refreshGitInfo: async (targetProjectId?: string) => {
    const initialState = get();
    const projectId = targetProjectId ?? initialState.activeProjectId;
    if (!projectId) return;

    const initialTarget = initialState.projects.get(projectId);
    if (!initialTarget) return;

    const projectPath = initialTarget.projectPath;

    const patchProject = (
      updates: Partial<Pick<ProjectData, "gitInfo" | "gitStatuses">>
    ) => {
      const cur = get();
      const t = cur.projects.get(projectId);
      // Project may have been closed mid-refresh — drop the write silently.
      if (!t) return;
      const updated = { ...t, ...updates };
      const newMap = new Map(cur.projects);
      newMap.set(projectId, updated);
      const patch: Partial<ProjectState> = { projects: newMap };
      if (cur.activeProjectId === projectId) {
        if ("gitInfo" in updates) patch.gitInfo = updates.gitInfo ?? null;
        if ("gitStatuses" in updates) patch.gitStatuses = updates.gitStatuses ?? {};
      }
      set(patch);
    };

    try {
      const gitInfo = await invoke<GitInfo>("get_git_info", {
        projectId,
        path: projectPath,
      });
      patchProject({ gitInfo });

      if (gitInfo.is_repo) {
        const response = await invoke<{ statuses: GitStatusMap }>("get_git_statuses", {
          projectId,
        });
        patchProject({ gitStatuses: response.statuses });
      } else {
        patchProject({ gitStatuses: {} });
      }
    } catch (err) {
      console.error("Failed to get git info:", err);
      patchProject({ gitInfo: null, gitStatuses: {} });
    }
  },

  // Computed
  getFilteredAssets: () => {
    const { scanResult, selectedDirectory, searchQuery, typeFilter, sortField, sortDirection, advancedFilters, gitStatuses } = get();
    if (!scanResult) return [];

    // Cheap identity check: if every input is the same reference as last call,
    // return the cached result. Setters replace values rather than mutate, so
    // this catches every real change without deep equality.
    const inputs = [
      scanResult,
      selectedDirectory,
      searchQuery,
      typeFilter,
      sortField,
      sortDirection,
      advancedFilters,
      gitStatuses,
    ] as const;
    if (
      filterCacheInputs !== null &&
      filterCacheInputs.length === inputs.length &&
      inputs.every((v, i) => Object.is(v, filterCacheInputs![i]))
    ) {
      return filterCacheResult;
    }

    let assets = [...scanResult.assets];

    // Filter by selected directory
    if (selectedDirectory) {
      assets = assets.filter((asset) => {
        const assetDir = dirname(asset.path);
        return assetDir === selectedDirectory || asset.path.startsWith(selectedDirectory + "/");
      });
    }

    // Filter by search query. Trim before matching, not just in the emptiness
    // gate: a leading or trailing space from an IME commit or paste would
    // otherwise join the needle. Interior spaces still match paths literally.
    if (searchQuery.trim()) {
      const query = searchQuery.trim().toLowerCase();
      assets = assets.filter(
        (asset) =>
          asset.name.toLowerCase().includes(query) ||
          asset.path.toLowerCase().includes(query)
      );
    }

    // Filter by asset type
    if (typeFilter) {
      const wanted = new Set(typeFilter);
      assets = assets.filter((asset) => wanted.has(asset.asset_type));
    }

    // Advanced filters
    if (advancedFilters.minSize !== null) {
      assets = assets.filter((asset) => asset.size >= advancedFilters.minSize!);
    }
    if (advancedFilters.maxSize !== null) {
      assets = assets.filter((asset) => asset.size <= advancedFilters.maxSize!);
    }
    // Metadata range filters keep an asset only when the field is present. A
    // `(value || 0)` coalesce made "max duration 10s" match every texture, script
    // and model (0 <= 10). Gate on presence first, then compare.
    if (advancedFilters.minWidth !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.width; return v != null && v >= advancedFilters.minWidth!; });
    }
    if (advancedFilters.maxWidth !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.width; return v != null && v <= advancedFilters.maxWidth!; });
    }
    if (advancedFilters.minHeight !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.height; return v != null && v >= advancedFilters.minHeight!; });
    }
    if (advancedFilters.maxHeight !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.height; return v != null && v <= advancedFilters.maxHeight!; });
    }
    if (advancedFilters.minVertices !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.vertex_count; return v != null && v >= advancedFilters.minVertices!; });
    }
    if (advancedFilters.maxVertices !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.vertex_count; return v != null && v <= advancedFilters.maxVertices!; });
    }
    if (advancedFilters.minFaces !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.face_count; return v != null && v >= advancedFilters.minFaces!; });
    }
    if (advancedFilters.maxFaces !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.face_count; return v != null && v <= advancedFilters.maxFaces!; });
    }
    if (advancedFilters.minDuration !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.duration_secs; return v != null && v >= advancedFilters.minDuration!; });
    }
    if (advancedFilters.maxDuration !== null) {
      assets = assets.filter((asset) => { const v = asset.metadata?.duration_secs; return v != null && v <= advancedFilters.maxDuration!; });
    }
    if (advancedFilters.hasAlpha !== null) {
      assets = assets.filter((asset) => asset.metadata?.has_alpha === advancedFilters.hasAlpha);
    }
    if (advancedFilters.colorSpace !== null) {
      assets = assets.filter((asset) => asset.metadata?.color_space === advancedFilters.colorSpace);
    }
    if (advancedFilters.extensions.length > 0) {
      assets = assets.filter((asset) =>
        advancedFilters.extensions.includes(asset.extension.toLowerCase())
      );
    }
    if (advancedFilters.gitStatusFilter.length > 0) {
      const wanted = new Set(advancedFilters.gitStatusFilter);
      assets = assets.filter((asset) => {
        const status = gitStatuses[asset.path];
        // Files absent from gitStatuses are unchanged — the backend only emits
        // entries for changed files — so undefined is a no-match.
        return status !== undefined && wanted.has(status);
      });
    }

    // Sort assets
    assets.sort((a, b) => {
      let comparison = 0;

      switch (sortField) {
        case "name":
          comparison = a.name.localeCompare(b.name);
          break;
        case "type":
          comparison = a.asset_type.localeCompare(b.asset_type);
          break;
        case "size":
          comparison = a.size - b.size;
          break;
        case "dimensions":
          const aDim = (a.metadata?.width || 0) * (a.metadata?.height || 0);
          const bDim = (b.metadata?.width || 0) * (b.metadata?.height || 0);
          comparison = aDim - bDim;
          break;
        case "vertices":
          comparison = (a.metadata?.vertex_count || 0) - (b.metadata?.vertex_count || 0);
          break;
        case "faces":
          comparison = (a.metadata?.face_count || 0) - (b.metadata?.face_count || 0);
          break;
        case "duration":
          comparison = (a.metadata?.duration_secs || 0) - (b.metadata?.duration_secs || 0);
          break;
        case "sampleRate":
          comparison = (a.metadata?.sample_rate || 0) - (b.metadata?.sample_rate || 0);
          break;
        case "extension":
          comparison = a.extension.localeCompare(b.extension);
          break;
      }

      return sortDirection === "asc" ? comparison : -comparison;
    });

    filterCacheInputs = inputs;
    filterCacheResult = assets;
    return assets;
  },
}));
