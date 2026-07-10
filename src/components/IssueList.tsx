import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, AlertTriangle, ChevronRight, Info, FileWarning, Layers, Download } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { basename, relativeToRoot } from "../lib/pathUtils";
import { exportTextFile } from "../lib/exportFile";
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
  /** Root-relative form of `issue.asset_path` for display; the absolute
   *  path stays available in the tooltip. */
  displayPath: string;
  expanded: boolean;
  onToggle: () => void;
  onLocate?: (path: string) => void;
  suggestionLabel: string;
  locateLabel: string;
}

/// `expanded` lives on the parent so virtualization (which unmounts rows
/// outside the overscan window) doesn't lose user state on scroll.
function IssueRow({ issue, displayPath, expanded, onToggle, onLocate, suggestionLabel, locateLabel }: IssueRowProps) {
  const fileName = basename(issue.asset_path);
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
          <span className="tc-issue-chev" data-expanded={expanded ? "true" : undefined}>
            <ChevronRight size={11} />
          </span>
        </div>
        <div className="tc-issue-meta">
          <strong>{fileName}</strong>
          <span style={{ color: "var(--text-4)" }}>·</span>
          <span title={issue.asset_path}>{displayPath}</span>
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
  /// Files changed after `result` was computed (watcher event / rescan) —
  /// the store's `analysisStale`. Shows the "results may be outdated" banner.
  stale?: boolean;
  isAnalyzing?: boolean;
  onAnalyze?: () => void;
  onLocate?: (path: string) => void;
}

/// Flattened virtual-list row. Group-by-rule mode interleaves headers and
/// issues; flat mode is just issues. Either way the virtualizer renders
/// from a single 1-D array so positioning math stays simple.
type VirtualRow =
  | { kind: "group-head"; key: string; ruleName: string; count: number }
  | { kind: "issue"; key: string; issue: Issue }
  | { kind: "dup-group"; key: string; ruleName: string; paths: string[] };

/// Collapse per-file duplicate issues into one row per content group.
/// `related_paths` (root-relative, original first) is the group identity;
/// every member issue carries the same list, so the first occurrence emits
/// the group row and the rest are dropped. Issues without the field (other
/// rules, or results from an older backend) pass through untouched.
function collapseDuplicates(issues: Issue[]): VirtualRow[] {
  const emitted = new Set<string>();
  const rows: VirtualRow[] = [];
  issues.forEach((issue, i) => {
    const group = issue.rule_id === "duplicate" ? issue.related_paths : undefined;
    if (group && group.length > 1) {
      const key = `dup:${group[0]}`;
      if (emitted.has(key)) return;
      emitted.add(key);
      rows.push({ kind: "dup-group", key, ruleName: issue.rule_name, paths: group });
      return;
    }
    rows.push({ kind: "issue", key: `${issue.rule_id}|${issue.asset_path}|${i}`, issue });
  });
  return rows;
}

/// How many group members show before the "Show all N" toggle. Groups from
/// generated/atlas content can run to hundreds of files.
const DUP_GROUP_PREVIEW = 5;
/// Hard cap on rendered members even when expanded — real libraries produce
/// groups with thousands of identical files (Kenney: 3178), and that many
/// DOM rows inside one virtualized card stalls the renderer.
const DUP_GROUP_MAX_EXPANDED = 200;

interface DupGroupRowProps {
  row: Extract<VirtualRow, { kind: "dup-group" }>;
  expanded: boolean;
  onToggle: () => void;
  onLocate?: (path: string) => void;
  projectPath: string | null;
  locateLabel: string;
}

