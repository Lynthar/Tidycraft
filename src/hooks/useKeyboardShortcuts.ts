import { useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useProjectStore } from "../stores/projectStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore, isBlockingOverlayOpen } from "../stores/uiStore";
import { getPlatform } from "../lib/platform";
import { menuActions } from "../lib/menuActions";

export function useKeyboardShortcuts() {
  // Keystroke ROUTING lives here (which combo, in which UI state, with what
  // preventDefault); the ACTIONS live in `lib/menuActions.ts`, shared with
  // the macOS menu bar — on macOS a menu accelerator consumes the keystroke
  // before the webview sees it, so the menu path must carry the same guards
  // and the two must never drift.
  //
  // Store state is read via `getState()` at keystroke time rather than
  // subscribed to. This hook is mounted at the App root and needs no
  // rendering of its own, but destructuring the store subscribed App to
  // EVERY store change — including the scan progress that lands every 100ms,
  // re-rendering the whole tree throughout a scan and undoing the `useShallow`
  // selectors App applies for exactly that reason. Reading on demand is also
  // always fresh, so the handler needs no dependency on the values.
  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const { key, ctrlKey, metaKey, shiftKey, altKey } = event;
      const modKey = ctrlKey || metaKey;

      // F12 / Cmd+Opt+I: toggle the webview inspector. Sits above every
      // guard below — including the text-field one — because a debugging
      // tool the app blocks while a modal or an input has focus is a
      // debugging tool for exactly the moments you don't need it.
      // Matched on `event.code`, not `key`: Option+I on macOS emits the
      // dead-key "ˆ", so `key.toLowerCase() === "i"` never fires there.
      // No dev-mode branch — the backend command is a no-op in release,
      // which keeps this one binding honest across both builds (same
      // reason `main.tsx` suppresses the native menu unconditionally).
      if (key === "F12" || (modKey && altKey && event.code === "KeyI")) {
        event.preventDefault();
        invoke("toggle_devtools").catch((e) =>
          console.error("toggle_devtools failed", e)
        );
        return;
      }

      // Ctrl/Cmd + K: toggle the command palette. Handled before the
      // input-blur guard so it works from inside any text field too.
      // The open-vs-close asymmetry (never open over a blocking modal,
      // always allow closing) lives in the shared action.
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
        void menuActions.openProject();
        return;
      }

      // Ctrl/Cmd + F: Focus search
      if (modKey && key.toLowerCase() === "f") {
        event.preventDefault();
        menuActions.focusSearch();
        return;
      }

      // Ctrl/Cmd + , : Open Settings. Cmd+, is the macOS Preferences
      // convention; Ctrl+, is a common settings accelerator on Windows/Linux.
      // Sits after the input + blocking-overlay guards so a literal comma
      // typed in a field is ignored and it never opens over another modal.
      if (modKey && key === ",") {
        event.preventDefault();
        menuActions.openSettings();
        return;
      }

      // Ctrl/Cmd + R: Rescan (if project is open). Routes through the shared
      // `rescan` store action so it's identical to the Header button. The
      // keystroke is only claimed when the rescan actually ran — with no
      // project open, ⌘R keeps whatever meaning the webview gives it.
      if (modKey && key.toLowerCase() === "r" && !shiftKey) {
        if (menuActions.rescan()) {
          event.preventDefault();
        }
        return;
      }

      // Ctrl/Cmd + Shift + R: Run analysis. Note ⌘R alone is rescan; the
      // shift modifier disambiguates. Old binding was ⌘⇧A but that collides
      // with Select All in many text contexts.
      if (modKey && shiftKey && key.toLowerCase() === "r") {
        if (menuActions.runAnalysis()) {
          event.preventDefault();
        }
        return;
      }

      // Escape: dismiss exactly ONE thing per press, walking from the most
      // transient state to the most expensive to recreate. One press used to
      // clear the preview selection and the search box together — two
      // meanings in one keystroke, and it threw away a hand-typed query along
      // with a single click — while leaving the checkbox selection, the state
      // with its own action bar, untouched. Keyboard-only semantics (no menu
      // equivalent), so it stays here rather than in menuActions.
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

      // Ctrl/Cmd + 1/2/3: Switch view modes. The mod key avoids stealing
      // bare digit keys from inputs and matches the design mock's labelling.
      if (modKey && !shiftKey) {
        if (key === "1") {
          event.preventDefault();
          menuActions.setViewMode("assets");
          return;
        }
        if (key === "2") {
          event.preventDefault();
          menuActions.setViewMode("issues");
          return;
        }
        if (key === "3") {
          event.preventDefault();
          menuActions.setViewMode("stats");
          return;
        }
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
  analyze: { key: "R", modifier: "Ctrl+Shift" },
  commandPalette: { key: "K", modifier: "Ctrl" },
  settings: { key: ",", modifier: "Ctrl" },
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
