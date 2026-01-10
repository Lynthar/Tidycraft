import { useState } from "react";
import { FolderOpen, X, Plus, ChevronDown, ChevronRight } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { cn } from "../lib/utils";

export function ProjectList() {
  const { t } = useTranslation();
  const { getProjectList, setActiveProject, closeProject, openProject, isScanning } = useProjectStore();
  const [isCollapsed, setIsCollapsed] = useState(false);

  const projects = getProjectList();

  const handleOpenProject = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("header.selectProjectFolder"),
    });

    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  };

  const handleCloseProject = (e: React.MouseEvent, projectId: string) => {
    e.stopPropagation();
    closeProject(projectId);
  };

  if (projects.length === 0) {
    return null;
  }

  return (
    <div className="w-full h-full bg-card-bg border-r border-border flex flex-col">
      {/* Header */}
      <div
        className="flex items-center justify-between px-3 py-2 border-b border-border cursor-pointer hover:bg-background/50 transition-colors"
        onClick={() => setIsCollapsed(!isCollapsed)}
      >
        <div className="flex items-center gap-2 text-sm font-medium text-text-secondary">
          {isCollapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
          <span>{t("projects.title")}</span>
          <span className="text-xs text-text-secondary">({projects.length})</span>
        </div>
      </div>

      {/* Project List */}
      {!isCollapsed && (
        <div className="flex-1 overflow-auto">
          <div className="py-1">
            {projects.map((project) => (
              <div
                key={project.id}
                onClick={() => setActiveProject(project.id)}
                className={cn(
                  "flex items-center gap-2 px-3 py-2 cursor-pointer transition-colors group",
                  project.isActive
                    ? "bg-primary/20 text-primary border-l-2 border-primary"
                    : "hover:bg-background text-text-primary border-l-2 border-transparent"
                )}
              >
                <FolderOpen size={14} className="shrink-0" />
                <span className="flex-1 text-sm truncate" title={project.path}>
                  {project.name}
                </span>
                <button
                  onClick={(e) => handleCloseProject(e, project.id)}
                  className={cn(
                    "p-0.5 rounded hover:bg-background/50 transition-opacity",
                    project.isActive ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                  )}
                  title={t("projects.close")}
                >
                  <X size={12} />
                </button>
              </div>
            ))}
          </div>

          {/* Add Project Button */}
          <div className="px-2 py-2 border-t border-border">
            <button
              onClick={handleOpenProject}
              disabled={isScanning}
              className="flex items-center gap-2 w-full px-2 py-1.5 text-sm text-text-secondary hover:text-text-primary hover:bg-background rounded transition-colors disabled:opacity-50"
            >
              <Plus size={14} />
              <span>{t("projects.addProject")}</span>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
