import {
  Fragment,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AlertTriangle,
  BarChart3,
  Command,
  Download,
  Files,
  Folder,
  Globe,
  Moon,
  Play,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Square,
  Sun,
  Tag,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";
import { useThemeStore } from "../stores/themeStore";
import { useUiStore } from "../stores/uiStore";
import { formatShortcut, SHORTCUTS } from "../hooks/useKeyboardShortcuts";
import { basename } from "../lib/pathUtils";
import type { AssetType } from "../types/asset";

/// Maximum number of asset quick-jump matches surfaced in one query.
/// Cap is intentional — typical projects have 1k–50k assets and rendering
/// thousands of <button>s would tank input responsiveness. Future expansion
/// (fuzzy ranking, Web Worker, virtualization) can replace `.slice` here.
const ASSET_RESULT_CAP = 50;

/// Canonical asset-type order for the Filter section. Keeps the menu
/// stable across scans regardless of which types the project happens to
/// contain. Types absent from `scanResult.type_counts` are skipped.
const FILTER_TYPE_ORDER: AssetType[] = [
  "texture",
  "model",
  "audio",
  "video",
  "animation",
  "material",
  "prefab",
  "scene",
  "script",
  "data",
  "other",
];

interface CmdItem {
  id: string;
  section: string;
  label: string;
  sub?: string;
  shortcut?: string;
  icon: React.ReactNode;
  onSelect: () => void;
}

interface CommandPaletteProps {
  /** Export dispatcher — App.tsx already owns the URL.createObjectURL +
   *  download flow; we just route the chosen format to it. */
  onExport: (format: "json" | "csv" | "html") => void;
}

export function CommandPalette({ onExport }: CommandPaletteProps) {
  const { t, i18n } = useTranslation();

  const cmdkOpen = useUiStore((s) => s.cmdkOpen);
  const setCmdkOpen = useUiStore((s) => s.setCmdkOpen);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const setTagManagerOpen = useUiStore((s) => s.setTagManagerOpen);
  const setAiPanelOpen = useUiStore((s) => s.setAiPanelOpen);
  const setLearnSetupOpen = useUiStore((s) => s.setLearnSetupOpen);

  const projectPath = useProjectStore((s) => s.projectPath);
  const scanResult = useProjectStore((s) => s.scanResult);
  const isScanning = useProjectStore((s) => s.isScanning);
  const projects = useProjectStore((s) => s.projects);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const runAnalysis = useProjectStore((s) => s.runAnalysis);
  const cancelScan = useProjectStore((s) => s.cancelScan);
  const rescan = useProjectStore((s) => s.rescan);
  const setViewMode = useProjectStore((s) => s.setViewMode);
  const setActiveProject = useProjectStore((s) => s.setActiveProject);
  const locateAsset = useProjectStore((s) => s.locateAsset);
  const closeProject = useProjectStore((s) => s.closeProject);
  const typeFilter = useProjectStore((s) => s.typeFilter);
  const setTypeFilter = useProjectStore((s) => s.setTypeFilter);
  const toggleTypeFilter = useProjectStore((s) => s.toggleTypeFilter);

  const { theme, toggleTheme } = useThemeStore();

  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [activeIndex, setActiveIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  // Reset query + cursor each time the palette opens.
  useEffect(() => {
    if (cmdkOpen) {
      setQuery("");
      setActiveIndex(0);
    }
  }, [cmdkOpen]);

  // Reset cursor when the filter result-set shape changes.
  useEffect(() => {
    setActiveIndex(0);
  }, [deferredQuery]);

  const close = () => setCmdkOpen(false);

  const items = useMemo<CmdItem[]>(() => {
    if (!cmdkOpen) return [];

    const list: CmdItem[] = [];
    const hasProject = !!projectPath;
    const hasScan = !!scanResult;

    // SECTION: Suggestions
    if (hasProject && !isScanning) {
      list.push({
        id: "run-analysis",
        section: t("commandPalette.section.suggestions"),
        label: t("commandPalette.items.runAnalysis"),
        sub: t("commandPalette.items.runAnalysisSub"),
        shortcut: formatShortcut(SHORTCUTS.analyze),
        icon: <Play size={13} />,
        onSelect: () => {
          runAnalysis();
          close();
        },
      });
    }
    if (isScanning) {
      list.push({
        id: "cancel-scan",
        section: t("commandPalette.section.suggestions"),
        label: t("commandPalette.items.cancelScan"),
        shortcut: "Esc",
        icon: <Square size={13} />,
        onSelect: () => {
          cancelScan();
          close();
        },
      });
    }
    if (hasProject && projectPath && !isScanning) {
      list.push({
        id: "rescan",
        section: t("commandPalette.section.suggestions"),
        label: t("commandPalette.items.rescan"),
        shortcut: formatShortcut(SHORTCUTS.rescan),
        icon: <RefreshCw size={13} />,
        onSelect: () => {
          // Same contract as the ⌘R it advertises: rescan() clears the
          // disk scan cache first. The bare force-open this used to call
          // kept serving cached entries — a different, weaker operation
          // under the same label.
          void rescan();
          close();
        },
      });
    }

    // AI Learning entries — only meaningful when there's a project
    // open. Sit alongside Run Analysis since both are project-level
    // long-running ops.
    if (hasProject) {
      list.push({
        id: "ai-learn",
        section: t("commandPalette.section.suggestions"),
        label: t("aiTagPanel.runLearning"),
        icon: <Sparkles size={13} />,
        onSelect: () => {
          setLearnSetupOpen(true);
          close();
        },
      });
    }

    // SECTION: Navigate — view modes (require an open project)
    if (hasProject) {
      list.push({
        id: "go-assets",
        section: t("commandPalette.section.navigate"),
        label: t("commandPalette.items.goAssets"),
        shortcut: formatShortcut(SHORTCUTS.viewAssets),
        icon: <Files size={13} />,
        onSelect: () => {
          setViewMode("assets");
          close();
        },
      });
      list.push({
        id: "go-issues",
        section: t("commandPalette.section.navigate"),
        label: t("commandPalette.items.goIssues"),
        shortcut: formatShortcut(SHORTCUTS.viewIssues),
        icon: <AlertTriangle size={13} />,
        onSelect: () => {
          setViewMode("issues");
          close();
        },
      });
      list.push({
        id: "go-stats",
        section: t("commandPalette.section.navigate"),
        label: t("commandPalette.items.goStats"),
        shortcut: formatShortcut(SHORTCUTS.viewStats),
        icon: <BarChart3 size={13} />,
        onSelect: () => {
          setViewMode("stats");
          close();
        },
      });
    }

    // SECTION: Navigate — switch to other open projects (only when ≥2 open)
    if (projects.size >= 2) {
      for (const p of projects.values()) {
        if (p.id === activeProjectId) continue;
        const name = basename(p.projectPath) || "Project";
        list.push({
          id: `switch-${p.id}`,
          section: t("commandPalette.section.navigate"),
          label: t("commandPalette.items.switchTo", { name }),
          sub: p.projectPath,
          icon: <Folder size={13} />,
          onSelect: () => {
            setActiveProject(p.id);
            close();
          },
        });
      }
    }

    // SECTION: Filter — quick-toggle by asset type. Only renders types
    // actually present in the current scan, sorted by canonical order.
    if (hasScan && scanResult) {
      const counts = scanResult.type_counts;
      for (const type of FILTER_TYPE_ORDER) {
        const count = counts[type];
        if (!count) continue;
        const isActive = typeFilter?.includes(type) ?? false;
        list.push({
          id: `filter-${type}`,
          section: t("commandPalette.section.filter"),
          label: t("commandPalette.items.filterBy", {
            type: t(`assetTypes.${type}`),
          }),
          sub: t("commandPalette.items.filterCount", { count }),
          icon: (
            <span
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: `var(--c-${type})`,
                outline: isActive
                  ? "2px solid var(--primary)"
                  : undefined,
                outlineOffset: 1,
              }}
            />
          ),
          onSelect: () => {
            // Membership toggle: repeated invocations compose a multi-type
            // union (the pills' Ctrl+click, without the modifier).
            toggleTypeFilter(type);
            setViewMode("assets");
            close();
          },
        });
      }
      if (typeFilter !== null) {
        list.push({
          id: "filter-clear",
          section: t("commandPalette.section.filter"),
          label: t("commandPalette.items.clearFilter"),
          icon: <X size={13} />,
          onSelect: () => {
            setTypeFilter(null);
            close();
          },
        });
      }
    }

    // SECTION: Resources — asset quick-jump (only when query is non-empty)
    if (hasScan && deferredQuery.trim()) {
      const q = deferredQuery.toLowerCase();
      const matches: typeof scanResult.assets = [];
      for (const a of scanResult.assets) {
        if (
          a.name.toLowerCase().includes(q) ||
          a.path.toLowerCase().includes(q)
        ) {
          matches.push(a);
          if (matches.length >= ASSET_RESULT_CAP) break;
        }
      }
      for (const asset of matches) {
        list.push({
          id: `asset-${asset.path}`,
          section: t("commandPalette.section.resources"),
          label: asset.name,
          sub: asset.path,
          icon: (
            <span
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: `var(--c-${asset.asset_type})`,
              }}
            />
          ),
          onSelect: () => {
            locateAsset(asset.path);
            close();
          },
        });
      }
    }

    // SECTION: Actions
    if (hasScan) {
      list.push({
        id: "suggest-tags",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.suggestTags"),
        icon: <Sparkles size={13} />,
        onSelect: () => {
          setAiPanelOpen(true);
          close();
        },
      });
    }
    if (hasProject) {
      list.push({
        id: "manage-tags",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.manageTags"),
        icon: <Tag size={13} />,
        onSelect: () => {
          setTagManagerOpen(true);
          close();
        },
      });
    }
    list.push({
      id: "toggle-theme",
      section: t("commandPalette.section.actions"),
      label:
        theme === "dark"
          ? t("commandPalette.items.switchToLight")
          : t("commandPalette.items.switchToDark"),
      icon: theme === "dark" ? <Sun size={13} /> : <Moon size={13} />,
      onSelect: () => {
        toggleTheme();
        close();
      },
    });
    list.push({
      id: "toggle-lang",
      section: t("commandPalette.section.actions"),
      label:
        i18n.language === "en"
          ? t("commandPalette.items.toggleLanguageZh")
          : t("commandPalette.items.toggleLanguageEn"),
      icon: <Globe size={13} />,
      onSelect: () => {
        const next = i18n.language === "en" ? "zh" : "en";
        i18n.changeLanguage(next);
        localStorage.setItem("language", next);
        close();
      },
    });
    if (hasScan) {
      list.push({
        id: "export-json",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.exportJson"),
        icon: <Download size={13} />,
        onSelect: () => {
          onExport("json");
          close();
        },
      });
      list.push({
        id: "export-csv",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.exportCsv"),
        icon: <Download size={13} />,
        onSelect: () => {
          onExport("csv");
          close();
        },
      });
      list.push({
        id: "export-html",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.exportHtml"),
        icon: <Download size={13} />,
        onSelect: () => {
          onExport("html");
          close();
        },
      });
    }
    if (hasProject && activeProjectId) {
      list.push({
        id: "close-project",
        section: t("commandPalette.section.actions"),
        label: t("commandPalette.items.closeProject"),
        icon: <X size={13} />,
        onSelect: () => {
          closeProject();
          close();
        },
      });
    }
    list.push({
      id: "settings",
      section: t("commandPalette.section.actions"),
      label: t("commandPalette.items.settings"),
      shortcut: formatShortcut(SHORTCUTS.settings),
      icon: <Settings size={13} />,
      onSelect: () => {
        setSettingsOpen(true);
        close();
      },
    });

    return list;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    cmdkOpen,
    projectPath,
    scanResult,
    isScanning,
    projects,
    activeProjectId,
    deferredQuery,
    theme,
    i18n.language,
    typeFilter,
  ]);

  // Filter static items by query. Resources items are pre-filtered against
  // the same query at source so we let them through unchanged.
  const filteredItems = useMemo(() => {
    if (!deferredQuery.trim()) return items;
    const q = deferredQuery.toLowerCase();
    return items.filter((it) => {
      if (it.section === t("commandPalette.section.resources")) return true;
      return (
        it.label.toLowerCase().includes(q) ||
        (it.sub ? it.sub.toLowerCase().includes(q) : false)
      );
    });
  }, [items, deferredQuery, t]);

  // Keyboard navigation while open. Mounted only when open so it doesn't
  // shadow the rest of the app's shortcuts.
  useEffect(() => {
    if (!cmdkOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, filteredItems.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Home") {
        e.preventDefault();
        setActiveIndex(0);
      } else if (e.key === "End") {
        e.preventDefault();
        setActiveIndex(Math.max(0, filteredItems.length - 1));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = filteredItems[activeIndex];
        if (item) item.onSelect();
      } else if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cmdkOpen, filteredItems, activeIndex]);

  // Keep the cursor visible as it moves. We query by index against the live
  // DOM rather than tracking refs per item — DOM order matches filteredItems
  // order, and this avoids the stale-callback-ref problem when keys are
  // stable across renders.
  useEffect(() => {
    if (!cmdkOpen) return;
    const items = listRef.current?.querySelectorAll<HTMLButtonElement>(
      ".tc-cmdk-item"
    );
    items?.[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, cmdkOpen, filteredItems]);

  if (!cmdkOpen) return null;

  let lastSection: string | null = null;

  return (
    <div className="tc-overlay" onClick={close}>
      <div className="tc-cmdk" onClick={(e) => e.stopPropagation()}>
        <div className="tc-cmdk-input">
          <Search size={16} />
          <input
            autoFocus
            placeholder={t("commandPalette.placeholder")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <span className="tc-kbd">esc</span>
        </div>
        <div className="tc-cmdk-list" ref={listRef}>
          {filteredItems.length === 0 ? (
            <div className="tc-cmdk-empty">{t("commandPalette.empty")}</div>
          ) : (
            filteredItems.map((it, i) => {
              const showSection = it.section !== lastSection;
              lastSection = it.section;
              return (
                <Fragment key={it.id}>
                  {showSection && (
                    <div className="tc-cmdk-section">{it.section}</div>
                  )}
                  <button
                    type="button"
                    className="tc-cmdk-item"
                    data-active={i === activeIndex ? "true" : undefined}
                    onClick={() => it.onSelect()}
                    onMouseEnter={() => setActiveIndex(i)}
                  >
                    <span className="tc-cmdk-item-icon">{it.icon}</span>
                    <span className="tc-cmdk-item-label">{it.label}</span>
                    {it.sub && (
                      <span className="tc-cmdk-item-sub">{it.sub}</span>
                    )}
                    {it.shortcut && (
                      <span className="tc-kbd mono">{it.shortcut}</span>
                    )}
                  </button>
                </Fragment>
              );
            })
          )}
        </div>
        <div className="tc-cmdk-foot">
          <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
            <Command size={11} /> Tidycraft
          </span>
          <span className="tc-cmdk-foot-keys">
            <span>
              <span className="tc-kbd">↑</span>
              <span className="tc-kbd">↓</span>{" "}
              {t("commandPalette.footer.navigate")}
            </span>
            <span>
              <span className="tc-kbd">↵</span>{" "}
              {t("commandPalette.footer.select")}
            </span>
          </span>
        </div>
      </div>
    </div>
  );
}
