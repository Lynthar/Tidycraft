import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
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
import { FileDown, Files, HardDrive, AlertTriangle, CheckCircle } from "lucide-react";
import { formatFileSize } from "../lib/utils";

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

const TYPE_COLORS: Record<string, string> = {
  texture: "#4ade80",
  model: "#60a5fa",
  audio: "#facc15",
  animation: "#a78bfa",
  material: "#f472b6",
  prefab: "#22d3d1",
  scene: "#fb923c",
  script: "#ef4444",
  data: "#94a3b8",
  other: "#6b7280",
};

const SIZE_ORDER = ["< 1 KB", "1-10 KB", "10-100 KB", "100 KB - 1 MB", "1-10 MB", "> 10 MB"];

interface StatsDashboardProps {
  issueCount?: number;
  passCount?: number;
  onExportJson?: () => void;
  onExportCsv?: () => void;
  onExportHtml?: () => void;
}

export function StatsDashboard({ issueCount = 0, passCount = 0, onExportJson, onExportCsv, onExportHtml }: StatsDashboardProps) {
  const { t } = useTranslation();
  const [stats, setStats] = useState<ProjectStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const loadStats = async () => {
      try {
        setLoading(true);
        const result = await invoke<ProjectStats>("get_project_stats");
        setStats(result);
        setError(null);
      } catch (err) {
        setError(String(err));
      } finally {
        setLoading(false);
      }
    };

    loadStats();
  }, []);

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
        {error || "No data available"}
      </div>
    );
  }

  // Prepare chart data
  const typeData = Object.entries(stats.type_distribution).map(([name, value]) => ({
    name: t(`assetTypes.${name}`),
    value,
    color: TYPE_COLORS[name] || "#6b7280",
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

  return (
    <div className="h-full overflow-auto p-4 space-y-4">
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
              <p className="text-xs text-text-secondary">Passed</p>
            </div>
          </div>
        </div>
      </div>

      {/* Charts Row */}
      <div className="grid grid-cols-2 gap-4">
        {/* Type Distribution Pie Chart */}
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <h3 className="text-sm font-medium mb-4">Type Distribution</h3>
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
                contentStyle={{
                  backgroundColor: "#1e1e2e",
                  border: "1px solid #313244",
                  borderRadius: "8px",
                }}
                formatter={(value) => [value, "Count"]}
              />
              <Legend />
            </PieChart>
          </ResponsiveContainer>
        </div>

        {/* Size Distribution Bar Chart */}
        <div className="bg-card-bg border border-border rounded-lg p-4">
          <h3 className="text-sm font-medium mb-4">Size Distribution</h3>
          <ResponsiveContainer width="100%" height={200}>
            <BarChart data={sizeData}>
              <XAxis dataKey="name" tick={{ fontSize: 10 }} />
              <YAxis tick={{ fontSize: 10 }} />
              <Tooltip
                contentStyle={{
                  backgroundColor: "#1e1e2e",
                  border: "1px solid #313244",
                  borderRadius: "8px",
                }}
              />
              <Bar dataKey="count" fill="#60a5fa" radius={[4, 4, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </div>

      {/* Extension Distribution */}
      <div className="bg-card-bg border border-border rounded-lg p-4">
        <h3 className="text-sm font-medium mb-4">Top Extensions</h3>
        <ResponsiveContainer width="100%" height={150}>
          <BarChart data={extensionData} layout="vertical">
            <XAxis type="number" tick={{ fontSize: 10 }} />
            <YAxis dataKey="name" type="category" tick={{ fontSize: 10 }} width={50} />
            <Tooltip
              contentStyle={{
                backgroundColor: "#1e1e2e",
                border: "1px solid #313244",
                borderRadius: "8px",
              }}
            />
            <Bar dataKey="count" fill="#a78bfa" radius={[0, 4, 4, 0]} />
          </BarChart>
        </ResponsiveContainer>
      </div>

      {/* Largest Files */}
      <div className="bg-card-bg border border-border rounded-lg p-4">
        <h3 className="text-sm font-medium mb-4">Largest Files</h3>
        <div className="space-y-2">
          {stats.largest_files.slice(0, 5).map((file, index) => (
            <div key={index} className="flex items-center justify-between text-sm">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-text-secondary">{index + 1}.</span>
                <span className="truncate">{file.name}</span>
                <span
                  className="px-1.5 py-0.5 text-[10px] rounded"
                  style={{ backgroundColor: `${TYPE_COLORS[file.asset_type]}20`, color: TYPE_COLORS[file.asset_type] }}
                >
                  {file.asset_type}
                </span>
              </div>
              <span className="text-text-secondary shrink-0">{formatFileSize(file.size)}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Export Buttons */}
      <div className="flex gap-2">
        <button
          onClick={onExportJson}
          className="flex items-center gap-2 px-4 py-2 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
        >
          <FileDown size={16} />
          Export JSON
        </button>
        <button
          onClick={onExportCsv}
          className="flex items-center gap-2 px-4 py-2 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
        >
          <FileDown size={16} />
          Export CSV
        </button>
        <button
          onClick={onExportHtml}
          className="flex items-center gap-2 px-4 py-2 bg-green-500/20 text-green-400 rounded hover:bg-green-500/30 transition-colors"
        >
          <FileDown size={16} />
          Export HTML Report
        </button>
      </div>
    </div>
  );
}
