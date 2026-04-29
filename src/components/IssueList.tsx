import { useState } from "react";
import { AlertCircle, AlertTriangle, Info, FileWarning } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Issue, Severity, AnalysisResult } from "../types/asset";

const SEV_TO_TONE: Record<Severity, "err" | "warn" | "info"> = {
  error: "err",
  warning: "warn",
  info: "info",
};

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
  const [filter, setFilter] = useState<Severity | "all">("all");

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

  const filteredIssues =
    filter === "all" ? result.issues : result.issues.filter((i) => i.severity === filter);

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
        ) : (
          filteredIssues.map((issue, index) => (
            <IssueRow
              key={`${issue.asset_path}-${index}`}
              issue={issue}
              onLocate={onLocate}
              suggestionLabel={t("issues.suggestion")}
              locateLabel={t("issues.locate")}
            />
          ))
        )}
      </div>
    </div>
  );
}
