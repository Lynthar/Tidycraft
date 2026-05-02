import { useEffect, useRef, useState } from "react";
import { X, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

/// How long the watcher "syncing" badge stays visible after the most recent
/// fs-change event. Coalescing window in `watcher.rs` is 500ms, so this
/// sits just above it: each event burst flashes once, back-to-back bursts
/// keep the badge lit continuously.
const WATCHER_PULSE_DURATION_MS = 700;

export function StatusBar() {
  const { t } = useTranslation();
  const {
    scanResult,
    isScanning,
    error,
    scanProgress,
    cancelScan,
    projectPath,
    openProject,
    clearError,
  } = useProjectStore();
  const watcherPulse = useProjectStore((s) => s.watcherPulse);

  const [syncing, setSyncing] = useState(false);
  const timerRef = useRef<number | null>(null);
  useEffect(() => {
    if (watcherPulse === 0) return; // never fired
    setSyncing(true);
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
    }
    timerRef.current = window.setTimeout(() => {
      setSyncing(false);
      timerRef.current = null;
    }, WATCHER_PULSE_DURATION_MS);
    return () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [watcherPulse]);

  if (error) {
    const handleRetry = () => {
      if (projectPath) openProject(projectPath, { force: true });
    };
    return (
      <footer className="tc-status" data-state="error">
        <span className="tc-status-err-icon">
          <AlertCircle size={13} />
        </span>
        <span style={{ fontSize: 11.5, fontWeight: 500 }}>
          {t("statusBar.error")}: {error}
        </span>
        <span className="tc-status-spacer" />
        {projectPath && (
          <button
            onClick={handleRetry}
            className="tc-scan-cancel"
            style={{ color: "var(--err)" }}
          >
            {t("statusBar.retry")}
          </button>
        )}
        <button onClick={clearError} className="tc-scan-cancel">
          {t("statusBar.dismiss")}
        </button>
      </footer>
    );
  }

  if (isScanning && scanProgress) {
    const progressPercent =
      scanProgress.total && scanProgress.total > 0
        ? Math.round((scanProgress.current / scanProgress.total) * 100)
        : 0;

    const currentFile = scanProgress.current_file
      ? scanProgress.current_file.split("/").pop() || scanProgress.current_file
      : "";

    return (
      <footer className="tc-status" data-state="scanning">
        <span className="tc-scan-phase">{t(`scanPhase.${scanProgress.phase}`)}…</span>
        {scanProgress.total && scanProgress.total > 0 && (
          <>
            <span className="mono" style={{ color: "var(--text-2)" }}>
              {scanProgress.current} / {scanProgress.total}{" "}
              <span style={{ color: "var(--text-3)" }}>({progressPercent}%)</span>
            </span>
            <span
              className="tc-scan-progress"
              style={{ ["--p" as string]: `${progressPercent}%` } as React.CSSProperties}
            >
              <i />
            </span>
          </>
        )}
        {currentFile && <span className="tc-scan-file">…/{currentFile}</span>}
        <span className="tc-status-spacer" />
        <button onClick={cancelScan} className="tc-scan-cancel">
          <X size={12} />
          {t("statusBar.cancel")}
        </button>
      </footer>
    );
  }

  if (isScanning) {
    return (
      <footer className="tc-status" data-state="scanning">
        <span className="tc-scan-phase">{t("statusBar.startingScan")}</span>
        <span className="tc-status-spacer" />
        <button onClick={cancelScan} className="tc-scan-cancel">
          <X size={12} />
          {t("statusBar.cancel")}
        </button>
      </footer>
    );
  }

  if (!scanResult) {
    return <footer className="tc-status">{t("statusBar.ready")}</footer>;
  }

  const typeCounts = scanResult.type_counts;
  const projectType = scanResult.project_type;
  // Sort by count desc so the mini-bar's largest segments render first
  // (left-to-right) and the inline legend picks the two most prevalent
  // types — useful at a glance for "what's this project mostly made of".
  const distribution = Object.entries(typeCounts)
    .filter(([, count]) => count > 0)
    .sort((a, b) => b[1] - a[1]);
  const totalForRatio = scanResult.total_count || 1;

  return (
    <footer className="tc-status">
      <span className="tc-status-pulse" aria-hidden />
      <span className="tc-status-item">{t("statusBar.connected")}</span>
      <span style={{ color: "var(--line-strong)" }}>·</span>
      {projectType && projectType !== "generic" && (
        <span
          className="tc-status-item"
          style={{
            padding: "1px 6px",
            borderRadius: 3,
            background: "color-mix(in oklch, var(--primary) 18%, transparent)",
            color: "var(--primary-strong)",
            fontSize: 10,
            textTransform: "uppercase",
            fontWeight: 600,
            letterSpacing: "0.04em",
          }}
        >
          {projectType}
        </span>
      )}
      <span className="tc-status-item">
        {t("statusBar.total")}{" "}
        <span className="tc-num">
          {scanResult.total_count} {t("statusBar.assets")}
        </span>
      </span>
      <span className="tc-status-item">
        {t("assetList.size")} <span className="tc-num">{formatFileSize(scanResult.total_size)}</span>
      </span>
      {distribution.length > 0 && (
        <span className="tc-stats-mini">
          <span className="tc-stats-bar" aria-hidden>
            {distribution.map(([type, count]) => (
              <i
                key={type}
                style={{
                  width: `${(count / totalForRatio) * 100}%`,
                  background: `var(--c-${type})`,
                }}
              />
            ))}
          </span>
          {distribution.slice(0, 2).map(([type, count]) => (
            <span
              key={type}
              style={{
                color: `var(--c-${type})`,
                display: "inline-flex",
                alignItems: "center",
              }}
            >
              <span
                className="tc-legend-dot"
                style={{ background: `var(--c-${type})` }}
              />
              {t(`assetTypes.${type}` as const)} {count}
            </span>
          ))}
        </span>
      )}
      {syncing && (
        <span className="tc-watcher-pulse" title={t("statusBar.syncing")}>
          {t("statusBar.syncing")}
        </span>
      )}
    </footer>
  );
}
