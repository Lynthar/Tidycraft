import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { ScanResult, AssetInfo } from "../types/asset";

interface ProjectState {
  // Project data
  projectPath: string | null;
  scanResult: ScanResult | null;
  isScanning: boolean;
  error: string | null;

  // UI state
  selectedDirectory: string | null;
  selectedAsset: AssetInfo | null;

  // Actions
  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  setSelectedDirectory: (path: string | null) => void;
  setSelectedAsset: (asset: AssetInfo | null) => void;

  // Computed
  getFilteredAssets: () => AssetInfo[];
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  // Initial state
  projectPath: null,
  scanResult: null,
  isScanning: false,
  error: null,
  selectedDirectory: null,
  selectedAsset: null,

  // Actions
  openProject: async (path: string) => {
    set({ isScanning: true, error: null });

    try {
      const result = await invoke<ScanResult>("scan_project", { path });
      set({
        projectPath: path,
        scanResult: result,
        isScanning: false,
        selectedDirectory: path,
        selectedAsset: null,
      });
    } catch (err) {
      set({
        error: String(err),
        isScanning: false,
      });
    }
  },

  closeProject: () => {
    set({
      projectPath: null,
      scanResult: null,
      error: null,
      selectedDirectory: null,
      selectedAsset: null,
    });
  },

  setSelectedDirectory: (path: string | null) => {
    set({ selectedDirectory: path, selectedAsset: null });
  },

  setSelectedAsset: (asset: AssetInfo | null) => {
    set({ selectedAsset: asset });
  },

  // Computed
  getFilteredAssets: () => {
    const { scanResult, selectedDirectory } = get();
    if (!scanResult) return [];

    if (!selectedDirectory) {
      return scanResult.assets;
    }

    // Filter assets by selected directory
    return scanResult.assets.filter((asset) => {
      const assetDir = asset.path.substring(0, asset.path.lastIndexOf("/"));
      return assetDir === selectedDirectory || asset.path.startsWith(selectedDirectory + "/");
    });
  },
}));
