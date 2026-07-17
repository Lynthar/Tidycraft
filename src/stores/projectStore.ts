import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { basename, dirname } from "../lib/pathUtils";
import type { ScanResult, AssetInfo, ScanProgress, AssetType, ProjectType, AnalysisResult, UndoResult, HistoryEntry, GitInfo, GitStatusMap, GitFileStatus, FsChangeEvent, DirectoryNode } from "../types/asset";
import { useSettingsStore } from "./settingsStore";
import { evictThumbs } from "../lib/thumbnailCache";

// Per-project filesystem-watcher unlisten handles. Kept outside the zustand
// store because function references don't belong in serialized state, and
// we need to dispose them on closeProject.
const fsWatchers = new Map<string, UnlistenFn>();

async function stopFsWatch(projectId: string) {
  const unlisten = fsWatchers.get(projectId);
  if (unlisten) {
    unlisten();
    fsWatchers.delete(projectId);
  }
  try {
    await invoke("stop_watching", { projectId });
  } catch (err) {
    console.error("Failed to stop watcher:", err);
  }
}

// Per-project debounced git-refresh timers. Each fs-change event resets the
// timer for that project; on quiescence we re-fetch git info + statuses so
// the branch chip and file badges don't drift from reality. The window sits
// above the watcher's own 500ms coalescing in `watcher.rs` — together they
// absorb bursts (batch rename, checkout, large copy) into a single refresh.
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

// Store-level memoization cache for getFilteredAssets. Shared across all
// components (AssetList, StatusBar, …) so they don't independently re-run
// the filter+sort on 10k+ assets. Invalidated automatically on any input
// identity change — scanResult, filters, and sort state are all replaced
// (not mutated) by their setters, so reference equality is a correct check.
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

// Data for a single project
export interface ProjectData {
  id: string;
  projectPath: string;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;
  analysisResult: AnalysisResult | null;
  /// True when files changed (watcher event or rescan) AFTER analysisResult
  /// was computed — the analysis is a point-in-time snapshot and is now
  /// showing pre-change data. IssueList surfaces this as a "re-run" banner,
  /// and passCount math treats the issue set as potentially referencing
  /// deleted files. Cleared by the next successful runAnalysis.
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
  advancedFilters: {
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
  },
  gitInfo: null,
  gitStatuses: {},
  hasCustomConfig: false,
});

