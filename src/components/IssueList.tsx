import { useMemo, useState } from "react";
import { AlertCircle, AlertTriangle, Info, FileWarning, Layers, Download } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import type { Issue, Severity, AnalysisResult } from "../types/asset";

const SEV_TO_TONE: Record<Severity, "err" | "warn" | "info"> = {
  error: "err",
  warning: "warn",
  info: "info",
};

/// Map rule_id to a fix-action verb. The button click still calls onLocate
/// (the actual fix is manual — we just open the asset for the user); only
/// the label changes to suggest *what* the fix is. Falls back to "Review".
function fixLabelKey(ruleId: string): string {
  if (ruleId.startsWith("naming.")) return "issues.fix.rename";
  if (
    ruleId === "texture.max_size" ||
    ruleId === "texture.min_size" ||
    ruleId === "texture.pot" ||
    ruleId === "texture.non_square"
  )
    return "issues.fix.resize";
  if (ruleId === "model.vertices" || ruleId === "model.faces")
    return "issues.fix.decimate";
  if (ruleId === "missing_reference") return "issues.fix.locate";
  return "issues.fix.review";
}

function SeverityIcon({ severity }: { severity: Severity }) {
  switch (severity) {
    case "error":
      return <AlertCircle size={13} />;
    case "warning":
      return <AlertTriangle size={13} />;
    case "info":
      return <Info size={13} />;
  }
}

interface IssueRowProps {
  issue: Issue;
  onLocate?: (path: string) => void;
  suggestionLabel: string;
  locateLabel: string;
}

function IssueRow({ issue, onLocate, suggestionLabel, locateLabel }: IssueRowProps) {
  const [expanded, setExpanded] = useState(false);
  const fileName = issue.asset_path.split("/").pop() || issue.asset_path;
  const tone = SEV_TO_TONE[issue.severity];

  return (
    <div
      className="tc-issue-row"
      data-expanded={expanded ? "true" : undefined}
      onClick={() => setExpanded(!expanded)}
    >
      <span className="tc-issue-icon" data-sev={tone}>
        <SeverityIcon severity={issue.severity} />
      </span>
      <div className="tc-issue-body">
        <div className="tc-issue-title">
          {issue.rule_name}
          <span className="tc-issue-rule-id">{issue.rule_id}</span>
        </div>
        <div className="tc-issue-meta">
          <strong>{fileName}</strong>
          <span style={{ color: "var(--text-4)" }}>·</span>
          <span>{issue.asset_path}</span>
        </div>
        {expanded && (
          <div className="tc-issue-detail" onClick={(e) => e.stopPropagation()}>
            <div>{issue.message}</div>
            {issue.suggestion && (
              <div>
                <span className="tc-issue-suggestion">{suggestionLabel}:</span>{" "}
                {issue.suggestion}
              </div>
            )}
          </div>
        )}
      </div>
      {onLocate && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onLocate(issue.asset_path);
          }}
          className="tc-issue-fix"
        >
          {locateLabel}
        </button>
      )}
    </div>
  );
}

interface IssueListProps {
  result: AnalysisResult | null;
  isAnalyzing?: boolean;
  onAnalyze?: () => void;
  onLocate?: (path: string) => void;
}

