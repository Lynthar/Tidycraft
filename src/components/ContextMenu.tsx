import { useEffect, useRef, useState } from "react";
import { Copy, Tag as TagIcon, FolderOpen, Trash2, Edit3, Check, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { cn } from "../lib/utils";
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
  onRevealInFinder: () => void;
  onRename: () => void;
  onDelete?: () => void;
  onOpenTagManager: () => void;
}

export function ContextMenu({
  isOpen,
  position,
  onClose,
  assetPath,
  assetTags,
  onCopyPath,
  onRevealInFinder,
  onRename,
  onDelete,
  onOpenTagManager,
}: ContextMenuProps) {
  const { t } = useTranslation();
  const { tags, addTagToAsset, removeTagFromAsset } = useTagsStore();
  const menuRef = useRef<HTMLDivElement>(null);
  const [showTagSubmenu, setShowTagSubmenu] = useState(false);
  const [submenuPosition, setSubmenuPosition] = useState<"right" | "left">("right");

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
    {
      icon: <Copy size={14} />,
      label: t("contextMenu.copyPath"),
      onClick: () => {
        onCopyPath();
        onClose();
      },
    },
    {
      icon: <FolderOpen size={14} />,
      label: t("contextMenu.revealInFinder"),
      onClick: () => {
        onRevealInFinder();
        onClose();
      },
    },
    { type: "separator" as const },
    {
      icon: <Edit3 size={14} />,
      label: t("contextMenu.rename"),
      onClick: () => {
        onRename();
        onClose();
      },
    },
    ...(onDelete
      ? [
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
        onMouseEnter={() => setShowTagSubmenu(true)}
        onMouseLeave={() => setShowTagSubmenu(false)}
      >
        <button className="flex items-center justify-between gap-2 w-full px-3 py-1.5 text-sm text-left hover:bg-background transition-colors">
          <div className="flex items-center gap-2">
            <TagIcon size={14} />
            <span>{t("contextMenu.tags")}</span>
          </div>
          <ChevronRight size={12} className="text-text-secondary" />
        </button>

        {/* Tag Submenu */}
        {showTagSubmenu && (
          <div
            className={cn(
              "absolute top-0 bg-card-bg border border-border rounded-lg shadow-xl py-1 min-w-[160px]",
              submenuPosition === "right" ? "left-full ml-1" : "right-full mr-1"
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
              (item as { danger?: boolean }).danger && "text-red-400 hover:text-red-300"
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
