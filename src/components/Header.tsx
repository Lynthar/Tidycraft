import { FolderOpen, RefreshCw, Search, X } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../stores/projectStore";
import type { AssetType } from "../types/asset";

const ASSET_TYPES: { value: AssetType | null; label: string }[] = [
  { value: null, label: "All Types" },
  { value: "texture", label: "Textures" },
  { value: "model", label: "Models" },
  { value: "audio", label: "Audio" },
  { value: "animation", label: "Animations" },
  { value: "material", label: "Materials" },
  { value: "prefab", label: "Prefabs" },
  { value: "scene", label: "Scenes" },
  { value: "script", label: "Scripts" },
  { value: "data", label: "Data" },
  { value: "other", label: "Other" },
];

export function Header() {
  const {
    projectPath,
    isScanning,
    scanResult,
    searchQuery,
    typeFilter,
    openProject,
    setSearchQuery,
    setTypeFilter,
  } = useProjectStore();

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
    <header className="h-12 bg-card-bg border-b border-border flex items-center justify-between px-4 gap-4">
      <div className="flex items-center gap-3 shrink-0">
        <span className="text-lg font-semibold text-primary">Tidycraft</span>
        {projectName && (
          <>
            <span className="text-text-secondary">|</span>
            <span className="text-text-primary">{projectName}</span>
          </>
        )}
      </div>

      {/* Search and Filter */}
      {scanResult && (
        <div className="flex items-center gap-3 flex-1 max-w-xl">
          {/* Search Input */}
          <div className="relative flex-1">
            <Search
              size={14}
              className="absolute left-2.5 top-1/2 -translate-y-1/2 text-text-secondary"
            />
            <input
              type="text"
              placeholder="Search assets..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full h-8 pl-8 pr-8 text-sm bg-background border border-border rounded
                         text-text-primary placeholder:text-text-secondary
                         focus:outline-none focus:border-primary transition-colors"
            />
            {searchQuery && (
              <button
                onClick={() => setSearchQuery("")}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-text-secondary hover:text-text-primary"
              >
                <X size={14} />
              </button>
            )}
          </div>

          {/* Type Filter */}
          <select
            value={typeFilter || ""}
            onChange={(e) => setTypeFilter((e.target.value as AssetType) || null)}
            className="h-8 px-2 text-sm bg-background border border-border rounded
                       text-text-primary focus:outline-none focus:border-primary
                       transition-colors cursor-pointer"
          >
            {ASSET_TYPES.map((type) => (
              <option key={type.value || "all"} value={type.value || ""}>
                {type.label}
              </option>
            ))}
          </select>
        </div>
      )}

      <div className="flex items-center gap-2 shrink-0">
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
