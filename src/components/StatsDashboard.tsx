import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useThemeStore } from "../stores/themeStore";
import {
  PieChart,
  Pie,
  Cell,
  BarChart,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import { FileDown, Files, HardDrive, AlertTriangle, CheckCircle, Unlink } from "lucide-react";
import { formatFileSize } from "../lib/utils";
import { basename } from "../lib/pathUtils";
import type {
  GodotProjectInfo,
  UnityProjectInfo,
  UnrealProjectInfo,
} from "../types/asset";

interface ProjectStats {
  total_assets: number;
  total_size: number;
  type_distribution: Record<string, number>;
  size_distribution: Record<string, number>;
  extension_distribution: Record<string, number>;
  largest_files: Array<{
    name: string;
    path: string;
    size: number;
    asset_type: string;
  }>;
  directory_sizes: Record<string, number>;
}

// Asset-type palette keys — mirror the `--c-<type>` design tokens
// (src/styles/redesign-tokens-v2.css) so the charts share the app's colors
// instead of a private hex set.
const ASSET_TYPE_KEYS = [
  "texture", "model", "audio", "video", "animation", "material",
  "prefab", "scene", "script", "data", "other",
] as const;

interface ChartColors {
  /// `--c-<type>` resolved to a concrete value, keyed by asset type.
  types: Record<string, string>;
  primary: string;
  accent: string;
  fallback: string;
}

/// Resolve the design tokens the charts need into concrete color strings.
/// recharts writes `fill` as an SVG presentation attribute, which — unlike an
/// inline `style` — does NOT resolve CSS `var()`, so we hand it the computed
/// `oklch(...)` literals instead. data-theme lives on <html> and custom
/// properties inherit, so reading documentElement is correct in both themes.
function resolveChartColors(): ChartColors {
  const cs = getComputedStyle(document.documentElement);
  const read = (name: string) => cs.getPropertyValue(name).trim();
  const types: Record<string, string> = {};
  for (const k of ASSET_TYPE_KEYS) types[k] = read(`--c-${k}`);
  return {
    types,
    primary: read("--primary"),
    accent: read("--accent"),
    fallback: read("--c-other"),
  };
}

// Shared recharts tooltip styling — token-driven so it theme-flips (these are
// inline styles, where `var()` DOES resolve). Replaces the hardcoded dark
// `#1e1e2e` box that read as a patch in light mode.
const TOOLTIP_CONTENT_STYLE = {
  backgroundColor: "var(--panel-2)",
  border: "1px solid var(--line)",
  borderRadius: "8px",
  color: "var(--text)",
};
const TOOLTIP_LABEL_STYLE = { color: "var(--text)" };
const TOOLTIP_ITEM_STYLE = { color: "var(--text-2)" };

const SIZE_ORDER = ["< 1 KB", "1-10 KB", "10-100 KB", "100 KB - 1 MB", "1-10 MB", "> 10 MB"];

/// Which engine card the project gets, tagged so render can switch on it.
/// `null` (no marker file / unparseable) simply hides the card.
type EngineInfo =
  | { kind: "unity"; info: UnityProjectInfo }
  | { kind: "godot"; info: GodotProjectInfo }
  | { kind: "unreal"; info: UnrealProjectInfo };

// Engine names are brands, not translatable strings.
const ENGINE_NAMES: Record<EngineInfo["kind"], string> = {
  unity: "Unity",
  godot: "Godot",
  unreal: "Unreal Engine",
};

/// One label/value line in the engine card. Callers skip empty values.
function EngineRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-4 min-w-0 text-sm">
      <span className="text-xs text-text-secondary shrink-0">{label}</span>
      <span className="truncate text-right" title={value}>
        {value}
      </span>
    </div>
  );
}

interface StatsDashboardProps {
  issueCount?: number;
  passCount?: number;
  onExportJson?: () => void;
  onExportCsv?: () => void;
  onExportHtml?: () => void;
}