export function IssueList({ result, isAnalyzing, onAnalyze, onLocate }: IssueListProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const [filter, setFilter] = useState<Severity | "all">("all");
  const [groupByRule, setGroupByRule] = useState(false);

  // All hooks must run before any early-return so React's hook order stays
  // stable across the (no-result / analyzing / has-result) branches below.
  // Both memos no-op gracefully when `result` is null.
  const filteredIssues = useMemo(() => {
    if (!result) return [];
    return filter === "all"
      ? result.issues
      : result.issues.filter((i) => i.severity === filter);
  }, [result, filter]);

  // Group by rule_id when toggle is on. Groups sort by rule_id ascending so
  // the order is stable across renders. Within a group, issues keep their
  // original order (which already matches severity → rule order).
  const groups = useMemo(() => {
    if (!groupByRule) return null;
    const map = new Map<string, Issue[]>();
    for (const issue of filteredIssues) {
      const list = map.get(issue.rule_id) ?? [];
      list.push(issue);
      map.set(issue.rule_id, list);
    }
    return Array.from(map.entries()).sort((a, b) => a[0].localeCompare(b[0]));
  }, [groupByRule, filteredIssues]);

  if (!result && !isAnalyzing) {
    return (
      <div className="tc-issues">
        <div className="tc-issues-empty">
          <FileWarning size={42} style={{ opacity: 0.3 }} />
          <p style={{ fontSize: 12.5 }}>{t("issues.noResults")}</p>
          {onAnalyze && (
            <button onClick={onAnalyze} className="tc-cta">
              {t("sidebar.runAnalysis")}
            </button>
          )}
        </div>
      </div>
    );
  }

  if (isAnalyzing) {
    return (
      <div className="tc-issues">
        <div className="tc-issues-empty">
          <p style={{ fontSize: 12.5 }}>{t("issues.analyzing")}</p>
        </div>
      </div>
    );
  }

  if (!result) return null;

  const handleExport = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("export_issues_to_json", {
        projectId: activeProjectId,
      });
      const blob = new Blob([json], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = "issues.json";
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export issues:", err);
    }
  };

  const filterPill = (
    key: typeof filter,
    tone: "all" | "err" | "warn" | "info",
    label: string,
    count: number,
    icon?: React.ReactNode
  ) => (
    <button
      onClick={() => setFilter(key)}
      className="tc-issues-pill"
      data-tone={tone}
      data-active={filter === key ? "true" : undefined}
    >
      {icon}
      {label}
      <span className="mono">{count}</span>
    </button>
  );

  return (
    <div className="tc-issues">
      <div className="tc-issues-toolbar">
        {filterPill("all", "all", t("issues.all"), result.issue_count)}
        {filterPill(
          "error",
          "err",
          t("issues.errors"),
          result.error_count,
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "var(--err)",
            }}
          />
        )}
        {filterPill(
          "warning",
          "warn",
          t("issues.warnings"),
          result.warning_count,
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "var(--warn)",
            }}
          />
        )}
        {filterPill(
          "info",
          "info",
          t("issues.info"),
          result.info_count,
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "var(--info)",
            }}
          />
        )}

        <span style={{ flex: 1 }} />

        <button
          onClick={() => setGroupByRule((v) => !v)}
          className="tc-issues-pill"
          data-active={groupByRule ? "true" : undefined}
        >
          <Layers size={11} />
          {t("issues.groupByRule")}
        </button>
        <button onClick={handleExport} className="tc-issues-pill">
          <Download size={11} />
          {t("issues.export")}
        </button>

        {onAnalyze && (
          <button
            onClick={onAnalyze}
            className="tc-issues-pill"
            data-tone="all"
            style={{ color: "var(--primary)" }}
          >
            {t("issues.reanalyze")}
          </button>
        )}
      </div>

      <div className="tc-issues-list">
        {filteredIssues.length === 0 ? (
          <div className="tc-issues-empty">
            <p style={{ fontSize: 12.5 }}>
              {filter === "all"
                ? t("issues.noIssues")
                : t("issues.noFilteredIssues", { filter: t(`issues.${filter}s`) })}
            </p>
          </div>
        ) : groupByRule && groups ? (
          groups.map(([ruleId, issues]) => (
            <div key={ruleId}>
              <div className="tc-issues-group-head">
                <span>{issues[0].rule_name}</span>
                <span className="mono">{issues.length}</span>
              </div>
              {issues.map((issue, index) => (
                <IssueRow
                  key={`${issue.asset_path}-${index}`}
                  issue={issue}
                  onLocate={onLocate}
                  suggestionLabel={t("issues.suggestion")}
                  locateLabel={t("issues.locate")}
                />
              ))}
            </div>
          ))
        ) : (
          filteredIssues.map((issue, index) => (
            <IssueRow
              key={`${issue.asset_path}-${index}`}
              issue={issue}
              onLocate={onLocate}
              suggestionLabel={t("issues.suggestion")}
              locateLabel={t(fixLabelKey(issue.rule_id))}
            />
          ))
        )}
      </div>
    </div>
  );
}
