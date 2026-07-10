import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useToastStore } from "../stores/toastStore";
import i18n from "../i18n";
import { basename } from "./pathUtils";

/// Shared export flow: native save dialog → fetch the payload → write via
/// the backend → success/error toast. Quietly returns when the user cancels
/// the dialog. Replaces the blob-`<a download>` pattern, which offered no
/// destination choice, no feedback, and swallowed failures into the console
/// (and doesn't work reliably in WKWebView at all).
export async function exportTextFile(opts: {
  /** Suggested filename shown in the save dialog, e.g. "assets.json". */
  defaultName: string;
  /** Human-readable filter label, e.g. "JSON". */
  filterName: string;
  /** Allowed extensions without dots, e.g. ["json"]. */
  extensions: string[];
  /** Producer for the file contents — runs only after a destination is chosen. */
  fetchContents: () => Promise<string>;
}): Promise<void> {
  const { defaultName, filterName, extensions, fetchContents } = opts;
  const { push } = useToastStore.getState();
  try {
    const path = await save({
      defaultPath: defaultName,
      filters: [{ name: filterName, extensions }],
    });
    if (!path) return; // user cancelled the dialog

    const contents = await fetchContents();
    await invoke("save_text_file", { path, contents });

    push({
      kind: "success",
      message: i18n.t("exportToast.saved", { name: basename(path) }),
      actionLabel: i18n.t("exportToast.showInFolder"),
      onAction: () => {
        invoke("show_in_file_manager", { path }).catch((err) =>
          console.error("Failed to reveal exported file:", err)
        );
      },
    });
  } catch (err) {
    push({
      kind: "error",
      message: i18n.t("exportToast.failed", { reason: String(err) }),
    });
  }
}
