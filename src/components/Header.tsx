import { RefObject } from "react";
import { FolderOpen, RefreshCw, Search, X, Globe, Sun, Moon } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useThemeStore } from "../stores/themeStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";
import type { AssetType } from "../types/asset";

interface HeaderProps {
  searchInputRef?: RefObject<HTMLInputElement>;
}

export function Header({ searchInputRef }: HeaderProps) {
  const { t, i18n } = useTranslation();
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
  const { theme, toggleTheme } = useThemeStore();

  const ASSET_TYPES: { value: AssetType | null; label: string }[] = [
    { value: null, label: t("assetTypes.all") },
    { value: "texture", label: t("assetTypes.texture") },
    { value: "model", label: t("assetTypes.model") },
    { value: "audio", label: t("assetTypes.audio") },
    { value: "animation", label: t("assetTypes.animation") },
    { value: "material", label: t("assetTypes.material") },
    { value: "prefab", label: t("assetTypes.prefab") },
    { value: "scene", label: t("assetTypes.scene") },
    { value: "script", label: t("assetTypes.script") },
    { value: "data", label: t("assetTypes.data") },
    { value: "other", label: t("assetTypes.other") },
  ];

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

  const handleRescan = () => {
    if (projectPath) {
      openProject(projectPath);
    }
  };

  const toggleLanguage = () => {
    const newLang = i18n.language === "en" ? "zh" : "en";
    i18n.changeLanguage(newLang);
    localStorage.setItem("language", newLang);
  };

  const projectName = projectPath
    ? projectPath.split("/").pop() || projectPath.split("\\").pop() || "Project"
    : null;

  return (
    <header className="h-12 bg-card-bg border-b border-border flex items-center justify-between px-4 gap-4">
      <div className="flex items-center gap-3 shrink-0">
        <span className="text-lg font-semibold text-primary">{t("app.name")}</span>
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
              ref={searchInputRef}
              type="text"
              placeholder={`${t("header.searchPlaceholder")} (${formatShortcut(SHORTCUTS.search)})`}
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
        {/* Theme Toggle */}
        <button
          onClick={toggleTheme}
          className="p-2 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        >
          {theme === "dark" ? <Sun size={18} /> : <Moon size={18} />}
        </button>

        {/* Language Toggle */}
        <button
          onClick={toggleLanguage}
          className="p-2 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          title={i18n.language === "en" ? "切换到中文" : "Switch to English"}
        >
          <Globe size={18} />
        </button>

        {projectPath && (
          <button
            onClick={handleRescan}
            disabled={isScanning}
            className="p-2 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors disabled:opacity-50"
            title={`${t("header.rescan")} (${formatShortcut(SHORTCUTS.rescan)})`}
          >
            <RefreshCw size={18} className={isScanning ? "animate-spin" : ""} />
          </button>
        )}
        <button
          onClick={handleOpenFolder}
          disabled={isScanning}
          className="flex items-center gap-2 px-3 py-1.5 bg-primary text-white rounded hover:bg-primary/90 transition-colors disabled:opacity-50"
          title={formatShortcut(SHORTCUTS.openFolder)}
        >
          <FolderOpen size={16} />
          <span>{t("header.openFolder")}</span>
        </button>
      </div>
    </header>
  );
}