/// One card per duplicate-content group: every member listed (original
/// tagged), each with its own locate action. Replaces N-1 identical
/// stacked cards per group.
function DupGroupRow({ row, expanded, onToggle, onLocate, projectPath, locateLabel }: DupGroupRowProps) {
  const { t } = useTranslation();
  const shown = row.paths.slice(0, expanded ? DUP_GROUP_MAX_EXPANDED : DUP_GROUP_PREVIEW);
  const hiddenCount = row.paths.length - shown.length;
  // Members are root-relative; locate needs the absolute path back.
  const toAbsolute = (rel: string) => (projectPath ? `${projectPath}/${rel}` : rel);

  return (
    <div className="tc-issue-row" data-expanded="true" style={{ cursor: "default" }}>
      <span className="tc-issue-icon" data-sev="warn">
        <SeverityIcon severity="warning" />
      </span>
      <div className="tc-issue-body">
        <div className="tc-issue-title">
          {row.ruleName}
          <span className="tc-issue-rule-id">duplicate</span>
          <span className="tc-dup-count">{t("issues.dupGroupCount", { count: row.paths.length })}</span>
        </div>
        <ul className="tc-dup-members">
          {shown.map((path, i) => (
            <li key={path}>
              <span className="tc-dup-path" title={path}>
                {path}
              </span>
              {i === 0 && <span className="tc-dup-original">{t("issues.dupOriginal")}</span>}
              {onLocate && (
                <button
                  className="tc-dup-locate"
                  onClick={() => onLocate(toAbsolute(path))}
                >
                  {locateLabel}
                </button>
              )}
            </li>
          ))}
        </ul>
        {!expanded && hiddenCount > 0 && (
          <button className="tc-dup-more" onClick={onToggle}>
            {t("issues.showAllMembers", { count: row.paths.length })}
          </button>
        )}
        {expanded && hiddenCount > 0 && (
          <span className="tc-dup-truncated">
            {t("issues.membersTruncated", { count: hiddenCount })}
          </span>
        )}
        {expanded && row.paths.length > DUP_GROUP_PREVIEW && (
          <button className="tc-dup-more" onClick={onToggle}>
            {t("issues.showFewerMembers")}
          </button>
        )}
        <div className="tc-issue-detail" style={{ marginTop: 6 }}>
          {t("issues.dupGroupHint")}
        </div>
      </div>
    </div>
  );
}

export function IssueList({ result, stale, isAnalyzing, onAnalyze, onLocate }: IssueListProps) {
  const { t } = useTranslation();
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const projectPath = useProjectStore((s) => s.projectPath);
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
  /// Duplicate-content issues collapse into one dup-group row per content
  /// group in both modes (see collapseDuplicates).
  const rows = useMemo<VirtualRow[]>(() => {
    if (groupByRule && groups) {
      const list: VirtualRow[] = [];
      for (const [ruleId, issues] of groups) {
        const collapsed = collapseDuplicates(issues);
        list.push({
          kind: "group-head",
          key: `head:${ruleId}`,
          ruleName: issues[0].rule_name,
          count: collapsed.length,
        });
        list.push(...collapsed);
      }
      return list;
    }
    return collapseDuplicates(filteredIssues);
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
      if (row.kind === "group-head") return 36;
      if (row.kind === "dup-group")
        return 88 + Math.min(row.paths.length, DUP_GROUP_PREVIEW) * 22;
      return 56;
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

  const handleExport = () => {
    if (!activeProjectId) return;
    exportTextFile({
      defaultName: "issues.json",
      filterName: "JSON",
      extensions: ["json"],
      fetchContents: () =>
        invoke<string>("export_issues_to_json", { projectId: activeProjectId }),
    });
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
      {/* Point-in-time snapshot warning: files changed since this analysis
          ran, so counts/paths below may reference a world that's gone. */}
      {stale && (
        <div
          className="flex items-center gap-2 px-3 py-2 text-xs shrink-0"
          style={{
            background: "color-mix(in oklch, var(--warn) 12%, transparent)",
            borderBottom: "1px solid var(--line)",
            color: "var(--text-2)",
          }}
        >
          <AlertTriangle size={12} style={{ color: "var(--warn)" }} />
          <span className="flex-1">{t("issues.staleBanner")}</span>
          {onAnalyze && (
            <button
              onClick={onAnalyze}
              disabled={isAnalyzing}
              className="px-2 py-0.5 rounded disabled:opacity-50"
              style={{ border: "1px solid var(--line)", color: "var(--text)" }}
            >
              {t("issues.reanalyze")}
            </button>
          )}
        </div>
      )}
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
                  ) : row.kind === "dup-group" ? (
                    <DupGroupRow
                      row={row}
                      expanded={expandedIds.has(row.key)}
                      onToggle={() => toggleExpanded(row.key)}
                      onLocate={onLocate}
                      projectPath={projectPath}
                      locateLabel={t("issues.locate")}
                    />
                  ) : (
                    <IssueRow
                      issue={row.issue}
                      displayPath={relativeToRoot(row.issue.asset_path, projectPath)}
                      expanded={expandedIds.has(row.key)}
                      onToggle={() => toggleExpanded(row.key)}
                      onLocate={onLocate}
                      suggestionLabel={t("issues.suggestion")}
                      locateLabel={t("issues.locate")}
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
