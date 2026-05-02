import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ScanResult, AssetInfo, ScanProgress, AssetType, ProjectType, AnalysisResult, UndoResult, HistoryEntry, GitInfo, GitStatusMap, FsChangeEvent } from "../types/asset";

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
  extensions: string[];
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
  isAnalyzing: boolean;
  viewMode: ViewMode;
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  typeFilter: AssetType | null;
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
    extensions: [],
  },
  gitInfo: null,
  gitStatuses: {},
  hasCustomConfig: false,
});

const generateProjectId = (): string => {
  return `project_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
};

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

  // Convenience getters for active project
  projectPath: string | null;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;
  analysisResult: AnalysisResult | null;
  isAnalyzing: boolean;
  viewMode: ViewMode;
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  typeFilter: AssetType | null;
  sortField: SortField;
  sortDirection: SortDirection;
  advancedFilters: AdvancedFilters;
  gitInfo: GitInfo | null;
  gitStatuses: GitStatusMap;
  hasCustomConfig: boolean;

  // Multi-project actions
  openProject: (path: string, options?: { force?: boolean }) => Promise<void>;
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
  clearError: () => void;
  runAnalysis: () => Promise<void>;
  setViewMode: (mode: ViewMode) => void;
  setSelectedDirectory: (path: string | null) => void;
  setSelectedAsset: (asset: AssetInfo | null) => void;
  setSearchQuery: (query: string) => void;
  setTypeFilter: (type: AssetType | null) => void;
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
  refreshGitInfo: () => Promise<void>;

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
        extensions: [],
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

  const updated: ProjectData = {
    ...target,
    scanResult: newScanResult,
    selectedAsset: newSelectedAsset,
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
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  // Multi-project initial state
  projects: new Map(),
  activeProjectId: null,

  // Global state
  canUndo: false,
  undoHistory: [],
  watcherPulse: 0,

  // Initial convenience fields (no active project)
  projectPath: null,
  scanResult: null,
  isScanning: false,
  error: null,
  scanProgress: null,
  analysisResult: null,
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
    extensions: [],
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

      // Use incremental scan command
      const { result } = await invoke<{ result: ScanResult; stats: { cached_files: number; rescanned_files: number } }>(
        "scan_project_incremental",
        { projectId, path }
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
        const updated = {
          ...target,
          scanResult: result,
          isScanning: false,
          selectedDirectory: path,
          selectedAsset: null,
          scanProgress: null,
          hasCustomConfig,
        };
        const newMap = new Map(state.projects);
        newMap.set(projectId, updated);
        const patch: Partial<ProjectState> = { projects: newMap };
        if (state.activeProjectId === projectId) {
          Object.assign(patch, syncFromActiveProject(updated));
        }
        set(patch);
      }

      // Fetch git info if this is still the active project.
      if (get().activeProjectId === projectId) {
        get().refreshGitInfo();
      }

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
  },

  setActiveProject: (projectId: string) => {
    const { projects } = get();
    const project = projects.get(projectId);

    if (project) {
      set({
        activeProjectId: projectId,
        ...syncFromActiveProject(project),
      });
    }
  },

  getProjectList: () => {
    const { projects, activeProjectId } = get();
    return Array.from(projects.values()).map((p) => ({
      id: p.id,
      name:
        p.projectPath.split("/").pop() ||
        p.projectPath.split("\\").pop() ||
        "Project",
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

  clearError: () => {
    set(updateActiveProject(get(), { error: null }));
  },

  runAnalysis: async () => {
    const { activeProjectId } = get();
    if (!activeProjectId) return;
    set(updateActiveProject(get(), { isAnalyzing: true }));

    // Re-read config at click time (not just at scan-complete) so users
    // can edit `tidycraft.toml` and re-run without rescanning. IO failure
    // falls back to defaults silently — `analyze_assets` will surface
    // toml *parse* errors via the normal error path.
    let configToml: string | null = null;
    let hasCustomConfig = false;
    try {
      configToml = await invoke<string | null>("read_project_config", {
        projectId: activeProjectId,
      });
      hasCustomConfig = configToml !== null;
    } catch (err) {
      console.warn("Failed to read tidycraft.toml; using defaults:", err);
    }

    try {
      const result = await invoke<AnalysisResult>("analyze_assets", {
        projectId: activeProjectId,
        configToml,
      });
      set(updateActiveProject(get(), {
        analysisResult: result,
        isAnalyzing: false,
        viewMode: "issues",
        hasCustomConfig,
      }));
    } catch (err) {
      console.error("Failed to analyze:", err);
      set(updateActiveProject(get(), {
        error: String(err),
        isAnalyzing: false,
        hasCustomConfig,
      }));
    }
  },

  setViewMode: (mode: ViewMode) => {
    set(updateActiveProject(get(), { viewMode: mode }));
  },

  setSelectedDirectory: (path: string | null) => {
    set(updateActiveProject(get(), { selectedDirectory: path, selectedAsset: null }));
  },

  setSelectedAsset: (asset: AssetInfo | null) => {
    set(updateActiveProject(get(), { selectedAsset: asset }));
  },

  setSearchQuery: (query: string) => {
    set(updateActiveProject(get(), { searchQuery: query }));
  },

  setTypeFilter: (type: AssetType | null) => {
    set(updateActiveProject(get(), { typeFilter: type }));
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
    if (asset) {
      const dir = path.substring(0, path.lastIndexOf("/"));
      set(updateActiveProject(get(), {
        viewMode: "assets",
        selectedDirectory: dir,
        selectedAsset: asset,
      }));
    }
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
        extensions: [],
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
  refreshGitInfo: async () => {
    const { activeProjectId, projectPath } = get();
    if (!activeProjectId || !projectPath) {
      set(updateActiveProject(get(), { gitInfo: null, gitStatuses: {} }));
      return;
    }

    try {
      const gitInfo = await invoke<GitInfo>("get_git_info", {
        projectId: activeProjectId,
        path: projectPath,
      });
      set(updateActiveProject(get(), { gitInfo }));

      if (gitInfo.is_repo) {
        const response = await invoke<{ statuses: GitStatusMap }>("get_git_statuses", {
          projectId: activeProjectId,
        });
        set(updateActiveProject(get(), { gitStatuses: response.statuses }));
      } else {
        set(updateActiveProject(get(), { gitStatuses: {} }));
      }
    } catch (err) {
      console.error("Failed to get git info:", err);
      set(updateActiveProject(get(), { gitInfo: null, gitStatuses: {} }));
    }
  },

  // Computed
  getFilteredAssets: () => {
    const { scanResult, selectedDirectory, searchQuery, typeFilter, sortField, sortDirection, advancedFilters } = get();
    if (!scanResult) return [];

    // Cheap identity check: if every input is the same reference as the
    // previous call, return the cached result. Because setters replace
    // values (new scanResult objects on fs-change, new advancedFilters
    // objects on setAdvancedFilters, etc.) this catches every real change
    // without needing deep equality.
    const inputs = [
      scanResult,
      selectedDirectory,
      searchQuery,
      typeFilter,
      sortField,
      sortDirection,
      advancedFilters,
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
        const assetDir = asset.path.substring(0, asset.path.lastIndexOf("/"));
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
      assets = assets.filter((asset) => asset.asset_type === typeFilter);
    }

    // Advanced filters
    if (advancedFilters.minSize !== null) {
      assets = assets.filter((asset) => asset.size >= advancedFilters.minSize!);
    }
    if (advancedFilters.maxSize !== null) {
      assets = assets.filter((asset) => asset.size <= advancedFilters.maxSize!);
    }
    if (advancedFilters.minWidth !== null) {
      assets = assets.filter((asset) => (asset.metadata?.width || 0) >= advancedFilters.minWidth!);
    }
    if (advancedFilters.maxWidth !== null) {
      assets = assets.filter((asset) => (asset.metadata?.width || 0) <= advancedFilters.maxWidth!);
    }
    if (advancedFilters.minHeight !== null) {
      assets = assets.filter((asset) => (asset.metadata?.height || 0) >= advancedFilters.minHeight!);
    }
    if (advancedFilters.maxHeight !== null) {
      assets = assets.filter((asset) => (asset.metadata?.height || 0) <= advancedFilters.maxHeight!);
    }
    if (advancedFilters.extensions.length > 0) {
      assets = assets.filter((asset) =>
        advancedFilters.extensions.includes(asset.extension.toLowerCase())
      );
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
