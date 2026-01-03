import { RefObject, useState, useRef, useEffect } from "react";
import { FolderOpen, RefreshCw, Search, X, Globe, Sun, Moon, GitBranch, ChevronDown, Check, Undo2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useThemeStore } from "../stores/themeStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";
import type { AssetType } from "../types/asset";
import { AdvancedFiltersPanel } from "./AdvancedFilters";
import { SearchHistory } from "./SearchHistory";
import { useSearchHistoryStore } from "../stores/searchHistoryStore";

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
    gitInfo,
    canUndo,
    openProject,
    setSearchQuery,
    setTypeFilter,
    undoLastOperation,
    refreshUndoState,
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

  // Language dropdown state
  const [showLangDropdown, setShowLangDropdown] = useState(false);
  const langDropdownRef = useRef<HTMLDivElement>(null);

  // Search history state
  const [showSearchHistory, setShowSearchHistory] = useState(false);
  const { addToHistory } = useSearchHistoryStore();

  const LANGUAGES = [
    { code: "en", label: "English" },
    { code: "zh", label: "中文" },
  ];

  const changeLanguage = (langCode: string) => {
    i18n.changeLanguage(langCode);
    localStorage.setItem("language", langCode);
    setShowLangDropdown(false);
  };

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (langDropdownRef.current && !langDropdownRef.current.contains(e.target as Node)) {
        setShowLangDropdown(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // Refresh undo state when project changes
  useEffect(() => {
    if (projectPath) {
      refreshUndoState();
    }
  }, [projectPath, refreshUndoState]);

  const handleUndo = async () => {
    const result = await undoLastOperation();
    if (result && result.success) {
      // Trigger rescan after undo
      if (projectPath) {
        openProject(projectPath);
      }
    }
  };

  const projectName = projectPath
    ? projectPath.split("/").pop() || projectPath.split("\\").pop() || "Project"
    : null;

  return (
    <header className="h-12 bg-card-bg border-b border-border flex items-center justify-between px-4 gap-4">
      <div className="flex items-center gap-3 shrink-0">
        <span className="text-lg font-semibold text-primary">{t("app.name")}</span>
        {projectPath && (
          <button
            onClick={handleUndo}
            disabled={!canUndo}
            className="p-1.5 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
            title={t("common.undo", "Undo")}
          >
            <Undo2 size={16} />
          </button>
        )}
        {projectName && (
          <>
            <span className="text-text-secondary">|</span>
            <span className="text-text-primary">{projectName}</span>
          </>
        )}
        {/* Git Branch Info */}
        {gitInfo?.is_repo && gitInfo.branch && (
          <div className="flex items-center gap-1.5 text-sm text-text-secondary">
            <GitBranch size={14} />
            <span>{gitInfo.branch}</span>
            {(gitInfo.ahead > 0 || gitInfo.behind > 0) && (
              <span className="text-xs">
                {gitInfo.ahead > 0 && <span className="text-green-400">↑{gitInfo.ahead}</span>}
                {gitInfo.behind > 0 && <span className="text-orange-400 ml-1">↓{gitInfo.behind}</span>}
              </span>
            )}
            {gitInfo.has_changes && (
              <span className="w-2 h-2 rounded-full bg-yellow-400" title={t("git.hasChanges")} />
            )}
          </div>
        )}
      </div>

      {/* Search and Filter */}
      {scanResult && (
        <div className="flex items-center gap-3 flex-1 max-w-xl">
          {/* Unified Search and Filter */}
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
              onFocus={() => setShowSearchHistory(true)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && searchQuery.trim()) {
                  addToHistory(searchQuery.trim());
                  setShowSearchHistory(false);
                  (e.target as HTMLInputElement).blur();
                } else if (e.key === "Escape") {
                  setSearchQuery("");
                  setShowSearchHistory(false);
                }
              }}
              className="w-full h-8 pl-8 pr-20 text-sm bg-background border border-border rounded
                         text-text-primary placeholder:text-text-secondary
                         focus:outline-none focus:border-primary transition-colors"
            />
            <div className="absolute right-2 top-1/2 -translate-y-1/2 flex items-center gap-1">
              {searchQuery && (
                <button
                  onClick={() => setSearchQuery("")}
                  className="p-0.5 text-text-secondary hover:text-text-primary transition-colors"
                >
                  <X size={14} />
                </button>
              )}
              <AdvancedFiltersPanel />
            </div>
            {/* Search History Dropdown */}
            <SearchHistory
              isVisible={showSearchHistory}
              searchQuery={searchQuery}
              onSelect={(query) => {
                setSearchQuery(query);
                addToHistory(query);
                setShowSearchHistory(false);
              }}
              onClose={() => setShowSearchHistory(false)}
            />
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
          title={theme === "dark" ? t("theme.switchToLight") : t("theme.switchToDark")}
        >
          {theme === "dark" ? <Sun size={18} /> : <Moon size={18} />}
        </button>

        {/* Language Dropdown */}
        <div className="relative" ref={langDropdownRef}>
          <button
            onClick={() => setShowLangDropdown(!showLangDropdown)}
            className="flex items-center gap-1 px-2 py-1.5 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
            title={t("settings.language")}
          >
            <Globe size={16} />
            <span className="text-sm">{LANGUAGES.find(l => l.code === i18n.language)?.label || "Language"}</span>
            <ChevronDown size={12} />
          </button>
          {showLangDropdown && (
            <div className="absolute right-0 top-full mt-1 bg-card-bg border border-border rounded-lg shadow-lg z-50 py-1 min-w-[140px]">
              {LANGUAGES.map((lang) => (
                <button
                  key={lang.code}
                  onClick={() => changeLanguage(lang.code)}
                  className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-background transition-colors"
                >
                  <span className="flex-1">{lang.label}</span>
                  {i18n.language === lang.code && <Check size={14} className="text-primary" />}
                </button>
              ))}
            </div>
          )}
        </div>

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
