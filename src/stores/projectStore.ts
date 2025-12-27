import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ScanResult, AssetInfo, ScanProgress, AssetType, AnalysisResult } from "../types/asset";

type ViewMode = "assets" | "issues" | "stats";

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
  locateAsset: (path: string) => void;

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

  // Computed
  getFilteredAssets: () => {
    const { scanResult, selectedDirectory, searchQuery, typeFilter } = get();
    if (!scanResult) return [];

    let assets = scanResult.assets;

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

    return assets;
  },
}));
