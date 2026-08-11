import { Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { invoke } from "@tauri-apps/api/core";
import i18n from "../i18n";
import { isMacOS } from "./platform";
import { menuActions } from "./menuActions";
import { useRecentsStore } from "../stores/recentsStore";
import { version as appVersion } from "../../package.json";

/// The macOS menu bar, built frontend-side. JS ownership is deliberate:
/// labels come straight from i18next (a Rust-built menu would need a second
/// copy of every translation shipped over IPC), the Open Recent submenu
/// reads `recentsStore` directly, and actions are plain closures into the
/// shared `menuActions` layer — the same guards the keyboard handler uses.
///
/// macOS-only by design (`installAppMenu` no-ops elsewhere): on Windows and
/// Linux a Tauri menu renders as an in-window menu strip, which would fight
/// the custom Header. On macOS the bar is app-global and free.
///
/// Two platform rules shape the structure (verified against tauri 2.11.1):
/// - The FIRST submenu automatically becomes the application menu; its label
///   is ignored in favor of the app name.
/// - Replacing the default menu replaces the Edit roles that make ⌘C/⌘V/⌘Z
///   work inside text fields — so this menu must re-include every one of
///   them (the predefined Undo…SelectAll block below is load-bearing, not
///   decoration).
///
/// Accelerators declared here are consumed by the menu system before the
/// webview sees the keydown, so each item's `action` is the only handler
/// that runs for its combo on macOS — hence everything routes through
/// `menuActions`, never through bespoke logic.

const GITHUB_URL = "https://github.com/Lynthar/Tidycraft";
const ISSUES_URL = "https://github.com/Lynthar/Tidycraft/issues";

function openUrl(url: string): void {
  invoke("open_url", { url }).catch((e) => console.error("open_url failed", e));
}

async function buildMenu(): Promise<Menu> {
  const t = (key: string) => i18n.t(key);
  const sep = () => PredefinedMenuItem.new({ item: "Separator" });

  const appSub = await Submenu.new({
    // Label ignored on macOS for the first submenu (shows the app name).
    text: "Tidycraft",
    items: [
      await PredefinedMenuItem.new({
        item: { About: { name: "Tidycraft", version: appVersion } },
        text: t("menu.about"),
      }),
      await sep(),
      await MenuItem.new({
        text: t("menu.settings"),
        accelerator: "CmdOrCtrl+,",
        action: () => menuActions.openSettings(),
      }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Services", text: t("menu.services") }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Hide", text: t("menu.hide") }),
      await PredefinedMenuItem.new({ item: "HideOthers", text: t("menu.hideOthers") }),
      await PredefinedMenuItem.new({ item: "ShowAll", text: t("menu.showAll") }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Quit", text: t("menu.quit") }),
    ],
  });

  // Open Recent ▸ — most-recent first, straight from recentsStore. With no
  // entries it degrades to the native pattern: one disabled placeholder.
  const recents = useRecentsStore.getState().recents;
  const recentItems =
    recents.length > 0
      ? [
          ...(await Promise.all(
            recents.map((r) =>
              MenuItem.new({
                text: r.name,
                action: () => menuActions.openRecentProject(r.path),
              })
            )
          )),
          await sep(),
          await MenuItem.new({
            text: t("menu.clearRecents"),
            action: () => useRecentsStore.getState().clear(),
          }),
        ]
      : [await MenuItem.new({ text: t("menu.noRecents"), enabled: false })];

  const fileSub = await Submenu.new({
    text: t("menu.file"),
    items: [
      await MenuItem.new({
        text: t("menu.openProject"),
        accelerator: "CmdOrCtrl+O",
        action: () => void menuActions.openProject(),
      }),
      await Submenu.new({ text: t("menu.openRecent"), items: recentItems }),
      await sep(),
      await MenuItem.new({
        text: t("menu.rescan"),
        accelerator: "CmdOrCtrl+R",
        action: () => {
          menuActions.rescan();
        },
      }),
      await MenuItem.new({
        text: t("menu.runAnalysis"),
        action: () => {
          menuActions.runAnalysis();
        },
      }),
      await sep(),
      await MenuItem.new({
        text: t("menu.closeProject"),
        action: () => menuActions.closeProject(),
      }),
      await PredefinedMenuItem.new({ item: "CloseWindow", text: t("menu.closeWindow") }),
    ],
  });

  const editSub = await Submenu.new({
    text: t("menu.edit"),
    items: [
      // Load-bearing: these roles are what keep ⌘Z/⌘X/⌘C/⌘V/⌘A working in
      // every text field once the default menu is replaced.
      await PredefinedMenuItem.new({ item: "Undo", text: t("menu.undo") }),
      await PredefinedMenuItem.new({ item: "Redo", text: t("menu.redo") }),
      await sep(),
      await PredefinedMenuItem.new({ item: "Cut", text: t("menu.cut") }),
      await PredefinedMenuItem.new({ item: "Copy", text: t("menu.copy") }),
      await PredefinedMenuItem.new({ item: "Paste", text: t("menu.paste") }),
      await PredefinedMenuItem.new({ item: "SelectAll", text: t("menu.selectAll") }),
      await sep(),
      await MenuItem.new({
        text: t("menu.find"),
        accelerator: "CmdOrCtrl+F",
        action: () => menuActions.focusSearch(),
      }),
    ],
  });

  const viewSub = await Submenu.new({
    text: t("menu.view"),
    items: [
      await MenuItem.new({
        text: t("menu.viewAssets"),
        action: () => menuActions.setViewMode("assets"),
      }),
      await MenuItem.new({
        text: t("menu.viewIssues"),
        action: () => menuActions.setViewMode("issues"),
      }),
      await MenuItem.new({
        text: t("menu.viewStats"),
        action: () => menuActions.setViewMode("stats"),
      }),
      await sep(),
      await MenuItem.new({
        text: t("menu.commandPalette"),
        accelerator: "CmdOrCtrl+K",
        action: () => menuActions.toggleCommandPalette(),
      }),
    ],
  });

  const windowSub = await Submenu.new({
    text: t("menu.window"),
    items: [
      await PredefinedMenuItem.new({ item: "Minimize", text: t("menu.minimize") }),
      await PredefinedMenuItem.new({ item: "Maximize", text: t("menu.zoom") }),
      await sep(),
      await PredefinedMenuItem.new({ item: "CloseWindow", text: t("menu.closeWindow") }),
    ],
  });

  const helpSub = await Submenu.new({
    text: t("menu.help"),
    items: [
      await MenuItem.new({
        text: t("menu.helpDocs"),
        action: () => openUrl(GITHUB_URL),
      }),
      await MenuItem.new({
        text: t("menu.reportIssue"),
        action: () => openUrl(ISSUES_URL),
      }),
    ],
  });

  return Menu.new({
    items: [appSub, fileSub, editSub, viewSub, windowSub, helpSub],
  });
}

// Live reference to the installed menu: its items hold the JS action
// closures, and a menu with no reachable JS object risks its callbacks being
// collected out from under the native bar.
let installed: Menu | null = null;
let installing = false;
let rebuildQueued = false;

async function rebuild(): Promise<void> {
  // Coalesce: a rebuild landing while another is mid-flight (language change
  // during the initial install, say) queues exactly one follow-up instead of
  // racing two setAsAppMenu calls.
  if (installing) {
    rebuildQueued = true;
    return;
  }
  installing = true;
  try {
    do {
      rebuildQueued = false;
      const menu = await buildMenu();
      await menu.setAsAppMenu();
      installed = menu;
    } while (rebuildQueued);
  } catch (e) {
    // A menu that failed to install leaves the previous one (or Tauri's
    // default) in place — log it, the app itself is unaffected.
    console.error("[appMenu] failed to install app menu", e);
  } finally {
    installing = false;
  }
}

/// Install the macOS menu bar and keep it current. No-op on other platforms.
/// Called once from main.tsx at boot.
export function installAppMenu(): void {
  if (!isMacOS()) return;

  void rebuild();

  // Full rebuild on either trigger: both are rare, and swapping the whole
  // menu is simpler and safer than reaching into live native items.
  i18n.on("languageChanged", () => void rebuild());
  useRecentsStore.subscribe((state, prev) => {
    if (state.recents !== prev.recents) void rebuild();
  });
}

/// Test-only view of the installed-menu reference (no test runner exists in
/// this repo today; kept for the CDP smoke probe, which asserts the non-mac
/// no-op by checking this stays null).
export function installedMenuForProbe(): Menu | null {
  return installed;
}
