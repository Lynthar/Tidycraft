import { FolderX } from "lucide-react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/projectStore";
import { basename } from "../lib/pathUtils";

/// Shown in the main area instead of the asset list when the active project's
/// folder is unusable. Without it the user gets an empty AssetList, which
/// reads as "this project has no assets" rather than "this folder is gone".
export function ProjectUnavailable() {
  const { t } = useTranslation();
  const projectPath = useProjectStore((s) => s.projectPath);
  const unavailable = useProjectStore((s) => s.unavailable);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const relocateProject = useProjectStore((s) => s.relocateProject);
  const removeProject = useProjectStore((s) => s.removeProject);
  const rescan = useProjectStore((s) => s.rescan);

  const handleRelocate = async () => {
    if (!activeProjectId) return;
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("header.selectProjectFolder"),
    });
    if (selected && typeof selected === "string") {
      await relocateProject(activeProjectId, selected);
    }
  };

  if (!unavailable || !projectPath) return null;

  // `unavailable` can only be one of these three kinds — "ok" isn't a value
  // this field can hold (see the UnavailableStatus type), so there is no
  // dead `ok` branch to carry here and `detail` narrows for free below.
  let copy: { title: string; body: string };
  switch (unavailable.kind) {
    case "missing":
      copy = {
        title: t("projects.unavailable.titleMissing"),
        body: t("projects.unavailable.bodyMissing"),
      };
      break;
    case "not_a_directory":
      copy = {
        title: t("projects.unavailable.titleNotADirectory"),
        body: t("projects.unavailable.bodyNotADirectory"),
      };
      break;
    case "unreadable":
      copy = {
        title: t("projects.unavailable.titleUnreadable"),
        body: t("projects.unavailable.bodyUnreadable", {
          detail: unavailable.detail,
        }),
      };
      break;
  }

  return (
    <div className="tc-unavail">
      <div className="tc-unavail-glyph">
        <FolderX size={28} />
      </div>
      <div className="tc-unavail-title">{copy.title}</div>
      <div className="tc-unavail-name">{basename(projectPath) || projectPath}</div>
      <div className="tc-unavail-path mono">{projectPath}</div>
      <div className="tc-unavail-body">{copy.body}</div>
      <div className="tc-unavail-actions">
        <button className="tc-cta" type="button" onClick={handleRelocate}>
          {t("projects.unavailable.relocate")}
        </button>
        <button className="tc-unavail-secondary" type="button" onClick={() => rescan()}>
          {t("projects.unavailable.checkAgain")}
        </button>
        <button
          className="tc-unavail-secondary"
          type="button"
          onClick={() => activeProjectId && removeProject(activeProjectId)}
        >
          {t("projects.unavailable.remove")}
        </button>
      </div>
    </div>
  );
}
