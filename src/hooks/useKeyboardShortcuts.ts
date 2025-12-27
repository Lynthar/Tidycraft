import { useEffect, useCallback } from "react";
import { useProjectStore } from "../stores/projectStore";
import { open } from "@tauri-apps/plugin-dialog";

interface KeyboardShortcuts {
  onOpenFolder?: () => void;
  onFocusSearch?: () => void;
}

export function useKeyboardShortcuts({ onOpenFolder, onFocusSearch }: KeyboardShortcuts = {}) {
  const {
    projectPath,
    isScanning,
    openProject,
    cancelScan,
    runAnalysis,
    setViewMode,
    setSelectedAsset,
    setSearchQuery,
  } = useProjectStore();

  const handleOpenFolder = useCallback(async () => {
    if (isScanning) return;

    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select Project Folder",
    });

    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  }, [isScanning, openProject]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const { key, ctrlKey, metaKey, shiftKey } = event;
      const modKey = ctrlKey || metaKey;

      // Ignore if user is typing in an input
      const target = event.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") {
        // Allow Escape to blur input
        if (key === "Escape") {
          target.blur();
          event.preventDefault();
        }
        return;
      }

      // Ctrl/Cmd + O: Open folder
      if (modKey && key.toLowerCase() === "o") {
        event.preventDefault();
        if (onOpenFolder) {
          onOpenFolder();
        } else {
          handleOpenFolder();
        }
        return;
      }

      // Ctrl/Cmd + F: Focus search
      if (modKey && key.toLowerCase() === "f") {
        event.preventDefault();
        if (onFocusSearch) {
          onFocusSearch();
        }
        return;
      }

      // Ctrl/Cmd + R: Rescan (if project is open)
      if (modKey && key.toLowerCase() === "r" && !shiftKey) {
        if (projectPath && !isScanning) {
          event.preventDefault();
          openProject(projectPath);
        }
        return;
      }

      // Ctrl/Cmd + Shift + A: Run analysis
      if (modKey && shiftKey && key.toLowerCase() === "a") {
        if (projectPath && !isScanning) {
          event.preventDefault();
          runAnalysis();
        }
        return;
      }

      // Escape: Cancel scan or clear selection
      if (key === "Escape") {
        if (isScanning) {
          cancelScan();
        } else {
          setSelectedAsset(null);
          setSearchQuery("");
        }
        return;
      }

      // 1, 2, 3: Switch view modes (only when not in input)
      if (!modKey && !shiftKey) {
        if (key === "1") {
          setViewMode("assets");
          return;
        }
        if (key === "2") {
          setViewMode("issues");
          return;
        }
        if (key === "3") {
          setViewMode("stats");
          return;
        }
      }
    },
    [
      projectPath,
      isScanning,
      handleOpenFolder,
      onOpenFolder,
      onFocusSearch,
      openProject,
      cancelScan,
      runAnalysis,
      setViewMode,
      setSelectedAsset,
      setSearchQuery,
    ]
  );

  useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [handleKeyDown]);
}

// Keyboard shortcut hints for UI
export const SHORTCUTS = {
  openFolder: { key: "O", modifier: "Ctrl" },
  search: { key: "F", modifier: "Ctrl" },
  rescan: { key: "R", modifier: "Ctrl" },
  analyze: { key: "A", modifier: "Ctrl+Shift" },
  escape: { key: "Esc", modifier: "" },
  viewAssets: { key: "1", modifier: "" },
  viewIssues: { key: "2", modifier: "" },
  viewStats: { key: "3", modifier: "" },
} as const;

export function formatShortcut(shortcut: { key: string; modifier: string }): string {
  if (shortcut.modifier) {
    return `${shortcut.modifier}+${shortcut.key}`;
  }
  return shortcut.key;
}
