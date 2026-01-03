import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ScanResult, AssetInfo, ScanProgress, AssetType, AnalysisResult, UndoResult, HistoryEntry, GitInfo, GitStatusMap } from "../types/asset";

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

interface ProjectState {
  // Project data
  projectPath: string | null;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;

  // Analysis
  analysisResult: AnalysisResult | null;
  isAnalyzing: boolean;

  // UI state
  viewMode: ViewMode;
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  typeFilter: AssetType | null;
  sortField: SortField;
  sortDirection: SortDirection;
  advancedFilters: AdvancedFilters;

  // Undo state
  canUndo: boolean;
  undoHistory: HistoryEntry[];

  // Git state
  gitInfo: GitInfo | null;
  gitStatuses: GitStatusMap;

  // Actions
  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  cancelScan: () => Promise<void>;
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

export const useProjectStore = create<ProjectState>((set, get) => ({
  // Initial state
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
  canUndo: false,
  undoHistory: [],
  gitInfo: null,
  gitStatuses: {},

  // Actions
  openProject: async (path: string) => {
    set({
      isScanning: true,
      error: null,
      scanProgress: null,
      analysisResult: null,
    });

    let unlisten: UnlistenFn | null = null;

    try {
      // Listen for progress events
      unlisten = await listen<ScanProgress>("scan-progress", (event) => {
        set({ scanProgress: event.payload });
      });

      // Use incremental scan command for faster rescans
      const { result } = await invoke<{ result: ScanResult; stats: { cached_files: number; rescanned_files: number } }>(
        "scan_project_incremental",
        { path }
      );
      set({
        projectPath: path,
        scanResult: result,
        isScanning: false,
        selectedDirectory: path,
        selectedAsset: null,
        scanProgress: null,
      });
      // Fetch git info after scan completes
      get().refreshGitInfo();
    } catch (err) {
      const errorMessage = String(err);
      // Don't show error for cancelled scans
      if (!errorMessage.includes("cancelled")) {
        set({
          error: errorMessage,
          isScanning: false,
          scanProgress: null,
        });
      } else {
        set({
          isScanning: false,
          scanProgress: null,
        });
      }
    } finally {
      if (unlisten) {
        unlisten();
      }
    }
  },

  closeProject: () => {
    set({
      projectPath: null,
      scanResult: null,
      error: null,
      selectedDirectory: null,
      selectedAsset: null,
      searchQuery: "",
      typeFilter: null,
      scanProgress: null,
      analysisResult: null,
      viewMode: "assets",
    });
  },

  cancelScan: async () => {
    try {
      await invoke("cancel_scan");
    } catch (err) {
      console.error("Failed to cancel scan:", err);
    }
  },

  runAnalysis: async () => {
    set({ isAnalyzing: true });

    try {
      const result = await invoke<AnalysisResult>("analyze_assets", {
        configToml: null, // Use default config
      });
      set({
        analysisResult: result,
        isAnalyzing: false,
        viewMode: "issues",
      });
    } catch (err) {
      console.error("Failed to analyze:", err);
      set({
        error: String(err),
        isAnalyzing: false,
      });
    }
  },

  setViewMode: (mode: ViewMode) => {
    set({ viewMode: mode });
  },

  setSelectedDirectory: (path: string | null) => {
    set({ selectedDirectory: path, selectedAsset: null });
  },

  setSelectedAsset: (asset: AssetInfo | null) => {
    set({ selectedAsset: asset });
  },

  setSearchQuery: (query: string) => {
    set({ searchQuery: query });
  },

  setTypeFilter: (type: AssetType | null) => {
    set({ typeFilter: type });
  },

  setSortField: (field: SortField) => {
    const { sortField, sortDirection } = get();
    if (sortField === field) {
      // Toggle direction if same field
      set({ sortDirection: sortDirection === "asc" ? "desc" : "asc" });
    } else {
      // New field, default to ascending
      set({ sortField: field, sortDirection: "asc" });
    }
  },

  toggleSortDirection: () => {
    const { sortDirection } = get();
    set({ sortDirection: sortDirection === "asc" ? "desc" : "asc" });
  },

  locateAsset: (path: string) => {
    const { scanResult } = get();
    if (!scanResult) return;

    const asset = scanResult.assets.find((a) => a.path === path);
    if (asset) {
      // Extract directory from path
      const dir = path.substring(0, path.lastIndexOf("/"));
      set({
        viewMode: "assets",
        selectedDirectory: dir,
        selectedAsset: asset,
      });
    }
  },

  setAdvancedFilters: (filters: Partial<AdvancedFilters>) => {
    const { advancedFilters } = get();
    set({ advancedFilters: { ...advancedFilters, ...filters } });
  },

  resetAdvancedFilters: () => {
    set({
      advancedFilters: {
        minSize: null,
        maxSize: null,
        minWidth: null,
        maxWidth: null,
        minHeight: null,
        maxHeight: null,
        extensions: [],
      },
    });
  },

  // Undo actions
  undoLastOperation: async () => {
    try {
      const result = await invoke<UndoResult>("undo_last_operation");
      // Refresh undo state after operation
      const canUndo = await invoke<boolean>("can_undo");
      const history = await invoke<HistoryEntry[]>("get_undo_history");
      set({ canUndo, undoHistory: history });
      return result;
    } catch (err) {
      console.error("Failed to undo:", err);
      return null;
    }
  },

  refreshUndoState: async () => {
    try {
      const canUndo = await invoke<boolean>("can_undo");
      const history = await invoke<HistoryEntry[]>("get_undo_history");
      set({ canUndo, undoHistory: history });
    } catch (err) {
      console.error("Failed to refresh undo state:", err);
    }
  },

  clearUndoHistory: async () => {
    try {
      await invoke("clear_undo_history");
      set({ canUndo: false, undoHistory: [] });
    } catch (err) {
      console.error("Failed to clear undo history:", err);
    }
  },

  // Git actions
  refreshGitInfo: async () => {
    const { projectPath } = get();
    if (!projectPath) {
      set({ gitInfo: null, gitStatuses: {} });
      return;
    }

    try {
      const gitInfo = await invoke<GitInfo>("get_git_info", { path: projectPath });
      set({ gitInfo });

      if (gitInfo.is_repo) {
        const response = await invoke<{ statuses: GitStatusMap }>("get_git_statuses");
        set({ gitStatuses: response.statuses });
      } else {
        set({ gitStatuses: {} });
      }
    } catch (err) {
      console.error("Failed to get git info:", err);
      set({ gitInfo: null, gitStatuses: {} });
    }
  },

  // Computed
  getFilteredAssets: () => {
    const { scanResult, selectedDirectory, searchQuery, typeFilter, sortField, sortDirection, advancedFilters } = get();
    if (!scanResult) return [];

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

    return assets;
  },
}));
