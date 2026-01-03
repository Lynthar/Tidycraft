import { Tag as TagIcon, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { cn } from "../lib/utils";

export function TagFilterPanel() {
  const { t } = useTranslation();
  const { tags, tagFilter, setTagFilter } = useTagsStore();

  if (tags.length === 0) {
    return null;
  }

  return (
    <div className="border-b border-border shrink-0">
      <div className="h-8 px-3 flex items-center text-xs text-text-secondary font-medium uppercase tracking-wide">
        <TagIcon size={12} className="mr-1.5" />
        {t("tags.title")}
      </div>
      <div className="px-2 pb-2 flex flex-wrap gap-1">
        {tags.map((tag) => {
          const isActive = tagFilter === tag.id;
          return (
            <button
              key={tag.id}
              onClick={() => setTagFilter(isActive ? null : tag.id)}
              className={cn(
                "flex items-center gap-1 px-2 py-1 rounded text-xs transition-colors",
                isActive
                  ? "ring-1 ring-offset-1 ring-offset-card-bg"
                  : "hover:bg-background"
              )}
              style={{
                backgroundColor: isActive ? `${tag.color}30` : `${tag.color}15`,
                color: tag.color,
                ...(isActive ? { ringColor: tag.color } : {}),
              }}
            >
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: tag.color }}
              />
              <span>{tag.name}</span>
              {isActive && <X size={10} />}
            </button>
          );
        })}
      </div>
    </div>
  );
}
