import { FolderOpen, RefreshCw } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/projectStore";

export function Header() {
  const { projectPath, isScanning, openProject } = useProjectStore();

  const handleOpenFolder = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Select Project Folder",
    });

    if (selected && typeof selected === "string") {
      openProject(selected);
    }
  };

  const handleRescan = () => {
    if (projectPath) {
      openProject(projectPath);
    }
  };

  const projectName = projectPath
    ? projectPath.split("/").pop() || projectPath.split("\\").pop() || "Project"
    : null;

  return (
    <header className="h-12 bg-card-bg border-b border-border flex items-center justify-between px-4">
      <div className="flex items-center gap-3">
        <span className="text-lg font-semibold text-primary">Tidycraft</span>
        {projectName && (
          <>
            <span className="text-text-secondary">|</span>
            <span className="text-text-primary">{projectName}</span>
          </>
        )}
      </div>

      <div className="flex items-center gap-2">
        {projectPath && (
          <button
            onClick={handleRescan}
            disabled={isScanning}
            className="p-2 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors disabled:opacity-50"
            title="Rescan"
          >
            <RefreshCw size={18} className={isScanning ? "animate-spin" : ""} />
          </button>
        )}
        <button
          onClick={handleOpenFolder}
          disabled={isScanning}
          className="flex items-center gap-2 px-3 py-1.5 bg-primary text-white rounded hover:bg-primary/90 transition-colors disabled:opacity-50"
        >
          <FolderOpen size={16} />
          <span>Open Folder</span>
        </button>
      </div>
    </header>
  );
}
