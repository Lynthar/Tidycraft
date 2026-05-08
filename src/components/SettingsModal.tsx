import { useEffect, useState } from "react";
import { X, GitBranch, Palette, Wrench, Trash2, Image as ImageIcon, FileCode, ExternalLink } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
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

/// Editable list of (extension → editor path) mappings. Reads/writes
/// `settingsStore.externalEditors`; persisting is immediate (each row
/// edit calls `setExternalEditor` / `removeExternalEditor`). The "draft"
/// row holds the in-progress new mapping so the user can pick path via
/// `Browse…` before the mapping appears in the live list — keeps the
/// store from churning on incomplete keystrokes.
function ExternalEditorsSection() {
  const { t } = useTranslation();
  const externalEditors = useSettingsStore((s) => s.externalEditors);
  const setExternalEditor = useSettingsStore((s) => s.setExternalEditor);
  const removeExternalEditor = useSettingsStore((s) => s.removeExternalEditor);

  const [draft, setDraft] = useState<{ ext: string; path: string } | null>(null);

  // Single-file picker without filters — the user might point at any
  // launcher shape (.exe, .app bundle, shell script, .desktop entry).
  // Validation happens at launch time via tauri-plugin-opener.
  const pickEditorPath = async (): Promise<string | null> => {
    try {
      const selected = await open({ multiple: false });
      return typeof selected === "string" ? selected : null;
    } catch (err) {
      console.error("Failed to pick editor:", err);
      return null;
    }
  };

  const entries = Object.entries(externalEditors).sort(([a], [b]) =>
    a.localeCompare(b)
  );

  const draftReady =
    draft !== null &&
    draft.ext.trim().length > 0 &&
    draft.path.trim().length > 0;

  return (
    <div className="space-y-2 pl-6">
      {entries.length === 0 && !draft && (
        <p className="text-xs italic" style={{ color: "var(--text-3)" }}>
          {t("settings.noExternalEditors")}
        </p>
      )}

      {entries.map(([ext, editorPath]) => (
        <div key={ext} className="flex items-center gap-2">
          <code
            className="text-xs px-2 py-1 rounded font-mono shrink-0"
            style={{
              background: "var(--panel-2)",
              border: "1px solid var(--line)",
              minWidth: 60,
              textAlign: "center",
            }}
          >
            {ext}
          </code>
          <span
            className="text-xs flex-1 truncate"
            style={{ color: "var(--text-3)" }}
            title={editorPath}
          >
            {editorPath}
          </span>
          <button
            onClick={async () => {
              const newPath = await pickEditorPath();
              if (newPath) setExternalEditor(ext, newPath);
            }}
            className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors shrink-0"
          >
            {t("settings.editorBrowse")}
          </button>
          <button
            onClick={() => removeExternalEditor(ext)}
            className="p-1 rounded hover:bg-background transition-colors shrink-0"
            title={t("settings.editorRemove")}
            style={{ color: "var(--text-3)" }}
          >
            <Trash2 size={12} />
          </button>
        </div>
      ))}

      {draft && (
        <div
          className="flex items-center gap-2 p-2 rounded"
          style={{
            background: "var(--panel-2)",
            border: "1px dashed var(--line)",
          }}
        >
          <input
            type="text"
            value={draft.ext}
            onChange={(e) => setDraft({ ...draft, ext: e.target.value })}
            placeholder={t("settings.editorExtensionPlaceholder")}
            autoFocus
            className="text-xs px-2 py-1 rounded font-mono"
            style={{
              background: "var(--panel)",
              border: "1px solid var(--line)",
              color: "var(--text)",
              width: 70,
            }}
          />
          <span
            className="text-xs flex-1 truncate"
            style={{ color: draft.path ? "var(--text-3)" : "var(--text-4)" }}
            title={draft.path || ""}
          >
            {draft.path || t("settings.editorPathPlaceholder")}
          </span>
          <button
            onClick={async () => {
              const newPath = await pickEditorPath();
              if (newPath) setDraft({ ...draft, path: newPath });
            }}
            className="px-2 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors shrink-0"
          >
            {t("settings.editorBrowse")}
          </button>
          <button
            onClick={() => {
              if (!draftReady || !draft) return;
              setExternalEditor(draft.ext, draft.path);
              setDraft(null);
            }}
            disabled={!draftReady}
            className="px-3 py-1 text-xs rounded transition-colors shrink-0"
            style={{
              background: draftReady ? "var(--primary)" : "var(--panel-2)",
              color: draftReady ? "var(--on-primary)" : "var(--text-4)",
              border: "1px solid var(--line)",
              cursor: draftReady ? "pointer" : "not-allowed",
            }}
          >
            ✓
          </button>
          <button
            onClick={() => setDraft(null)}
            className="p-1 rounded hover:bg-background text-text-secondary transition-colors shrink-0"
          >
            <X size={12} />
          </button>
        </div>
      )}

      {!draft && (
        <button
          onClick={() => setDraft({ ext: "", path: "" })}
          className="px-3 py-1 text-xs rounded border border-border hover:bg-background text-text-secondary hover:text-text-primary transition-colors inline-flex items-center gap-1"
        >
          + {t("settings.addEditor")}
        </button>
      )}
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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      {/* `max-h-[90vh]` + flex column lets the body scroll when content
          exceeds viewport height. Header + footer stay pinned. Without
          this, opening Settings on a non-fullscreen window (or after we
          add more sections) clips the bottom (Maintenance / Done button
          becomes unreachable). */}
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-full max-w-md flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border shrink-0">
          <h2 className="text-lg font-semibold">{t("settings.title")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content — scrolls when overflowing the viewport-bound modal. */}
        <div className="p-4 space-y-6 overflow-y-auto flex-1 min-h-0">
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

          {/* External Editors Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <ExternalLink size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.externalEditorsSection")}
              </h3>
            </div>
            <p
              className="text-xs pl-6 mb-3"
              style={{ color: "var(--text-3)" }}
            >
              {t("settings.externalEditorsHint")}
            </p>
            <ExternalEditorsSection />
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
        <div className="flex justify-end px-4 py-3 border-t border-border shrink-0">
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
