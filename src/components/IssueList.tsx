import { useState } from "react";
import { AlertCircle, AlertTriangle, Info, ChevronDown, ChevronRight, FileWarning } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../lib/utils";
import type { Issue, Severity, AnalysisResult } from "../types/asset";

interface IssueRowProps {
  issue: Issue;
  onLocate?: (path: string) => void;
  suggestionLabel: string;
  locateLabel: string;
}

function SeverityIcon({ severity }: { severity: Severity }) {
  switch (severity) {
    case "error":
      return <AlertCircle size={16} className="text-error shrink-0" />;
    case "warning":
      return <AlertTriangle size={16} className="text-warning shrink-0" />;
    case "info":
      return <Info size={16} className="text-info shrink-0" />;
  }
}

function IssueRow({ issue, onLocate, suggestionLabel, locateLabel }: IssueRowProps) {
  const [expanded, setExpanded] = useState(false);

  const fileName = issue.asset_path.split("/").pop() || issue.asset_path;

  return (
    <div className="border-b border-border">
      <div
        className={cn(
          "flex items-start gap-3 p-3 cursor-pointer hover:bg-background transition-colors",
          expanded && "bg-background"
        )}
        onClick={() => setExpanded(!expanded)}
      >
        <button className="mt-0.5 shrink-0">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
        <SeverityIcon severity={issue.severity} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="font-medium text-sm">{issue.rule_name}</span>
            <span className="text-xs text-text-secondary px-1.5 py-0.5 bg-background rounded">
              {issue.rule_id}
            </span>
          </div>
          <p className="text-sm text-text-secondary mt-0.5 truncate">{fileName}</p>
        </div>
      </div>

      {expanded && (
        <div className="px-10 pb-3 space-y-2">
          <p className="text-sm">{issue.message}</p>
          {issue.suggestion && (
            <p className="text-sm text-text-secondary">
              <span className="text-primary">{suggestionLabel}:</span> {issue.suggestion}
            </p>
          )}
          <div className="flex items-center gap-2 text-xs">
            <span className="text-text-secondary truncate">{issue.asset_path}</span>
            {onLocate && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onLocate(issue.asset_path);
                }}
                className="px-2 py-1 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors shrink-0"
              >
                {locateLabel}
              </button>
            )}
          </div>
        </div>
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
      <div className="h-full flex flex-col items-center justify-center text-text-secondary gap-4">
        <FileWarning size={48} className="opacity-30" />
        <p>{t("issues.noResults")}</p>
        {onAnalyze && (
          <button
            onClick={onAnalyze}
            className="px-4 py-2 bg-primary text-white rounded hover:bg-primary/90 transition-colors"
          >
            {t("sidebar.runAnalysis")}
          </button>
        )}
      </div>
    );
  }

  if (isAnalyzing) {
    return (
      <div className="h-full flex items-center justify-center text-text-secondary">
        {t("issues.analyzing")}
      </div>
    );
  }

  if (!result) return null;

  const filteredIssues =
    filter === "all" ? result.issues : result.issues.filter((i) => i.severity === filter);

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="shrink-0 p-3 border-b border-border bg-card-bg">
        <div className="flex items-center justify-between mb-2">
          <h3 className="font-medium">{t("issues.title")} ({result.issue_count})</h3>
          {onAnalyze && (
            <button
              onClick={onAnalyze}
              className="text-xs px-2 py-1 bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
            >
              {t("issues.reanalyze")}
            </button>
          )}
        </div>

        {/* Filter tabs */}
        <div className="flex gap-2">
          <button
            onClick={() => setFilter("all")}
            className={cn(
              "px-2 py-1 text-xs rounded transition-colors",
              filter === "all"
                ? "bg-text-secondary/20 text-text-primary"
                : "text-text-secondary hover:bg-background"
            )}
          >
            {t("issues.all")} ({result.issue_count})
          </button>
          <button
            onClick={() => setFilter("error")}
            className={cn(
              "px-2 py-1 text-xs rounded transition-colors flex items-center gap-1",
              filter === "error"
                ? "bg-error/20 text-error"
                : "text-text-secondary hover:bg-background"
            )}
          >
            <AlertCircle size={12} /> {t("issues.errors")} ({result.error_count})
          </button>
          <button
            onClick={() => setFilter("warning")}
            className={cn(
              "px-2 py-1 text-xs rounded transition-colors flex items-center gap-1",
              filter === "warning"
                ? "bg-warning/20 text-warning"
                : "text-text-secondary hover:bg-background"
            )}
          >
            <AlertTriangle size={12} /> {t("issues.warnings")} ({result.warning_count})
          </button>
          <button
            onClick={() => setFilter("info")}
            className={cn(
              "px-2 py-1 text-xs rounded transition-colors flex items-center gap-1",
              filter === "info"
                ? "bg-info/20 text-info"
                : "text-text-secondary hover:bg-background"
            )}
          >
            <Info size={12} /> {t("issues.info")} ({result.info_count})
          </button>
        </div>
      </div>

      {/* Issue list */}
      <div className="flex-1 overflow-auto">
        {filteredIssues.length === 0 ? (
          <div className="flex items-center justify-center h-full text-text-secondary">
            {filter === "all" ? t("issues.noIssues") : t("issues.noFilteredIssues", { filter: t(`issues.${filter}s`) })}
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
