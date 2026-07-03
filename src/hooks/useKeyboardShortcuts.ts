import { useEffect, useCallback } from "react";
import { useProjectStore } from "../stores/projectStore";
import { useUiStore, isBlockingOverlayOpen } from "../stores/uiStore";
import { open } from "@tauri-apps/plugin-dialog";
import { getPlatform } from "../lib/platform";
import { useTranslation } from "react-i18next";

interface KeyboardShortcuts {
  onOpenFolder?: () => void;
  onFocusSearch?: () => void;
}

export function useKeyboardShortcuts({ onOpenFolder, onFocusSearch }: KeyboardShortcuts = {}) {
  const {
    projectPath,
    isScanning,
    openProject,
    rescan,
    cancelScan,
    runAnalysis,
    setViewMode,
    setSelectedAsset,
    setSearchQuery,
  } = useProjectStore();
  const { t } = useTranslation();

  const handleOpenFolder = useCallback(async () => {
    if (isScanning) return;

    const selected = await open({
      directory: true,
      multiple: false,
      title: t("header.selectProjectFolder"),
    });

    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  }, [isScanning, openProject, t]);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const { key, ctrlKey, metaKey, shiftKey } = event;
      const modKey = ctrlKey || metaKey;

      // Ctrl/Cmd + K: toggle the command palette. Handled before the
      // input-blur guard so it works from inside any text field too.
      // Only OPEN it when nothing else blocking is up: otherwise the palette
      // (z-100) stacks over an open modal (Settings / Tag Manager / the AI +
      // learning modals, z-50) and its switch/close-project commands would run
      // underneath that modal, so the modal's Apply/Save then writes to the
      // wrong project. Still allow Ctrl+K to CLOSE the palette when it's the
      // overlay that's open.
      if (modKey && key.toLowerCase() === "k") {
        const ui = useUiStore.getState();
        if (ui.cmdkOpen || !isBlockingOverlayOpen()) {
          event.preventDefault();
          ui.toggleCmdk();
        }
        return;
      }

      // While the command palette owns the keyboard, every other shortcut
      // here would compete with its own listener (Esc, ↑/↓, etc.). Bail
      // out so CommandPalette.tsx can drive navigation cleanly.
      if (useUiStore.getState().cmdkOpen) return;

      // Likewise, don't let global shortcuts (Ctrl+1/2/3, rescan, focus search,
      // Escape, …) fire underneath any other blocking modal — Settings, Tag
      // Manager, the AI / learning modals, or the dependency graph. They have
      // their own controls and the user isn't navigating the list behind them.
      if (isBlockingOverlayOpen()) return;

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

      // Ctrl/Cmd + R: Rescan (if project is open). Routes through the shared
      // `rescan` store action so it's identical to the Header button. The old
      // `openProject(projectPath)` (no force) was a no-op for the already-open
      // active project, so Ctrl+R silently did nothing.
      if (modKey && key.toLowerCase() === "r" && !shiftKey) {
        if (projectPath && !isScanning) {
          event.preventDefault();
          rescan();
        }
        return;
      }

      // Ctrl/Cmd + Shift + R: Run analysis. Note ⌘R alone is rescan; the
      // shift modifier disambiguates. Old binding was ⌘⇧A but that collides
      // with Select All in many text contexts.
      if (modKey && shiftKey && key.toLowerCase() === "r") {
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

      // Ctrl/Cmd + 1/2/3: Switch view modes. The mod key avoids stealing
      // bare digit keys from inputs and matches the design mock's labelling.
      if (modKey && !shiftKey) {
        if (key === "1") {
          event.preventDefault();
          setViewMode("assets");
          return;
        }
        if (key === "2") {
          event.preventDefault();
          setViewMode("issues");
          return;
        }
        if (key === "3") {
          event.preventDefault();
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
      rescan,
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
  analyze: { key: "R", modifier: "Ctrl+Shift" },
  commandPalette: { key: "K", modifier: "Ctrl" },
  escape: { key: "Esc", modifier: "" },
  viewAssets: { key: "1", modifier: "Ctrl" },
  viewIssues: { key: "2", modifier: "Ctrl" },
  viewStats: { key: "3", modifier: "Ctrl" },
} as const;

/// macOS Aqua HIG glyphs for modifier keys. On macOS we render shortcuts
/// glued (no `+`) per HIG; on Windows / Linux we keep the readable
/// "Ctrl+Shift+R" form. CommandPalette already hard-codes ⌘/⇧ glyphs;
/// this helper fixes the Header / Sidebar tooltips that previously
/// always printed "Ctrl+R" regardless of platform.
const MAC_MODIFIER_GLYPHS: Record<string, string> = {
  Ctrl: "⌘",
  Shift: "⇧",
  Alt: "⌥",
  Meta: "⌘",
};

export function formatShortcut(shortcut: { key: string; modifier: string }): string {
  if (!shortcut.modifier) return shortcut.key;
  if (getPlatform() === "macos") {
    const glyphs = shortcut.modifier
      .split("+")
      .map((part) => MAC_MODIFIER_GLYPHS[part] ?? part)
      .join("");
    return `${glyphs}${shortcut.key}`;
  }
  return `${shortcut.modifier}+${shortcut.key}`;
}
