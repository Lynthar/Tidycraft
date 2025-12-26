import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { ScanResult, AssetInfo, ScanProgress, AssetType } from "../types/asset";

interface ProjectState {
  // Project data
  projectPath: string | null;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;
  scanProgress: ScanProgress | null;

  // UI state
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;
  searchQuery: string;
  typeFilter: AssetType | null;

  // Actions
  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  cancelScan: () => Promise<void>;
  setSelectedDirectory: (path: string | null) => void;
  setSelectedAsset: (asset: AssetInfo | null) => void;
  setSearchQuery: (query: string) => void;
  setTypeFilter: (type: AssetType | null) => void;

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
  selectedDirectory: null,
  selectedAsset: null,
  searchQuery: "",
  typeFilter: null,

  // Actions
  openProject: async (path: string) => {
    set({ isScanning: true, error: null, scanProgress: null });

    let unlisten: UnlistenFn | null = null;

    try {
      // Listen for progress events
      unlisten = await listen<ScanProgress>("scan-progress", (event) => {
        set({ scanProgress: event.payload });
      });

      // Use async scan command
      const result = await invoke<ScanResult>("scan_project_async", { path });
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
    });
  },

  cancelScan: async () => {
    try {
      await invoke("cancel_scan");
    } catch (err) {
      console.error("Failed to cancel scan:", err);
    }
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
