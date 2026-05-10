import { useEffect, useRef, useState } from "react";
import {
  Copy,
  CopyPlus,
  FolderInput,
  Tag as TagIcon,
  FolderOpen,
  Trash2,
  Edit3,
  Check,
  ChevronRight,
  ExternalLink,
  Settings as SettingsIcon,
  Sparkles,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useUiStore } from "../stores/uiStore";
import { cn } from "../lib/utils";
import { getExtension, getEditorDisplayName } from "../lib/pathUtils";
import type { Tag } from "../types/asset";

interface Position {
  x: number;
  y: number;
}

interface ContextMenuProps {
  isOpen: boolean;
  position: Position;
  onClose: () => void;
  assetPath: string;
  assetTags: Tag[];
  onCopyPath: () => void;
  onRevealInFileManager: () => void;
  onOpenWithDefaultApp: () => void;
  onRename: () => void;
  onDuplicate?: () => void;
  onMoveTo?: () => void;
  onCopyTo?: () => void;
  onDelete?: () => void;
  onOpenTagManager: () => void;
  /** Open the AIAnalyzeModal for this asset (or the multi-selection
   *  containing it). Omitted when AI tagging is disabled in Settings. */
  onAITag?: () => void;
}

export function ContextMenu({
  isOpen,
  position,
  onClose,
  assetPath,
  assetTags,
  onCopyPath,
  onRevealInFileManager,
  onOpenWithDefaultApp,
  onRename,
  onDuplicate,
  onMoveTo,
  onCopyTo,
  onDelete,
  onOpenTagManager,
  onAITag,
}: ContextMenuProps) {
  const { t } = useTranslation();
  const { tags, addTagToAsset, removeTagFromAsset } = useTagsStore();
  const externalEditors = useSettingsStore((s) => s.externalEditors);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const menuRef = useRef<HTMLDivElement>(null);

  // Per-asset editor decision: if the user mapped this extension, prefer
  // that editor; otherwise show the "Configure editor for .ext…" muted
  // entry so the user knows where to set it up. Files without extensions
  // (no dot in basename) get neither — the default-app row is enough.
  const ext = getExtension(assetPath);
  const editorPath = ext ? externalEditors[ext] : undefined;
  const editorName = editorPath ? getEditorDisplayName(editorPath) : undefined;
  const [showTagSubmenu, setShowTagSubmenu] = useState(false);
  const [submenuPosition, setSubmenuPosition] = useState<"right" | "left">("right");

  // Debounced close so quick diagonal cursor movements between the parent
  // button and the submenu don't prematurely unmount it. Leaving fires a
  // 150ms timer that re-entering cancels.
  const closeTimerRef = useRef<number | null>(null);
  const openSubmenu = () => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setShowTagSubmenu(true);
  };
  const scheduleCloseSubmenu = () => {
    if (closeTimerRef.current !== null) return;
    closeTimerRef.current = window.setTimeout(() => {
      setShowTagSubmenu(false);
      closeTimerRef.current = null;
    }, 150);
  };
  useEffect(() => {
    return () => {
      if (closeTimerRef.current !== null) {
        window.clearTimeout(closeTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!isOpen) return;

    const handleClickOutside = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, onClose]);

  // Adjust menu position to stay within viewport
  useEffect(() => {
    if (!isOpen || !menuRef.current) return;

    const menu = menuRef.current;
    const rect = menu.getBoundingClientRect();
    const viewportWidth = window.innerWidth;

    // Check if submenu would overflow right side
    if (position.x + rect.width + 200 > viewportWidth) {
      setSubmenuPosition("left");
    } else {
      setSubmenuPosition("right");
    }
  }, [isOpen, position]);

  if (!isOpen) return null;

  const handleTagToggle = async (tagId: string) => {
    const hasTag = assetTags.some((t) => t.id === tagId);
    if (hasTag) {
      await removeTagFromAsset(assetPath, tagId);
    } else {
      await addTagToAsset(assetPath, tagId);
    }
  };

  const menuItems = [
    // Headline action: open in the user's chosen editor for this
    // extension, falling back to a "Configure…" entry that jumps to
    // Settings so first-time users know how to set it up.
    ...(editorPath && editorName
      ? [
          {
            icon: <ExternalLink size={14} />,
            label: t("contextMenu.openInEditor", { name: editorName }),
            onClick: async () => {
              try {
                await invoke("open_in_editor", {
                  path: assetPath,
                  editor: editorPath,
                });
              } catch (err) {
                console.error("Failed to open in editor:", err);
              }
              onClose();
            },
          },
        ]
      : ext
      ? [
          {
            icon: <SettingsIcon size={14} />,
            label: t("contextMenu.configureEditor", { ext }),
            onClick: () => {
              setSettingsOpen(true);
              onClose();
            },
            muted: true,
          },
        ]
      : []),
    {
      icon: <FolderOpen size={14} />,
      label: t("contextMenu.revealInFileManager"),
      onClick: () => {
        onRevealInFileManager();
        onClose();
      },
    },
    {
      icon: <ExternalLink size={14} />,
      label: t("contextMenu.openWithDefaultApp"),
      onClick: () => {
        onOpenWithDefaultApp();
        onClose();
      },
    },
    {
      icon: <Copy size={14} />,
      label: t("contextMenu.copyPath"),
      onClick: () => {
        onCopyPath();
        onClose();
      },
    },
    ...(onAITag
      ? [
          { type: "separator" as const },
          {
            icon: <Sparkles size={14} />,
            label: t("aiAnalyze.entryLabel"),
            onClick: () => {
              onAITag();
              onClose();
            },
          },
        ]
      : []),
    { type: "separator" as const },
    {
      icon: <Edit3 size={14} />,
      label: t("contextMenu.rename"),
      onClick: () => {
        onRename();
        onClose();
      },
    },
    ...(onDuplicate
      ? [
          {
            icon: <CopyPlus size={14} />,
            label: t("contextMenu.duplicate"),
            onClick: () => {
              onDuplicate();
              onClose();
            },
          },
        ]
      : []),
    ...(onMoveTo
      ? [
          {
            icon: <FolderInput size={14} />,
            label: t("contextMenu.moveTo"),
            onClick: () => {
              onMoveTo();
              onClose();
            },
          },
        ]
      : []),
    ...(onCopyTo
      ? [
          {
            icon: <CopyPlus size={14} />,
            label: t("contextMenu.copyTo"),
            onClick: () => {
              onCopyTo();
              onClose();
            },
          },
        ]
      : []),
    ...(onDelete
      ? [
          { type: "separator" as const },
          {
            icon: <Trash2 size={14} />,
            label: t("contextMenu.delete"),
            onClick: () => {
              onDelete();
              onClose();
            },
            danger: true,
          },
        ]
      : []),
  ];

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-card-bg border border-border rounded-lg shadow-xl py-1 min-w-[180px]"
      style={{
        left: Math.min(position.x, window.innerWidth - 200),
        top: Math.min(position.y, window.innerHeight - 300),
      }}
    >
      {/* Tag Submenu Item */}
      <div
        className="relative"
        onMouseEnter={openSubmenu}
        onMouseLeave={scheduleCloseSubmenu}
      >
        <button className="flex items-center justify-between gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-background transition-colors">
          <div className="flex items-center gap-2">
            <TagIcon size={14} />
            <span>{t("contextMenu.tags")}</span>
          </div>
          <ChevronRight size={12} className="text-text-secondary" />
        </button>

        {/* Tag Submenu — zero gap to avoid a "no-element" transit zone between
            parent and submenu. The close timer in the wrapper handlers covers
            diagonal cursor paths that briefly dip outside both rects. */}
        {showTagSubmenu && (
          <div
            onMouseEnter={openSubmenu}
            onMouseLeave={scheduleCloseSubmenu}
            className={cn(
              "absolute top-0 bg-card-bg border border-border rounded-lg shadow-xl py-1 min-w-[160px]",
              submenuPosition === "right" ? "left-full" : "right-full"
            )}
          >
            {tags.length === 0 ? (
              <div className="px-3 py-2 text-sm text-text-secondary">{t("tags.noTags")}</div>
            ) : (
              tags.map((tag) => {
                const isSelected = assetTags.some((t) => t.id === tag.id);
                return (
                  <button
                    key={tag.id}
                    onClick={() => handleTagToggle(tag.id)}
                    className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-background transition-colors"
                  >
                    <span
                      className="w-3 h-3 rounded-full shrink-0"
                      style={{ backgroundColor: tag.color }}
                    />
                    <span className="flex-1 truncate">{tag.name}</span>
                    {isSelected && <Check size={14} className="text-primary shrink-0" />}
                  </button>
                );
              })
            )}
            <div className="border-t border-border mt-1 pt-1">
              <button
                onClick={() => {
                  onOpenTagManager();
                  onClose();
                }}
                className="flex items-center gap-2 w-full px-3 py-1.5 text-sm text-text-secondary hover:text-text-primary hover:bg-background transition-colors"
              >
                {t("tags.manageTitle")}
              </button>
            </div>
          </div>
        )}
      </div>

      <div className="border-t border-border my-1" />

      {/* Other Menu Items */}
      {menuItems.map((item, index) => {
        if (item.type === "separator") {
          return <div key={index} className="border-t border-border my-1" />;
        }
        return (
          <button
            key={index}
            onClick={item.onClick}
            className={cn(
              "flex items-center gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-background transition-colors",
              (item as { danger?: boolean }).danger && "text-red-400 hover:text-red-300",
              (item as { muted?: boolean }).muted && "italic text-text-secondary"
            )}
          >
            {item.icon}
            <span>{item.label}</span>
          </button>
        );
      })}
    </div>
  );
}
