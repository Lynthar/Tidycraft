import { RefObject, useState, useRef, useEffect } from "react";
import {
  RefreshCw,
  Search,
  X,
  Globe,
  Sun,
  Moon,
  GitBranch,
  Check,
  Undo2,
  Settings,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useShallow } from "zustand/react/shallow";
import { useTagsStore } from "../stores/tagsStore";
import { useThemeStore } from "../stores/themeStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useUiStore } from "../stores/uiStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";
import { AdvancedFiltersPanel } from "./AdvancedFilters";
import { SearchHistory } from "./SearchHistory";
import { ProjectSwitcher } from "./ProjectSwitcher";
import { useSearchHistoryStore } from "../stores/searchHistoryStore";

interface HeaderProps {
  searchInputRef?: RefObject<HTMLInputElement>;
}

const LANGUAGES = [
  { code: "en", label: "English" },
  { code: "zh", label: "中文" },
];

export function Header({ searchInputRef }: HeaderProps) {
  const { t, i18n } = useTranslation();
  const {
    projectPath,
    isScanning,
    scanResult,
    searchQuery,
    gitInfo,
    canUndo,
    rescan,
    setSearchQuery,
    undoLastOperation,
    refreshUndoState,
    refreshGitInfo,
  } = useProjectStore(
    useShallow((s) => ({ projectPath: s.projectPath, isScanning: s.isScanning, scanResult: s.scanResult, searchQuery: s.searchQuery, gitInfo: s.gitInfo, canUndo: s.canUndo, rescan: s.rescan, setSearchQuery: s.setSearchQuery, undoLastOperation: s.undoLastOperation, refreshUndoState: s.refreshUndoState, refreshGitInfo: s.refreshGitInfo, }))
  );
  const { theme, toggleTheme } = useThemeStore();
  const { showBranchInfo, showAheadBehind } = useSettingsStore();
  const { addToHistory } = useSearchHistoryStore();
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);

  const [showLangDropdown, setShowLangDropdown] = useState(false);
  const [showSearchHistory, setShowSearchHistory] = useState(false);
  const [refreshingGit, setRefreshingGit] = useState(false);
  const langDropdownRef = useRef<HTMLDivElement>(null);

  // Manual git-status refresh. The spinner stays up until the git IO actually
  // settles, but for at least 600ms so tiny repos (near-instant refresh) still
  // show clear feedback instead of a flicker — the old fixed 600ms timeout
  // could stop the spinner while a slower refresh was still running. Errors
  // are swallowed by refreshGitInfo's own try/catch.
  const handleGitRefresh = async () => {
    if (refreshingGit) return;
    setRefreshingGit(true);
    try {
      await Promise.all([
        refreshGitInfo(),
        new Promise((resolve) => setTimeout(resolve, 600)),
      ]);
    } finally {
      setRefreshingGit(false);
    }
  };

  // Rescan = clear the scan cache + force re-open, via the shared store action
  // so the Ctrl+R shortcut (advertised in this button's tooltip) does exactly
  // the same thing. The action no-ops without a project / while scanning.
  const handleRescan = () => {
    rescan();
  };

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

  useEffect(() => {
    if (projectPath) {
      refreshUndoState();
    }
  }, [projectPath, refreshUndoState]);

  const handleUndo = async () => {
    const result = await undoLastOperation();
    if (result && result.success) {
      // Undo carried tag bindings back to the original paths on the backend;
      // re-sync the tags store so they reappear without waiting for the
      // watcher's scanResult refresh. The scan list itself is refreshed by
      // the watcher — the bare openProject(path) that used to sit here was
      // a guaranteed no-op (already-open + not-force short-circuits).
      await useTagsStore.getState().loadTags();
    }
  };

  return (
    <header className="tc-header">
      {/* Brand */}
      <div className="tc-brand">
        <div className="tc-brand-mark" />
        <span className="tc-brand-name">{t("app.name")}</span>
      </div>

      {/* Project bar — undo + project switcher dropdown + git branch sidecar */}
      <div className="tc-proj-bar">
        {projectPath && (
          <button
            onClick={handleUndo}
            disabled={!canUndo}
            className="tc-icon-btn"
            title={t("common.undo", "Undo")}
          >
            <Undo2 size={14} />
          </button>
        )}
        <ProjectSwitcher />
        {projectPath && showBranchInfo && gitInfo?.is_repo && gitInfo.branch && (
          <span
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 4,
              color: "var(--text-3)",
              fontSize: 11,
              paddingLeft: 8,
              borderLeft: "1px solid var(--line-soft)",
            }}
          >
            <GitBranch size={12} />
            <span>{gitInfo.branch}</span>
            {showAheadBehind && (gitInfo.ahead > 0 || gitInfo.behind > 0) && (
              <span style={{ fontSize: 10 }}>
                {gitInfo.ahead > 0 && (
                  <span style={{ color: "var(--ok)" }}>↑{gitInfo.ahead}</span>
                )}
                {gitInfo.behind > 0 && (
                  <span style={{ color: "var(--warn)", marginLeft: 4 }}>
                    ↓{gitInfo.behind}
                  </span>
                )}
              </span>
            )}
            {gitInfo.has_changes && (
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: "50%",
                  background: "var(--warn)",
                }}
                title={t("git.hasChanges")}
              />
            )}
            <button
              type="button"
              onClick={handleGitRefresh}
              title={t("git.refresh")}
              style={{
                background: "transparent",
                border: 0,
                cursor: "pointer",
                padding: "1px 2px",
                marginLeft: 2,
                color: "var(--text-3)",
                display: "inline-flex",
                alignItems: "center",
              }}
              onMouseEnter={(e) =>
                (e.currentTarget.style.color = "var(--text)")
              }
              onMouseLeave={(e) =>
                (e.currentTarget.style.color = "var(--text-3)")
              }
            >
              <RefreshCw
                size={10}
                className={refreshingGit ? "animate-spin" : ""}
              />
            </button>
          </span>
        )}
      </div>

      {/* Search (only when there's a scan result to search through) */}
      {scanResult && (
        <div
          style={{
            position: "relative",
            flex: 1,
            maxWidth: 520,
            marginLeft: "auto",
            display: "flex",
            alignItems: "center",
            gap: 8,
          }}
        >
          <label className="tc-search">
            <Search size={13} />
            <input
              ref={searchInputRef}
              type="text"
              placeholder={t("header.searchPlaceholder")}
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
            />
            {searchQuery ? (
              <button
                onClick={() => setSearchQuery("")}
                className="tc-icon-btn"
                style={{ width: 20, height: 20 }}
                title={t("header.clearSearch", "Clear")}
              >
                <X size={12} />
              </button>
            ) : (
              <span className="tc-kbd">{formatShortcut(SHORTCUTS.search)}</span>
            )}
          </label>
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
          <AdvancedFiltersPanel />
        </div>
      )}

      {/* Header actions */}
      <div className="tc-header-actions">
        <button
          onClick={() => setSettingsOpen(true)}
          className="tc-icon-btn"
          title={t("settings.title")}
        >
          <Settings size={14} />
        </button>

        <button
          onClick={toggleTheme}
          className="tc-icon-btn"
          title={theme === "dark" ? t("theme.switchToLight") : t("theme.switchToDark")}
        >
          {theme === "dark" ? <Sun size={14} /> : <Moon size={14} />}
        </button>

        <div style={{ position: "relative" }} ref={langDropdownRef}>
          <button
            onClick={() => setShowLangDropdown(!showLangDropdown)}
            className="tc-icon-btn"
            data-active={showLangDropdown ? "true" : undefined}
            title={t("settings.language")}
          >
            <Globe size={14} />
          </button>
          {showLangDropdown && (
            <div
              style={{
                position: "absolute",
                right: 0,
                top: "calc(100% + 4px)",
                zIndex: 50,
                minWidth: 140,
                padding: "4px 0",
                background: "var(--panel)",
                border: "1px solid var(--line)",
                borderRadius: 8,
                boxShadow: "var(--shadow-pop)",
              }}
            >
              {LANGUAGES.map((lang) => (
                <button
                  key={lang.code}
                  onClick={() => changeLanguage(lang.code)}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    width: "100%",
                    padding: "6px 12px",
                    fontSize: 12.5,
                    textAlign: "left",
                    color: "var(--text)",
                    background: "transparent",
                    border: 0,
                    cursor: "pointer",
                  }}
                  onMouseEnter={(e) => (e.currentTarget.style.background = "var(--panel-hover)")}
                  onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                >
                  <span style={{ flex: 1 }}>{lang.label}</span>
                  {i18n.language === lang.code && (
                    <Check size={14} style={{ color: "var(--primary)" }} />
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        {projectPath && (
          <button
            onClick={handleRescan}
            disabled={isScanning}
            className="tc-icon-btn"
            title={`${t("header.rescan")} (${formatShortcut(SHORTCUTS.rescan)})`}
          >
            <RefreshCw size={14} className={isScanning ? "animate-spin" : ""} />
          </button>
        )}
      </div>

    </header>
  );
}
