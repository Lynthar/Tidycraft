import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, AlertTriangle, Info, FileWarning, Layers, Download } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
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
  if (ruleId === "pbr_set.incomplete") return "issues.fix.add_textures";
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
  expanded: boolean;
  onToggle: () => void;
  onLocate?: (path: string) => void;
  suggestionLabel: string;
  locateLabel: string;
}

/// `expanded` lives on the parent so virtualization (which unmounts rows
/// outside the overscan window) doesn't lose user state on scroll.
function IssueRow({ issue, expanded, onToggle, onLocate, suggestionLabel, locateLabel }: IssueRowProps) {
  const fileName = issue.asset_path.split("/").pop() || issue.asset_path;
  const tone = SEV_TO_TONE[issue.severity];

  return (
    <div
      className="tc-issue-row"
      data-expanded={expanded ? "true" : undefined}
      onClick={onToggle}
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

/// Flattened virtual-list row. Group-by-rule mode interleaves headers and
/// issues; flat mode is just issues. Either way the virtualizer renders
/// from a single 1-D array so positioning math stays simple.
type VirtualRow =
  | { kind: "group-head"; key: string; ruleName: string; count: number }
  | { kind: "issue"; key: string; issue: Issue };

export function IssueList({ result, isAnalyzing, onAnalyze, onLocate }: IssueListProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const [filter, setFilter] = useState<Severity | "all">("all");
  const [groupByRule, setGroupByRule] = useState(false);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  // All hooks must run before any early-return so React's hook order stays
  // stable across the (no-result / analyzing / has-result) branches below.
  // The memos no-op gracefully when `result` is null.
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

  /// Issue keys carry rule_id + asset_path + group-local index. The trailing
  /// index disambiguates the rare case where the same rule fires twice on
  /// one asset; without it both rows would share an expanded slot.
  const rows = useMemo<VirtualRow[]>(() => {
    if (groupByRule && groups) {
      const list: VirtualRow[] = [];
      for (const [ruleId, issues] of groups) {
        list.push({
          kind: "group-head",
          key: `head:${ruleId}`,
          ruleName: issues[0].rule_name,
          count: issues.length,
        });
        issues.forEach((issue, i) => {
          list.push({
            kind: "issue",
            key: `${ruleId}|${issue.asset_path}|${i}`,
            issue,
          });
        });
      }
      return list;
    }
    return filteredIssues.map((issue, i) => ({
      kind: "issue" as const,
      key: `${issue.rule_id}|${issue.asset_path}|${i}`,
      issue,
    }));
  }, [groupByRule, groups, filteredIssues]);

  const parentRef = useRef<HTMLDivElement>(null);

  /// Stable callback identities — react-virtual's `useVirtualizer` re-runs
  /// internal effects when these references change, and inline arrow
  /// functions per render were producing a re-render storm under certain
  /// transitions (run-analysis-while-changing-views). estimateSize is a
  /// rough constant by row kind only; measureElement feeds the true height
  /// (including expanded-row growth) back to the virtualizer via
  /// ResizeObserver — no need to closure over expandedIds here.
  const getScrollElement = useCallback(() => parentRef.current, []);
  const estimateSize = useCallback(
    (index: number) => {
      const row = rows[index];
      return row.kind === "group-head" ? 36 : 56;
    },
    [rows]
  );
  const getItemKey = useCallback((index: number) => rows[index].key, [rows]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement,
    estimateSize,
    overscan: 8,
    getItemKey,
  });

  /// Reset expanded state and scroll position when the underlying issue set
  /// changes (filter / group toggle / re-analyze). The functional setter
  /// returns `prev` when already empty so we don't churn a fresh Set
  /// reference each time and force a no-op re-render.
  useEffect(() => {
    setExpandedIds((prev) => (prev.size === 0 ? prev : new Set()));
    if (parentRef.current) parentRef.current.scrollTop = 0;
  }, [filter, groupByRule, result]);

  const toggleExpanded = useCallback((id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

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

  const virtualItems = virtualizer.getVirtualItems();

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

      <div ref={parentRef} className="tc-issues-list">
        {filteredIssues.length === 0 ? (
          <div className="tc-issues-empty">
            <p style={{ fontSize: 12.5 }}>
              {filter === "all"
                ? t("issues.noIssues")
                : t("issues.noFilteredIssues", { filter: t(`issues.${filter}s`) })}
            </p>
          </div>
        ) : (
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {virtualItems.map((virtualItem) => {
              const row = rows[virtualItem.index];
              return (
                <div
                  key={virtualItem.key}
                  ref={virtualizer.measureElement}
                  data-index={virtualItem.index}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${virtualItem.start}px)`,
                  }}
                >
                  {row.kind === "group-head" ? (
                    <div className="tc-issues-group-head">
                      <span>{row.ruleName}</span>
                      <span className="mono">{row.count}</span>
                    </div>
                  ) : (
                    <IssueRow
                      issue={row.issue}
                      expanded={expandedIds.has(row.key)}
                      onToggle={() => toggleExpanded(row.key)}
                      onLocate={onLocate}
                      suggestionLabel={t("issues.suggestion")}
                      locateLabel={
                        groupByRule
                          ? t("issues.locate")
                          : t(fixLabelKey(row.issue.rule_id))
                      }
                    />
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
