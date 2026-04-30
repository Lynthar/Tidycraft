import { X, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { formatFileSize } from "../lib/utils";

export function StatusBar() {
  const { t } = useTranslation();
  const { scanResult, isScanning, error, scanProgress, cancelScan } =
    useProjectStore();

  if (error) {
    return (
      <footer className="tc-status" data-state="error">
        <span className="tc-status-err-icon">
          <AlertCircle size={13} />
        </span>
        <span style={{ fontSize: 11.5, fontWeight: 500 }}>
          {t("statusBar.error")}: {error}
        </span>
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

  return (
    <footer className="tc-status">
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
      {typeCounts.texture && (
        <span className="tc-status-item text-c-texture">
          {t("statusBar.textures")} {typeCounts.texture}
        </span>
      )}
      {typeCounts.model && (
        <span className="tc-status-item text-c-model">
          {t("statusBar.models")} {typeCounts.model}
        </span>
      )}
      {typeCounts.audio && (
        <span className="tc-status-item text-c-audio">
          {t("statusBar.audio")} {typeCounts.audio}
        </span>
      )}
    </footer>
  );
}
