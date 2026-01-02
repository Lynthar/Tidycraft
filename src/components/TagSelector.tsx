import { useState, useRef, useEffect } from "react";
import { Tag as TagIcon, Check, Plus, Settings } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { cn } from "../lib/utils";
import type { Tag } from "../types/asset";

interface TagSelectorProps {
  assetPath: string;
  assetTags: Tag[];
  onOpenManager: () => void;
}

export function TagSelector({ assetPath, assetTags, onOpenManager }: TagSelectorProps) {
  const { t } = useTranslation();
  const { tags, addTagToAsset, removeTagFromAsset } = useTagsStore();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handleToggleTag = async (tagId: string) => {
    const hasTag = assetTags.some((t) => t.id === tagId);
    if (hasTag) {
      await removeTagFromAsset(assetPath, tagId);
    } else {
      await addTagToAsset(assetPath, tagId);
    }
  };

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-1 px-2 py-1 text-xs text-text-secondary hover:text-text-primary hover:bg-background rounded transition-colors"
      >
        <TagIcon size={12} />
        <span>{t("tags.addTag")}</span>
      </button>

      {isOpen && (
        <div className="absolute top-full left-0 mt-1 w-48 bg-card-bg border border-border rounded-lg shadow-lg z-50 overflow-hidden">
          <div className="max-h-48 overflow-auto">
            {tags.length === 0 ? (
              <p className="p-3 text-sm text-text-secondary text-center">
                {t("tags.noTags")}
              </p>
            ) : (
              tags.map((tag) => {
                const isSelected = assetTags.some((t) => t.id === tag.id);
                return (
                  <button
                    key={tag.id}
                    onClick={() => handleToggleTag(tag.id)}
                    className={cn(
                      "flex items-center gap-2 w-full px-3 py-2 text-sm text-left hover:bg-background transition-colors",
                      isSelected && "bg-primary/10"
                    )}
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
          </div>
          <div className="border-t border-border">
            <button
              onClick={() => {
                setIsOpen(false);
                onOpenManager();
              }}
              className="flex items-center gap-2 w-full px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-background transition-colors"
            >
              <Settings size={14} />
              {t("tags.manageTitle")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// Tag badge for displaying tags inline
export function TagBadge({ tag, onRemove }: { tag: Tag; onRemove?: () => void }) {
  return (
    <span
      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium"
      style={{
        backgroundColor: `${tag.color}20`,
        color: tag.color,
      }}
    >
      {tag.name}
      {onRemove && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className="hover:bg-white/20 rounded"
        >
          ×
        </button>
      )}
    </span>
  );
}

// Batch tag selector for multiple assets
interface BatchTagSelectorProps {
  selectedPaths: string[];
  onOpenManager: () => void;
}

export function BatchTagSelector({ selectedPaths, onOpenManager }: BatchTagSelectorProps) {
  const { t } = useTranslation();
  const { tags, addTagToAssets } = useTagsStore();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handleAddTag = async (tagId: string) => {
    await addTagToAssets(selectedPaths, tagId);
    setIsOpen(false);
  };

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-1.5 px-3 py-1.5 text-sm bg-primary/20 text-primary rounded hover:bg-primary/30 transition-colors"
      >
        <TagIcon size={14} />
        {t("tags.addTag")}
      </button>

      {isOpen && (
        <div className="absolute top-full left-0 mt-1 w-48 bg-card-bg border border-border rounded-lg shadow-lg z-50 overflow-hidden">
          <div className="max-h-48 overflow-auto">
            {tags.length === 0 ? (
              <p className="p-3 text-sm text-text-secondary text-center">
                {t("tags.noTags")}
              </p>
            ) : (
              tags.map((tag) => (
                <button
                  key={tag.id}
                  onClick={() => handleAddTag(tag.id)}
                  className="flex items-center gap-2 w-full px-3 py-2 text-sm text-left hover:bg-background transition-colors"
                >
                  <span
                    className="w-3 h-3 rounded-full shrink-0"
                    style={{ backgroundColor: tag.color }}
                  />
                  <span className="flex-1 truncate">{tag.name}</span>
                  <Plus size={14} className="text-text-secondary" />
                </button>
              ))
            )}
          </div>
          <div className="border-t border-border">
            <button
              onClick={() => {
                setIsOpen(false);
                onOpenManager();
              }}
              className="flex items-center gap-2 w-full px-3 py-2 text-sm text-text-secondary hover:text-text-primary hover:bg-background transition-colors"
            >
              <Settings size={14} />
              {t("tags.manageTitle")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
