import { FolderOpen } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useRecentsStore } from "../stores/recentsStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";

export function EmptyState() {
  const { t } = useTranslation();
  const { openProject } = useProjectStore();
  const recents = useRecentsStore((s) => s.recents);

  const handleOpenFolder = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("header.selectProjectFolder"),
    });
    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  };

  return (
    <div className="tc-empty">
      <div className="tc-empty-glyph">
        <FolderOpen size={28} />
      </div>
      <div className="tc-empty-title">
        {t("emptyState.title", "Open a project to begin")}
      </div>
      <div className="tc-empty-sub">
        {recents.length > 0
          ? t(
              "emptyState.subtitle",
              "Pick a recent project from the switcher in the top-left, or open a folder to begin scanning."
            )
          : t(
              "emptyState.subtitleFirst",
              "Open a folder to start scanning your game assets."
            )}
      </div>
      <div className="tc-empty-actions">
        <button onClick={handleOpenFolder} className="tc-cta">
          <FolderOpen size={13} />
          <span>{t("header.openFolder")}</span>
          <span className="tc-kbd" style={{ marginLeft: 6 }}>
            {formatShortcut(SHORTCUTS.openFolder)}
          </span>
        </button>
      </div>
    </div>
  );
}
