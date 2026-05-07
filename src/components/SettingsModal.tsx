import { useEffect, useState } from "react";
import { X, GitBranch, Palette, Wrench, Trash2, Image as ImageIcon, FileCode, ExternalLink } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "../stores/settingsStore";
import { useThemeStore, type ThemePreference } from "../stores/themeStore";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

function ToggleSwitch({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <label className="flex items-start gap-3 cursor-pointer group">
      <div className="pt-0.5">
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          onClick={() => onChange(!checked)}
          className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 ${
            checked ? "bg-primary" : "bg-gray-600"
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              checked ? "translate-x-4" : "translate-x-0"
            }`}
          />
        </button>
      </div>
      <div className="flex-1">
        <span className="text-sm font-medium text-text-primary group-hover:text-primary transition-colors">
          {label}
        </span>
        {description && (
          <p className="text-xs text-text-secondary mt-0.5">{description}</p>
        )}
      </div>
    </label>
  );
}

function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  label,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  label: string;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-sm font-medium text-text-primary">{label}</span>
      <div
        className="inline-flex p-0.5 rounded-md"
        style={{ background: "var(--panel-2)", border: "1px solid var(--line)" }}
      >
        {options.map((opt) => {
          const active = value === opt.value;
          return (
            <button
              key={opt.value}
              type="button"
              onClick={() => onChange(opt.value)}
              className="px-3 py-1 text-xs rounded transition-colors"
              style={{
                background: active ? "var(--panel)" : "transparent",
                color: active ? "var(--text)" : "var(--text-3)",
                fontWeight: active ? 500 : 400,
                border: 0,
                cursor: "pointer",
                boxShadow: active ? "0 1px 2px oklch(0% 0 0 / 0.05)" : "none",
              }}
            >
              {opt.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function SettingsModal({ isOpen, onClose }: SettingsModalProps) {
  const { t, i18n } = useTranslation();
  const {
    showGitStatusIndicators,
    showBranchInfo,
    showAheadBehind,
    setShowGitStatusIndicators,
    setShowBranchInfo,
    setShowAheadBehind,
  } = useSettingsStore();
  const preference = useThemeStore((s) => s.preference);
  const setPreference = useThemeStore((s) => s.setPreference);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const hasCustomConfig = useProjectStore((s) => s.hasCustomConfig);
  const setHasCustomConfig = useProjectStore((s) => s.setHasCustomConfig);
  const undoHistory = useProjectStore((s) => s.undoHistory);
  const refreshUndoState = useProjectStore((s) => s.refreshUndoState);
  const clearUndoHistory = useProjectStore((s) => s.clearUndoHistory);

  const [thumbCacheBytes, setThumbCacheBytes] = useState<number | null>(null);
  const [clearingCache, setClearingCache] = useState(false);
  const [clearingUndo, setClearingUndo] = useState(false);
  const [editingRules, setEditingRules] = useState(false);
  const [rulesError, setRulesError] = useState<string | null>(null);

  // Pull cache size whenever the modal opens. Fast (single readdir+stat
  // pass on the cache dir), no need to debounce or memoize.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    (async () => {
      try {
        const size = await invoke<number>("get_thumbnail_cache_size");
        if (!cancelled) setThumbCacheBytes(size);
      } catch (err) {
        console.error("Failed to read thumb cache size:", err);
        if (!cancelled) setThumbCacheBytes(null);
      }
    })();
    if (activeProjectId) refreshUndoState();
    return () => {
      cancelled = true;
    };
  }, [isOpen, activeProjectId, refreshUndoState]);

  const setLanguage = (lang: string) => {
    i18n.changeLanguage(lang);
    localStorage.setItem("language", lang);
  };

  const handleClearThumbCache = async () => {
    setClearingCache(true);
    try {
      await invoke<number>("clear_thumbnail_cache");
      setThumbCacheBytes(0);
    } catch (err) {
      console.error("Failed to clear thumbnail cache:", err);
    } finally {
      setClearingCache(false);
    }
  };

  const handleClearUndoHistory = async () => {
    if (!activeProjectId) return;
    setClearingUndo(true);
    try {
      await clearUndoHistory();
    } catch (err) {
      console.error("Failed to clear undo history:", err);
    } finally {
      setClearingUndo(false);
    }
  };

  // Open the project's tidycraft.toml in the OS default editor. Backend
  // creates the file from a commented template if it doesn't exist yet,
  // so the user always has something to start from.
  const handleEditRules = async () => {
    if (!activeProjectId) return;
    setEditingRules(true);
    setRulesError(null);
    try {
      const path = await invoke<string>("ensure_project_config", {
        projectId: activeProjectId,
      });
      setHasCustomConfig(true);
      await invoke("open_with_default_app", { path });
    } catch (err) {
      console.error("Failed to open rules editor:", err);
      setRulesError(String(err));
    } finally {
      setEditingRules(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-full max-w-md">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold">{t("settings.title")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-6">
          {/* Appearance Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Palette size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.appearanceSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <SegmentedControl<ThemePreference>
                value={preference}
                onChange={setPreference}
                label={t("settings.theme")}
                options={[
                  { value: "dark", label: t("settings.themeDark") },
                  { value: "light", label: t("settings.themeLight") },
                  { value: "system", label: t("settings.themeSystem") },
                ]}
              />
              <SegmentedControl<string>
                value={i18n.language}
                onChange={setLanguage}
                label={t("settings.language")}
                options={[
                  { value: "en", label: t("settings.english") },
                  { value: "zh", label: t("settings.chinese") },
                ]}
              />
            </div>
          </div>

          {/* Git Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <GitBranch size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.gitSection")}
              </h3>
            </div>
            <div className="space-y-4 pl-6">
              <ToggleSwitch
                checked={showBranchInfo}
                onChange={setShowBranchInfo}
                label={t("settings.showBranchInfo")}
                description={t("settings.showBranchInfoDesc")}
              />
              <ToggleSwitch
                checked={showAheadBehind}
                onChange={setShowAheadBehind}
                label={t("settings.showAheadBehind")}
                description={t("settings.showAheadBehindDesc")}
              />
              <ToggleSwitch
                checked={showGitStatusIndicators}
                onChange={setShowGitStatusIndicators}
                label={t("settings.showGitStatusIndicators")}
                description={t("settings.showGitStatusIndicatorsDesc")}
              />
            </div>
          </div>

          {/* Analysis Rules Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <FileCode size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.analysisRulesSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-medium text-text-primary">
                    {t("settings.analysisRulesEdit")}
                  </span>
                  <p className="text-xs text-text-secondary mt-0.5">
                    {!activeProjectId
                      ? t("settings.analysisRulesNoProject")
                      : hasCustomConfig
                      ? t("settings.analysisRulesCustom")
                      : t("settings.analysisRulesDefault")}
                  </p>
                  {rulesError && (
                    <p className="text-xs mt-1" style={{ color: "var(--err)" }}>
                      {rulesError}
                    </p>
                  )}
                </div>
                <button
                  onClick={handleEditRules}
                  disabled={!activeProjectId || editingRules}
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors inline-flex items-center gap-1"
                >
                  <ExternalLink size={11} />
                  {editingRules ? t("settings.opening") : t("settings.analysisRulesEditButton")}
                </button>
              </div>
            </div>
          </div>

          {/* Maintenance Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <Wrench size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.maintenanceSection")}
              </h3>
            </div>
            <div className="space-y-3 pl-6">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-2 flex-1 min-w-0">
                  <ImageIcon
                    size={14}
                    className="text-text-secondary mt-0.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-text-primary">
                      {t("settings.thumbnailCache")}
                    </span>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {thumbCacheBytes === null
                        ? t("settings.cacheSizeUnknown")
                        : t("settings.cacheSize", {
                            size: formatFileSize(thumbCacheBytes),
                          })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClearThumbCache}
                  disabled={
                    clearingCache ||
                    thumbCacheBytes === null ||
                    thumbCacheBytes === 0
                  }
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {clearingCache
                    ? t("settings.clearing")
                    : t("settings.clear")}
                </button>
              </div>

              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-2 flex-1 min-w-0">
                  <Trash2
                    size={14}
                    className="text-text-secondary mt-0.5 shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium text-text-primary">
                      {t("settings.undoHistory")}
                    </span>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {!activeProjectId
                        ? t("settings.undoNoProject")
                        : t("settings.undoEntries", {
                            count: undoHistory.length,
                          })}
                    </p>
                  </div>
                </div>
                <button
                  onClick={handleClearUndoHistory}
                  disabled={
                    clearingUndo ||
                    !activeProjectId ||
                    undoHistory.length === 0
                  }
                  className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  {clearingUndo
                    ? t("settings.clearing")
                    : t("settings.clear")}
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end px-4 py-3 border-t border-border">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium bg-primary text-white rounded hover:bg-primary/90 transition-colors"
          >
            {t("common.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