const generateProjectId = (): string => {
  return `project_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
};

// Bridge registered by tagsStore at its module init. locateAsset needs to
// clear tag filters that would hide the located asset, but tag state lives
// in tagsStore — and tagsStore already imports this store (its top-level
// `useProjectStore.subscribe(...)` runs at module evaluation), so a static
// import from here would close an ESM cycle and TDZ-crash at startup.
interface TagFilterBridge {
  /// Clear the active tag filters if (and only if) they would exclude
  /// `path` from the asset list.
  clearIfFiltering: (path: string) => void;
}
let tagFilterBridge: TagFilterBridge | null = null;
export const registerTagFilterBridge = (bridge: TagFilterBridge) => {
  tagFilterBridge = bridge;
};

// True when a project entry is an unhydrated stub — registered (usually by
// session restore) but never scanned — that should be lazily hydrated the
// moment it becomes the active project. Deliberately false when `error` is
// set: auto-retrying a permanent failure (path gone, permission denied)
// would loop the user through "switch → error → switch back → error"
// forever; the Header rescan button remains the manual retry path.
// Shared by setActiveProject and closeProject so every code path that can
// promote a project to active applies the same hydration rule.
const needsHydration = (project: ProjectData): boolean =>
  project.scanResult === null && !project.isScanning && !project.error;

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

  /// Monotonic counter bumped by every locateAsset call. The asset views
  /// scroll the selected row into view on [selectedAsset.path, locatePulse]
  /// — the pulse makes a repeat locate of the already-selected asset still
  /// scroll, without making every ordinary filter change fight the user's
  /// scroll position.
  locatePulse: number;

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

  // Multi-project actions
  openProject: (path: string, options?: { force?: boolean }) => Promise<void>;
  /// Register a project with the backend and add a stub `ProjectData` to
  /// the projects Map without triggering a scan. Used by session restore
  /// so non-active projects appear in the sidebar instantly; their full
  /// hydration runs lazily when the user switches to them. Idempotent —
  /// calling this for a path that's already in the Map is a no-op.
  registerProjectStub: (rawPath: string) => Promise<void>;
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
  }[];

  // Active project actions
  cancelScan: () => Promise<void>;
  /// Cache-clearing force rescan, shared by the Header rescan button and the
  /// Ctrl+R shortcut (the button's tooltip advertises Ctrl+R, so the two must
  /// behave identically). No-op without an active project or while scanning.
  rescan: () => Promise<void>;
  clearError: () => void;
  runAnalysis: () => Promise<void>;
  /// Optimistic prune after the duplicate-group cleanup trashed all but one
  /// copy: drop that group's issue from `projectId`'s analysisResult so the
  /// card disappears immediately (counts recomputed wholesale). `groupKey`
  /// is the group's first `related_paths` member (root-relative), the same
  /// identity the issue list collapses on. Deliberately does NOT touch
  /// `analysisStale` — the deletions make OTHER issues stale too, and the
  /// watcher-driven banner is the source of truth for that.
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
      advancedFilters: {
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
      },
      gitInfo: null,
      gitStatuses: {},
      hasCustomConfig: false,
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
  };
};

// True when `path` names a directory node anywhere in `tree`. Used to keep
// `selectedDirectory` honest against a fresh directory tree — filtering the
// asset list by a directory that no longer exists yields a blank view with
// no explanation.
function directoryExistsInTree(tree: DirectoryNode, path: string): boolean {
  if (tree.path === path) return true;
  return tree.children.some((child) => directoryExistsInTree(child, path));
}

// Apply a filesystem-change event from the backend watcher into the store.
// Runs outside any specific store action — targets the project the event was
// emitted for, even if the user has switched away. See openProject's scan
// handler for the same pattern.
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

  // Reconcile selectedAsset: swap to the fresh copy if it was re-parsed, or
  // drop it if the file was deleted.
  let newSelectedAsset = target.selectedAsset;
  if (newSelectedAsset) {
    if (event.removed.includes(newSelectedAsset.path)) {
      newSelectedAsset = null;
    } else {
      const fresh = event.updated.find((a) => a.path === newSelectedAsset!.path);
      if (fresh) newSelectedAsset = fresh;
    }
  }

  // Reconcile selectedDirectory: an external delete of the selected folder
  // used to leave the list filtering against a ghost path — blank view, no
  // explanation, and clicking the (gone) tree node couldn't fix it.
  let newSelectedDirectory = target.selectedDirectory;
  if (
    newSelectedDirectory &&
    !directoryExistsInTree(event.directory_tree, newSelectedDirectory)
  ) {
    newSelectedDirectory = newScanResult.root_path;
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
  }
  useProjectStore.setState(patch);

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
  advancedFilters: {
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
  },
  gitInfo: null,
  gitStatuses: {},
  hasCustomConfig: false,

  // Multi-project actions
  openProject: async (rawPath: string, options?: { force?: boolean }) => {
    const { projects } = get();

    // Normalize path separators. The Tauri dialog returns OS-native paths
    // (backslashes on Windows) but everything downstream — the scanner,
    // `selectedDirectory` filtering, `convertFileSrc`, tree navigation —
    // expects forward slashes. Backend does the same for scan result paths.
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

    // Reuse the existing projectId on force-rescan so the backend's
    // ProjectState (undo history, watcher, tags, git manager) survives.
    const projectId = existingProject?.id ?? generateProjectId();

    // Register with the backend BEFORE flipping activeProjectId, so that
    // subscribers like tagsStore (which re-load on activeProjectId change)
    // don't race an unregistered project into their invoke calls. The backend
    // registry is idempotent — calling register again on an existing project
    // is a no-op.
    try {
      await invoke("register_project", { projectId, path });
    } catch (err) {
      console.error("Failed to register project:", err);
      return;
    }

    // Concurrent double-open guard: a second openProject for the same path
    // that started while our register_project was in flight may have written
    // its Map entry already (both calls snapshot `projects` BEFORE any
    // await). Adopt the winner instead of inserting a duplicate row; our
    // freshly generated backend registration is dropped best-effort.
    if (!existingProject) {
      const winner = Array.from(get().projects.values()).find(
        (p) => p.projectPath === path
      );
      if (winner) {
        void invoke("unregister_project", { projectId }).catch(() => {});
        get().setActiveProject(winner.id);
        return;
      }
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
        }
      : { ...createDefaultProjectData(projectId, path), isScanning: true };

    const newProjects = new Map(projects);
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

      // Apply scan result to the project that owns it (not necessarily active).
      const state = get();
      const target = state.projects.get(projectId);
      if (target) {
        // Force-rescan promises to keep UI state; honor it for the directory
        // selection too — the old unconditional reset to root kicked the user
        // out of the folder they were working in on every Ctrl+R. Falls back
        // to `null` (= whole project) when the directory vanished between
        // scans — NOT the root path: "scoped to root" and "no scope" must be
        // one state, or the tree highlight, the scope bar, and the type-pill
        // counts all disagree about whether a scope is active.
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
        };
        const newMap = new Map(state.projects);
        newMap.set(projectId, updated);
        const patch: Partial<ProjectState> = { projects: newMap };
        if (state.activeProjectId === projectId) {
          Object.assign(patch, syncFromActiveProject(updated));
        }
        set(patch);
      }

      // Refresh git info for this project specifically — refreshGitInfo
      // patches the right entry in the projects Map regardless of which
      // project is currently active, so we don't need an "is still active"
      // guard here.
      get().refreshGitInfo(projectId);

      // Start the filesystem watcher now that the cache is populated.
      // Events that arrive before the scan completes would be no-ops on the
      // backend (no cached_scan to patch), so ordering matters.
      // On force-rescan, the watcher is already running from the original
      // open — skip to avoid stacking duplicate listeners / watch handles.
      if (!fsWatchers.has(projectId)) {
        try {
          const fsUnlisten = await listen<FsChangeEvent>(
            `fs-change-${projectId}`,
            (event) => applyFsChange(projectId, event.payload)
          );
          fsWatchers.set(projectId, fsUnlisten);
          await invoke("start_watching", { projectId });
        } catch (err) {
          console.error("Failed to start watcher:", err);
          await stopFsWatch(projectId);
        }
      }
    } catch (err) {
      // A concurrent scan already owns this project's scan state (the backend
      // rejected this one); let that scan finish and drive isScanning /
      // scanResult — don't clobber it here. The finally block still unlistens.
      if (String(err).includes("already in progress")) return;
      const errorMessage = String(err);
      const state = get();
      const target = state.projects.get(projectId);
      if (!target) return;
      const isCancelled = errorMessage.includes("cancelled");
      const updated = {
        ...target,
        isScanning: false,
        scanProgress: null,
        error: isCancelled ? null : errorMessage,
      };
      const newMap = new Map(state.projects);
      newMap.set(projectId, updated);
      const patch: Partial<ProjectState> = { projects: newMap };
      if (state.activeProjectId === projectId) {
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

    // Closing the active project promotes the next one directly (bypassing
    // setActiveProject), so apply the same lazy-hydration rule here: a
    // promoted session-restored stub would otherwise render a permanently
    // blank asset view — clicking its row hits setActiveProject's same-id
    // early return, leaving a force rescan as the only way out.
    if (idToClose === activeProjectId && activeProject && needsHydration(activeProject)) {
      void get().openProject(activeProject.projectPath, { force: true });
    }
  },

  setActiveProject: (projectId: string) => {
    const { projects, activeProjectId } = get();
    if (projectId === activeProjectId) return;
    const project = projects.get(projectId);
    if (!project) return;

    // Lazy hydration: if this is a stub (see needsHydration), kick off a
    // full openProject. openProject's force=true path will replace the
    // stub's ProjectData with isScanning=true, set activeProjectId,
    // wire scan-progress and fs-change listeners, run the scan, and
    // start the watcher + refresh git on completion.
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
    // Drop the on-disk scan cache so even files whose mtime didn't change get
    // reclassified (e.g. after a format gained a new AssetType), then re-open
    // with force. Best-effort cache clear — a failure still proceeds to the
    // force rescan so the button / shortcut is never a dead end.
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

    /// Snapshot the project that owns this analysis. All subsequent
    /// state writes target THIS project's entry in the Map even if the
    /// user switches active project mid-flight — same pattern as
    /// openProject's scan handler. Mirror fields only sync when the
    /// started project is still active at write time.
    const startedProject = startState.projects.get(startedProjectId);
    if (!startedProject) return;
    /// Re-entry guard. UI surfaces this via `disabled={isAnalyzing}` on
    /// the Sidebar button, but Ctrl+Shift+R and the CommandPalette entry
    /// don't gate on it — collapse the check here so every entry path
    /// is safe.
    if (startedProject.isAnalyzing) return;

    /// Snapshot the view the user was on when they kicked off analysis.
    /// If they navigate elsewhere mid-flight, leave them where they went —
    /// silently yanking them back to "issues" feels intrusive. We auto-
    /// switch only when the started project is still active AND the
    /// active view is unchanged at completion.
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

    // Re-read config at click time (not just at scan-complete) so users
    // can edit `tidycraft.toml` and re-run without rescanning. IO failure
    // falls back to defaults silently — `analyze_assets` will surface
    // toml *parse* errors via the normal error path.
    let configToml: string | null = null;
    let hasCustomConfig = false;
    try {
      configToml = await invoke<string | null>("read_project_config", {
        projectId: startedProjectId,
      });
      hasCustomConfig = configToml !== null;
    } catch (err) {
      console.warn("Failed to read tidycraft.toml; using defaults:", err);
    }

    try {
      const result = await invoke<AnalysisResult>("analyze_assets", {
        projectId: startedProjectId,
        configToml,
      });
      const updates: Partial<ProjectData> = {
        analysisResult: result,
        // Fresh snapshot: whatever staleness the previous one accumulated
        // is history now.
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
    // Normalize the project root to null: filtering by the root is identical
    // to no filter, and letting both representations exist is exactly what
    // desynced the tree highlight from the real scope.
    const normalized = path && path === get().projectPath ? null : path;
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

    const dir = dirname(path);
    set(updateActiveProject(get(), {
      viewMode: "assets",
      selectedDirectory: dir,
      selectedAsset: asset,
    }));

    // "Locate" must actually land on a visible row. If the current filters
    // exclude the target (checked through the real pipeline, not a
    // re-implementation of it), reset them; when they don't, leave the
    // user's filter context alone. Tag filters live in tagsStore and are
    // handled through the registered bridge (import-cycle constraint, see
    // registerTagFilterBridge).
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
      advancedFilters: {
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
      },
    }));
  },

  // Undo actions (scoped to active project)
  undoLastOperation: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return null;
    try {
      const result = await invoke<UndoResult>("undo_last_operation", { projectId: activeProjectId });
      const canUndo = await invoke<boolean>("can_undo", { projectId: activeProjectId });
      const history = await invoke<HistoryEntry[]>("get_undo_history", { projectId: activeProjectId });
      set({ canUndo, undoHistory: history });
      return result;
    } catch (err) {
      console.error("Failed to undo:", err);
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
      set({ canUndo, undoHistory: history });
    } catch (err) {
      console.error("Failed to refresh undo state:", err);
    }
  },

  clearUndoHistory: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return;
    try {
      await invoke("clear_undo_history", { projectId: activeProjectId });
      set({ canUndo: false, undoHistory: [] });
    } catch (err) {
      console.error("Failed to clear undo history:", err);
    }
  },

  // Git actions
  //
  // Refreshes the gitInfo + gitStatuses for a specific project. If
  // `targetProjectId` is omitted, uses the currently active project.
  //
  // Race-safe: the target project id is captured once at entry and used for
  // every backend call and every store write. Without this, async awaits
  // between getting state and setting it could let the user switch projects
  // mid-flight, causing project A's git data to be written to project B.
  //
  // The patch goes directly into the projects Map for the target id;
  // convenience mirror fields are only updated if the target is still the
  // active project at write time. Same pattern as `applyFsChange`.
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

    // Cheap identity check: if every input is the same reference as the
    // previous call, return the cached result. Because setters replace
    // values (new scanResult objects on fs-change, new advancedFilters
    // objects on setAdvancedFilters, etc.) this catches every real change
    // without needing deep equality. `gitStatuses` is in here because the
    // gitStatusFilter branch below reads from it; refreshGitInfo replaces
    // the whole map by reference, so identity tracks freshness correctly.
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

    // Filter by search query
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
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
    // Metadata range filters: an asset is kept only when the field is actually
    // present. The old `(value || 0)` coalesced a missing field to 0, so e.g.
    // "max duration 10s" matched every texture/script/model (0 <= 10) — assets
    // that have no duration at all. Gate on presence first, then compare.
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
        // Files not in gitStatuses are unchanged (the backend only emits
        // entries for changed files), so they only match if "unchanged" is
        // explicitly wanted — which the UI doesn't expose, so effectively
        // never. Treat undefined as a no-match.
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
