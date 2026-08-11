import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/projectStore";
import { useUiStore, isBlockingOverlayOpen } from "../stores/uiStore";
import i18n from "../i18n";

/// App-level actions reachable from more than one surface — the keyboard
/// handler (`useKeyboardShortcuts`) and the macOS menu bar (`appMenu.ts`)
/// both route through here, guards included, so the two can never drift.
///
/// The guards are NOT redundant with the keyboard hook's own checks: on
/// macOS, an accelerator declared on a menu item is consumed by the menu
/// system before the webview ever sees the keydown, so the menu action is
/// the ONLY code that runs for those combos there. If the blocking-overlay
/// guard lived solely in the keydown handler, ⌘K from the menu would open
/// the palette over a modal and its project-switching commands would write
/// to the wrong project — a bug this codebase has already fixed once.

/// Global actions are frozen while the command palette owns the keyboard or
/// a blocking modal is up — same rule the keydown handler applies.
function globalActionsFrozen(): boolean {
  return useUiStore.getState().cmdkOpen || isBlockingOverlayOpen();
}

// The search input lives in Header; App registers a focuser at mount so this
// module (and the menu built from it) can reach it without prop drilling.
let searchFocuser: (() => void) | null = null;
export function registerSearchFocus(fn: (() => void) | null) {
  searchFocuser = fn;
}

// Same arrangement for the asset list, which the search box hands focus to on
// a down arrow. The list registers itself rather than App registering it: the
// slot is then empty exactly when there is no list to focus — the grid view,
// no project open, a scan still running — and the caller can tell.
let assetListFocuser: (() => void) | null = null;
export function registerAssetListFocus(fn: (() => void) | null) {
  assetListFocuser = fn;
}

/// Move keyboard focus into the asset list, reporting whether there was one to
/// move it into. Callers use the answer to decide whether to claim the
/// keystroke — the same did-run contract `rescan` has.
export function focusAssetList(): boolean {
  if (!assetListFocuser) return false;
  assetListFocuser();
  return true;
}

export const menuActions = {
  /// Folder picker → openProject. No-op while a scan is running (opening
  /// mid-scan would race the active project) or while an overlay is up.
  openProject: async () => {
    if (globalActionsFrozen()) return;
    if (useProjectStore.getState().isScanning) return;

    const selected = await open({
      directory: true,
      multiple: false,
      title: i18n.t("header.selectProjectFolder"),
    });

    if (selected && typeof selected === "string") {
      void useProjectStore.getState().openProject(selected);
    }
  },

  openRecentProject: (path: string) => {
    if (globalActionsFrozen()) return;
    if (useProjectStore.getState().isScanning) return;
    void useProjectStore.getState().openProject(path);
  },

  closeProject: () => {
    if (globalActionsFrozen()) return;
    useProjectStore.getState().closeProject();
  },

  /// Returns whether it actually ran — the keyboard handler only
  /// preventDefault()s the keystroke when it did (with no project open,
  /// ⌘R must keep whatever meaning the webview gives it, exactly as before).
  rescan: (): boolean => {
    if (globalActionsFrozen()) return false;
    const s = useProjectStore.getState();
    if (!s.projectPath || s.isScanning) return false;
    s.rescan();
    return true;
  },

  /// Same did-run contract as `rescan` — see there.
  runAnalysis: (): boolean => {
    if (globalActionsFrozen()) return false;
    const s = useProjectStore.getState();
    if (!s.projectPath || s.isScanning) return false;
    void s.runAnalysis();
    return true;
  },

  focusSearch: () => {
    if (globalActionsFrozen()) return;
    searchFocuser?.();
  },

  openSettings: () => {
    if (globalActionsFrozen()) return;
    useUiStore.getState().setSettingsOpen(true);
  },

  setViewMode: (mode: "assets" | "issues" | "stats") => {
    if (globalActionsFrozen()) return;
    useProjectStore.getState().setViewMode(mode);
  },

  /// ⌘K keeps its own asymmetric guard: opening is blocked under a modal
  /// (the palette would stack above it and its commands would run against
  /// the wrong project), but CLOSING an already-open palette must always
  /// work — including via the same shortcut that opened it.
  toggleCommandPalette: () => {
    const ui = useUiStore.getState();
    if (ui.cmdkOpen || !isBlockingOverlayOpen()) {
      ui.toggleCmdk();
    }
  },
};
