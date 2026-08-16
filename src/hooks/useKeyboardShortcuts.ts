import { useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/projectStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, isBlockingOverlayOpen } from "../stores/uiStore";
import { getPlatform } from "../lib/platform";
import { menuActions } from "../lib/menuActions";

export function useKeyboardShortcuts() {
  // Keystroke ROUTING lives here; the ACTIONS live in `lib/menuActions.ts`, shared
  // with the macOS menu bar. Store state is read via `getState()` at keystroke
  // time rather than subscribed to, so scan progress does not re-render App.
  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const { key, ctrlKey, metaKey, shiftKey, altKey } = event;
      const modKey = ctrlKey || metaKey;

      // F12 / Cmd+Opt+I: toggle the webview inspector, above every guard below —
      // a debugging tool the app blocks under a modal is useless. Matched on
      // `event.code`, since Option+I on macOS emits the dead-key "ˆ".
      if (key === "F12" || (modKey && altKey && event.code === "KeyI")) {
        event.preventDefault();
        invoke("toggle_devtools").catch((e) =>
          console.error("toggle_devtools failed", e)
        );
        return;
      }

      // Ctrl/Cmd + K: toggle the command palette, handled before the input-blur
      // guard so it works from inside a text field. The open-vs-close asymmetry
      // lives in the shared action.
      if (modKey && key.toLowerCase() === "k") {
        if (useUiStore.getState().cmdkOpen || !isBlockingOverlayOpen()) {
          event.preventDefault();
        }
        menuActions.toggleCommandPalette();
        return;
      }

      // While the command palette owns the keyboard, every other shortcut
      // here would compete with its own listener (Esc, ↑/↓, etc.). Bail
      // out so CommandPalette.tsx can drive navigation cleanly.
      if (useUiStore.getState().cmdkOpen) return;

      // Likewise, don't let global shortcuts fire underneath any other blocking
      // modal — they have their own controls, and the user isn't navigating the
      // list behind them.
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
        void menuActions.openProject();
        return;
      }

      // Ctrl/Cmd + F: Focus search
      if (modKey && key.toLowerCase() === "f") {
        event.preventDefault();
        menuActions.focusSearch();
        return;
      }

      // Ctrl/Cmd + , : Settings. Sits after the input and blocking-overlay guards
      // so a literal comma typed in a field is ignored and it never opens over
      // another modal.
      if (modKey && key === ",") {
        event.preventDefault();
        menuActions.openSettings();
        return;
      }

      // Ctrl/Cmd + R: rescan, routed through the shared action so it matches the
      // Header button. The keystroke is claimed only when the rescan actually ran —
      // with no project open, ⌘R keeps whatever meaning the webview gives it.
      if (modKey && key.toLowerCase() === "r" && !shiftKey) {
        if (menuActions.rescan()) {
          event.preventDefault();
        }
        return;
      }

      // Escape: dismiss exactly ONE thing per press, walking from the most
      // transient state to the most expensive to recreate. Keyboard-only semantics,
      // so it stays here rather than in menuActions.
      if (key === "Escape") {
        const {
          isScanning,
          cancelScan,
          selectedAsset,
          setSelectedAsset,
          searchQuery,
          setSearchQuery,
        } = useProjectStore.getState();
        const { selectedPaths, clearSelection } = useSelectionStore.getState();
        if (isScanning) {
          cancelScan();
        } else if (selectedPaths.size > 0) {
          clearSelection();
        } else if (selectedAsset) {
          setSelectedAsset(null);
        } else if (searchQuery) {
          setSearchQuery("");
        }
        return;
      }
    },
    []
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
  commandPalette: { key: "K", modifier: "Ctrl" },
  settings: { key: ",", modifier: "Ctrl" },
  escape: { key: "Esc", modifier: "" },
} as const;

/// macOS Aqua HIG glyphs for modifier keys: shortcuts render glued (no `+`) on
/// macOS and in the readable "Ctrl+O" form elsewhere. CommandPalette hard-codes
/// its own glyphs; this covers the Header and Sidebar tooltips.
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
