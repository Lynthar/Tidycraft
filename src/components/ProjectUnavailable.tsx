import { useEffect, useRef, useState } from "react";
import { FolderX } from "lucide-react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/projectStore";
import { basename } from "../lib/pathUtils";

/// Which project a check belongs to is part of the state: two unavailable
/// projects side by side reuse this component across a switch, so a bare timestamp
/// would surface on a project the user never asked to re-check.
type CheckState =
  | { projectId: string; phase: "checking" }
  | { projectId: string; phase: "checked"; at: string };

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
  const openProject = useProjectStore((s) => s.openProject);

  const [check, setCheck] = useState<CheckState | null>(null);

  // The check crosses an await, and "remove from workspace" sits right next to
  // it — the user can unmount this panel while the check is still running.
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const checking = check?.projectId === activeProjectId && check.phase === "checking";
  const checkedAt =
    check?.projectId === activeProjectId && check.phase === "checked" ? check.at : null;

  const handleCheckAgain = async () => {
    if (!activeProjectId || !projectPath || checking) return;
    setCheck({ projectId: activeProjectId, phase: "checking" });
    try {
      // Deliberately not `rescan()`: that clears the on-disk scan cache first,
      // which makes the recovery scan re-read every file, and throws the cache
      // away again on each hopeful click while the folder is still missing. This
      // question is "is the folder back?", and openProject answers it by checking
      // the path before it scans anything.
      await openProject(projectPath, { force: true });
    } finally {
      // A folder that came back unmounts this panel, so the only state worth
      // writing here is the one nobody sees unless it is still gone.
      if (alive.current) {
        setCheck({
          projectId: activeProjectId,
          phase: "checked",
          // Seconds, not just minutes: two clicks inside one minute would
          // otherwise leave the line unchanged and read as no response again.
          at: new Date().toLocaleTimeString(),
        });
      }
    }
  };

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
        <button
          className="tc-unavail-secondary"
          type="button"
          disabled={checking}
          onClick={handleCheckAgain}
        >
          {checking ? t("projects.unavailable.checking") : t("projects.unavailable.checkAgain")}
        </button>
      </div>
      {checkedAt && (
        <div className="tc-unavail-result">
          {t("projects.unavailable.checkedAt", { time: checkedAt })}
        </div>
      )}
      <div className="tc-unavail-forget">
        <button
          className="tc-unavail-danger"
          type="button"
          onClick={() => activeProjectId && removeProject(activeProjectId)}
        >
          {t("projects.unavailable.remove")}
        </button>
      </div>
    </div>
  );
}
