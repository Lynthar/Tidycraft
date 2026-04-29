import { useState, useEffect, useRef, useCallback } from "react";
import { ChevronDown, FolderOpen, X } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";
import type { ProjectType } from "../types/asset";

function glyphLetter(name: string, engine: ProjectType | undefined): string {
  if (engine === "unity") return "Un";
  if (engine === "unreal") return "UE";
  if (engine === "godot") return "G";
  if (engine === "generic") return name.charAt(0).toUpperCase() || "P";
  return name.charAt(0).toUpperCase() || "P";
}

export function ProjectSwitcher() {
  const { t } = useTranslation();
  const {
    projectPath,
    scanResult,
    activeProjectId,
    getProjectList,
    setActiveProject,
    closeProject,
    openProject,
    isScanning,
  } = useProjectStore();

  const projects = getProjectList();
  const activeProject = projects.find((p) => p.isActive);
  const wrapperRef = useRef<HTMLDivElement>(null);

  // Default-open as onboarding when there is no active project on mount.
  const [isOpen, setIsOpen] = useState(() => activeProjectId === null);

  // Auto-close once a project becomes active (e.g. after session restore
  // finishes or the user picks a folder from the dropdown itself).
  const wasNullRef = useRef(activeProjectId === null);
  useEffect(() => {
    if (wasNullRef.current && activeProjectId !== null) {
      setIsOpen(false);
    }
    wasNullRef.current = activeProjectId === null;
  }, [activeProjectId]);

  // Click-outside + ESC to close.
  useEffect(() => {
    if (!isOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (wrapperRef.current && !wrapperRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setIsOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [isOpen]);

  const handleOpenFolder = useCallback(async () => {
    setIsOpen(false);
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("header.selectProjectFolder"),
    });
    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  }, [openProject, t]);

  const handleSelect = useCallback(
    (projectId: string) => {
      setActiveProject(projectId);
      setIsOpen(false);
    },
    [setActiveProject]
  );

  const handleClose = useCallback(
    (e: React.MouseEvent, projectId: string) => {
      e.stopPropagation();
      closeProject(projectId);
    },
    [closeProject]
  );

  // Engine is only known for the *active* project's scanResult; other
  // entries fall back to "generic" until we have a richer per-project cache.
  const activeEngine = scanResult?.project_type;
  const triggerLabel =
    activeProject?.name ?? t("projects.selectProject", "Select project");

  return (
    <div className="tc-proj-switch-wrap" ref={wrapperRef}>
      <button
        className="tc-proj-switch"
        data-open={isOpen ? "true" : undefined}
        onClick={() => setIsOpen((v) => !v)}
        title={projectPath ?? undefined}
      >
        <span className="tc-proj-dot" />
        <span className="tc-proj-name">{triggerLabel}</span>
        {activeEngine && activeEngine !== "generic" && (
          <span className="tc-proj-type" data-engine={activeEngine}>
            {activeEngine.toUpperCase()}
          </span>
        )}
        <span className="tc-proj-chev">
          <ChevronDown size={12} />
        </span>
      </button>

      {isOpen && (
        <div className="tc-projmenu" role="menu">
          <div className="tc-projmenu-head">
            <span className="tc-projmenu-title">
              {t("projects.title", "Projects")}
            </span>
            {projects.length > 0 && (
              <span className="tc-projmenu-sub">{projects.length}</span>
            )}
          </div>

          {projects.length > 0 ? (
            <div className="tc-projmenu-list">
              {projects.map((p) => {
                const engine = p.isActive ? activeEngine : undefined;
                const engineKey = (engine ?? "generic") as
                  | "unity"
                  | "unreal"
                  | "godot"
                  | "generic";
                // Outer is a div with role="button" because we need a nested
                // close <button> inside, and <button> in <button> is illegal.
                return (
                  <div
                    key={p.id}
                    className="tc-projmenu-item"
                    data-current={p.isActive ? "true" : undefined}
                    onClick={() => handleSelect(p.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        handleSelect(p.id);
                      }
                    }}
                    role="button"
                    tabIndex={0}
                    title={p.path}
                  >
                    <span className="tc-projmenu-glyph" data-engine={engineKey}>
                      {glyphLetter(p.name, engine)}
                    </span>
                    <span className="tc-projmenu-body">
                      <span className="tc-projmenu-name">
                        <span style={{
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}>
                          {p.name}
                        </span>
                        {p.isActive && (
                          <span className="tc-projmenu-curr">
                            {t("projects.current", "current")}
                          </span>
                        )}
                      </span>
                      <span className="tc-projmenu-path">{p.path}</span>
                    </span>
                    <span className="tc-projmenu-meta">
                      {engine && engine !== "generic" && (
                        <span className="tc-proj-type" data-engine={engine}>
                          {engine.toUpperCase()}
                        </span>
                      )}
                      <button
                        className="tc-projmenu-close"
                        onClick={(e) => handleClose(e, p.id)}
                        title={t("projects.close", "Close")}
                        aria-label={t("projects.close", "Close")}
                      >
                        <X size={12} />
                      </button>
                    </span>
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="tc-projmenu-empty">
              {t("projects.empty", "No projects open")}
            </div>
          )}

          <div className="tc-projmenu-divider" />
          <div className="tc-projmenu-actions">
            <button
              className="tc-projmenu-action"
              onClick={handleOpenFolder}
              disabled={isScanning}
            >
              <FolderOpen size={13} />
              <span>{t("header.openFolder")}</span>
              <span className="tc-kbd mono">
                {formatShortcut(SHORTCUTS.openFolder)}
              </span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