export function StatsDashboard({ issueCount = 0, passCount = 0, onExportJson, onExportCsv, onExportHtml }: StatsDashboardProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const scanResult = useProjectStore((s) => s.scanResult);
  const locateAsset = useProjectStore((s) => s.locateAsset);
  const [stats, setStats] = useState<ProjectStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Unused-assets panel: lazy, button-triggered. `find_unused_assets` parses
  // every prefab/scene/material for GUID refs, so it's too heavy to run on
  // every Stats open — the user scans on demand. Unity-only for now (the
  // backend rejects non-Unity projects); Godot support is a later step.
  const projectType = scanResult?.project_type;
  const [unused, setUnused] = useState<string[] | null>(null);
  const [unusedLoading, setUnusedLoading] = useState(false);
  const [unusedError, setUnusedError] = useState<string | null>(null);

  // Chart palette resolved from the design tokens, recomputed on theme flip
  // so the charts share the app's colors (recharts SVG fills can't consume
  // var(); see resolveChartColors). applyTheme sets data-theme before the
  // store updates, so this reads the new theme's values on re-render.
  const theme = useThemeStore((s) => s.theme);
  const chartColors = useMemo(() => resolveChartColors(), [theme]);

  // Engine info card: cheap marker-file parse backend-side
  // (ProjectVersion.txt / project.godot / *.uproject). `null` hides the
  // card; stale-response guard mirrors the stats fetch below.
  const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
  const rootPath = scanResult?.root_path;
  useEffect(() => {
    if (!rootPath || !projectType || projectType === "generic") {
      setEngineInfo(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        if (projectType === "unity") {
          const info = await invoke<UnityProjectInfo | null>(
            "get_unity_project_info",
            { rootPath }
          );
          if (!cancelled) setEngineInfo(info ? { kind: "unity", info } : null);
        } else if (projectType === "godot") {
          const info = await invoke<GodotProjectInfo | null>(
            "get_godot_project_info",
            { rootPath }
          );
          if (!cancelled) setEngineInfo(info ? { kind: "godot", info } : null);
        } else if (projectType === "unreal") {
          const info = await invoke<UnrealProjectInfo | null>(
            "get_unreal_project_info",
            { rootPath }
          );
          if (!cancelled) setEngineInfo(info ? { kind: "unreal", info } : null);
        }
      } catch {
        if (!cancelled) setEngineInfo(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [rootPath, projectType]);

  const scanUnused = async () => {
    if (!activeProjectId) return;
    // Snapshot the target project: find_unused_assets is heavy, so the user may
    // switch projects mid-scan. Ignore the result if they did, so we never write
    // one project's unused list into another's view.
    const pid = activeProjectId;
    setUnusedLoading(true);
    setUnusedError(null);
    try {
      const result = await invoke<string[]>("find_unused_assets", {
        projectId: pid,
      });
      if (useProjectStore.getState().activeProjectId === pid) setUnused(result);
    } catch (err) {
      if (useProjectStore.getState().activeProjectId === pid) setUnusedError(String(err));
    } finally {
      setUnusedLoading(false);
    }
  };

  useEffect(() => {
    // Reset the on-demand unused-assets panel when the project changes so we
    // never show a previous project's result. Keyed on the project only —
    // watcher-driven scanResult refreshes must NOT wipe a user-requested
    // unused scan (it goes stale, but stale beats vanishing mid-read).
    setUnused(null);
    setUnusedError(null);
  }, [activeProjectId]);

  useEffect(() => {
    if (!activeProjectId) {
      setStats(null);
      setLoading(false);
      return;
    }
    // Guard against a stale get_project_stats resolving after the user switched
    // projects: the cleanup flips `cancelled` so a late response can't overwrite
    // the now-active project's stats.
    let cancelled = false;
    const loadStats = async () => {
      try {
        setLoading(true);
        const result = await invoke<ProjectStats>("get_project_stats", {
          projectId: activeProjectId,
        });
        if (cancelled) return;
        setStats(result);
        setError(null);
      } catch (err) {
        if (!cancelled) setError(String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    loadStats();
    return () => {
      cancelled = true;
    };
    // `scanResult` is a dependency so watcher file changes and Ctrl+R rescans
    // (same project id, new result object) refetch — totals/charts/largest
    // files used to freeze at first render while the sibling passCount card
    // kept updating. During a forced rescan the mirror briefly goes null; the
    // extra fetch is harmless (the backend serves its last cached scan).
  }, [activeProjectId, scanResult]);

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        {t("assetPreview.loading")}
      </div>
    );
  }

  if (error || !stats) {
    return (
      <div className="flex items-center justify-center h-full text-text-secondary">
        {error || t("stats.noData")}
      </div>
    );
  }

  // Prepare chart data
  const typeData = Object.entries(stats.type_distribution).map(([name, value]) => ({
    name: t(`assetTypes.${name}`),
    value,
    color: chartColors.types[name] || chartColors.fallback,
  }));

  const sizeData = SIZE_ORDER
    .filter((bucket) => stats.size_distribution[bucket])
    .map((bucket) => ({
      name: bucket,
      count: stats.size_distribution[bucket] || 0,
    }));

  // Top extensions
  const extensionData = Object.entries(stats.extension_distribution)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8)
    .map(([name, value]) => ({
      name: `.${name}`,
      count: value,
    }));

  // Engine card content: a version chip + label/value rows, per engine.
  // Empty/absent fields are skipped rather than rendered as "-".
  const engineRows: Array<{ label: string; value: string }> = [];
  let engineVersion: string | undefined;
  let engineVersionTitle: string | undefined;
  if (engineInfo?.kind === "unity") {
    engineVersion = engineInfo.info.editor_version;
    // Full changeset revision lives in the tooltip.
    engineVersionTitle = engineInfo.info.editor_version_with_revision;
  } else if (engineInfo?.kind === "godot") {
    const g = engineInfo.info;
    engineVersion = g.godot_version;
    engineRows.push({ label: t("stats.engineProject"), value: g.project_name });
    if (g.main_scene)
      engineRows.push({ label: t("stats.engineMainScene"), value: g.main_scene });
    if (g.renderer)
      engineRows.push({ label: t("stats.engineRenderer"), value: g.renderer });
    if (g.autoloads.length > 0)
      engineRows.push({
        label: t("stats.engineAutoloads"),
        value: g.autoloads.map((a) => a.name).join(", "),
      });
    if (g.features.length > 0)
      engineRows.push({
        label: t("stats.engineFeatures"),
        value: g.features.join(", "),
      });
  } else if (engineInfo?.kind === "unreal") {
    const u = engineInfo.info;
    engineVersion = u.engine_association;
    engineRows.push({ label: t("stats.engineProject"), value: u.project_name });
    if (u.modules.length > 0)
      engineRows.push({
        label: t("stats.engineModules"),
        value: u.modules.map((m) => m.name).join(", "),
      });
    if (u.plugins.length > 0)
      engineRows.push({
        label: t("stats.enginePlugins"),
        value: `${u.plugins.filter((p) => p.enabled).length} / ${u.plugins.length}`,
      });
    if (u.target_platforms.length > 0)
      engineRows.push({
        label: t("stats.enginePlatforms"),
        value: u.target_platforms.join(", "),
      });
  }

  return (
    <div className="h-full overflow-auto p-4 space-y-4">
      {/* Engine card: identity + key config from the project's marker file */}
      {engineInfo && (
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className={`flex items-center gap-2 ${engineRows.length > 0 ? "mb-3" : ""}`}>
            <h3 className="text-sm font-medium">
              {ENGINE_NAMES[engineInfo.kind]}
            </h3>
            {engineVersion && (
              <span
                className="px-1.5 py-0.5 text-[10px] font-mono rounded bg-primary/20 text-primary"
                title={engineVersionTitle}
              >
                {engineVersion}
              </span>
            )}
          </div>
          {engineRows.length > 0 && (
            <div className="grid grid-cols-2 gap-x-8 gap-y-1">
              {engineRows.map((row) => (
                <EngineRow key={row.label} label={row.label} value={row.value} />
              ))}
            </div>
          )}
        </div>
      )}

      {/* Summary Cards */}
      <div className="grid grid-cols-4 gap-4">
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-primary/20 rounded-lg">
              <Files className="text-primary" size={24} />
            </div>
            <div>
              <p className="text-2xl font-bold text-text-primary">{stats.total_assets.toLocaleString()}</p>
              <p className="text-xs text-text-secondary">{t("statusBar.total")} {t("statusBar.assets")}</p>
            </div>
          </div>
        </div>

        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-blue-500/20 rounded-lg">
              <HardDrive className="text-blue-400" size={24} />
            </div>
            <div>
              <p className="text-2xl font-bold text-text-primary">{formatFileSize(stats.total_size)}</p>
              <p className="text-xs text-text-secondary">{t("assetList.size")}</p>
            </div>
          </div>
        </div>

        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-warning/20 rounded-lg">
              <AlertTriangle className="text-warning" size={24} />
            </div>
            <div>
              <p className="text-2xl font-bold text-text-primary">{issueCount}</p>
              <p className="text-xs text-text-secondary">{t("issues.title")}</p>
            </div>
          </div>
        </div>

        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className="flex items-center gap-3">
            <div className="p-2 bg-green-500/20 rounded-lg">
              <CheckCircle className="text-green-400" size={24} />
            </div>
            <div>
              <p className="text-2xl font-bold text-text-primary">{passCount}</p>
              <p className="text-xs text-text-secondary">{t("stats.passed")}</p>
            </div>
          </div>
        </div>
      </div>

      {/* Charts Row */}
      <div className="grid grid-cols-2 gap-4">
        {/* Type Distribution Pie Chart */}
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <h3 className="text-sm font-medium mb-4">{t("stats.typeDistribution")}</h3>
          <ResponsiveContainer width="100%" height={200}>
            <PieChart>
              <Pie
                data={typeData}
                cx="50%"
                cy="50%"
                innerRadius={50}
                outerRadius={80}
                paddingAngle={2}
                dataKey="value"
              >
                {typeData.map((entry, index) => (
                  <Cell key={`cell-${index}`} fill={entry.color} />
                ))}
              </Pie>
              <Tooltip
                contentStyle={TOOLTIP_CONTENT_STYLE}
                labelStyle={TOOLTIP_LABEL_STYLE}
                itemStyle={TOOLTIP_ITEM_STYLE}
                formatter={(value) => [value, t("stats.count")]}
              />
              <Legend />
            </PieChart>
          </ResponsiveContainer>
        </div>

        {/* Size Distribution Bar Chart */}
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <h3 className="text-sm font-medium mb-4">{t("stats.sizeDistribution")}</h3>
          <ResponsiveContainer width="100%" height={200}>
            <BarChart data={sizeData}>
              <XAxis dataKey="name" tick={{ fontSize: 10 }} />
              <YAxis tick={{ fontSize: 10 }} />
              <Tooltip
                contentStyle={TOOLTIP_CONTENT_STYLE}
                labelStyle={TOOLTIP_LABEL_STYLE}
                itemStyle={TOOLTIP_ITEM_STYLE}
              />
              <Bar dataKey="count" fill={chartColors.primary} radius={[4, 4, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>

      {/* Extension Distribution */}
      <div className="bg-card-bg border border-border rounded-lg p-4">
        <h3 className="text-sm font-medium mb-4">{t("stats.topExtensions")}</h3>
        <ResponsiveContainer width="100%" height={150}>
          <BarChart data={extensionData} layout="vertical" margin={{ top: 8, right: 12, bottom: 2, left: 0 }}>
            <XAxis type="number" tick={{ fontSize: 10 }} />
            <YAxis dataKey="name" type="category" tick={{ fontSize: 10 }} width={50} interval={0} />
            <Tooltip
              contentStyle={TOOLTIP_CONTENT_STYLE}
              labelStyle={TOOLTIP_LABEL_STYLE}
              itemStyle={TOOLTIP_ITEM_STYLE}
            />
            <Bar dataKey="count" fill={chartColors.accent} radius={[0, 4, 4, 0]} />
          </BarChart>
        </ResponsiveContainer>
      </div>

      {/* Largest Files */}
      <div className="bg-card-bg border border-border rounded-lg p-4">
        <h3 className="text-sm font-medium mb-4">{t("stats.largestFiles")}</h3>
        <div className="space-y-2">
          {stats.largest_files.slice(0, 5).map((file, index) => (
            <div key={index} className="flex items-center justify-between text-sm">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-text-secondary">{index + 1}.</span>
                <span className="truncate">{file.name}</span>
                <span
                  className="px-1.5 py-0.5 text-[10px] rounded"
                  style={{
                    backgroundColor: `color-mix(in oklab, var(--c-${file.asset_type}, var(--c-other)) 15%, transparent)`,
                    color: `var(--c-${file.asset_type}, var(--c-other))`,
                  }}
                >
                  {file.asset_type}
                </span>
              </div>
              <span className="text-text-secondary shrink-0">{formatFileSize(file.size)}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Unused Assets (Unity + Godot; on-demand scan) */}
      {(projectType === "unity" || projectType === "godot") && (
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <div className="flex items-center justify-between mb-1">
            <h3 className="text-sm font-medium flex items-center gap-2">
              <Unlink size={14} />
              {t("stats.unusedAssets")}
              {unused !== null && (
                <span className="text-text-secondary">· {unused.length}</span>
              )}
            </h3>
            <button
              onClick={scanUnused}
              disabled={unusedLoading}
              className="flex items-center gap-1.5 px-3 py-1.5 text-xs bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors disabled:opacity-50"
            >
              {unusedLoading
                ? t("stats.scanningUnused")
                : unused === null
                ? t("stats.scanUnused")
                : t("stats.rescanUnused")}
            </button>
          </div>
          <p className="text-[11px] text-text-secondary mb-3">
            {projectType === "godot"
              ? t("stats.unusedAssetsHintGodot")
              : t("stats.unusedAssetsHint")}
          </p>
          {unusedError ? (
            <p className="text-xs text-warning">{unusedError}</p>
          ) : unused === null ? null : unused.length === 0 ? (
            <p className="text-xs text-text-secondary">{t("stats.noUnusedAssets")}</p>
          ) : (
            <div className="space-y-1 max-h-64 overflow-auto">
              {unused.map((path) => (
                <div
                  key={path}
                  className="flex items-center justify-between gap-2 text-sm"
                >
                  <span className="truncate" title={path}>
                    {basename(path)}
                  </span>
                  <button
                    onClick={() => locateAsset(path)}
                    className="shrink-0 text-xs text-primary hover:underline"
                  >
                    {t("issues.locate")}
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Export Buttons */}
      <div className="flex gap-2">
        <button
          onClick={onExportJson}
          className="flex items-center gap-2 px-4 py-2 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
        >
          <FileDown size={16} />
          {t("stats.exportJson")}
        </button>
        <button
          onClick={onExportCsv}
          className="flex items-center gap-2 px-4 py-2 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
        >
          <FileDown size={16} />
          {t("stats.exportCsv")}
        </button>
        <button
          onClick={onExportHtml}
          className="flex items-center gap-2 px-4 py-2 bg-green-500/20 text-green-400 rounded hover:bg-green-500/30 transition-colors"
        >
          <FileDown size={16} />
          {t("stats.exportHtml")}
        </button>
      </div>
    </div>
  );
}
