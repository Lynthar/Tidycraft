import { useState } from "react";
import { Tag as TagIcon, X, ChevronDown, ChevronRight, Plus, Pencil, Trash2, Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { cn } from "../lib/utils";

const TAG_COLORS = [
  "#ef4444", "#f97316", "#eab308", "#22c55e", "#10b981",
  "#14b8a6", "#06b6d4", "#3b82f6", "#6366f1", "#8b5cf6",
  "#a855f7", "#d946ef", "#ec4899", "#f43f5e",
];

export function TagFilterPanel() {
  const { t } = useTranslation();
  const { tags, tagFilter, setTagFilter, createTag, updateTag, deleteTag } = useTagsStore();
  const [isExpanded, setIsExpanded] = useState(true);
  const [isCreating, setIsCreating] = useState(false);
  const [editingTagId, setEditingTagId] = useState<string | null>(null);
  const [newTagName, setNewTagName] = useState("");
  const [newTagColor, setNewTagColor] = useState(TAG_COLORS[0]);
  const [showColorPicker, setShowColorPicker] = useState(false);

  const handleCreateTag = async () => {
    if (!newTagName.trim()) return;
    await createTag(newTagName.trim(), newTagColor);
    setNewTagName("");
    setNewTagColor(TAG_COLORS[Math.floor(Math.random() * TAG_COLORS.length)]);
    setIsCreating(false);
  };

  const handleUpdateTag = async (tagId: string, name: string) => {
    if (!name.trim()) return;
    await updateTag(tagId, name.trim());
    setEditingTagId(null);
  };

  const handleDeleteTag = async (tagId: string) => {
    await deleteTag(tagId);
    if (tagFilter === tagId) {
      setTagFilter(null);
    }
  };

  return (
    <div className="border-b border-border shrink-0">
      {/* Header - Collapsible */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full h-8 px-3 flex items-center justify-between text-xs text-text-secondary font-medium uppercase tracking-wide hover:bg-background transition-colors"
      >
        <div className="flex items-center gap-1.5">
          {isExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          <TagIcon size={12} />
          {t("tags.title")}
          {tags.length > 0 && (
            <span className="text-text-secondary/60">({tags.length})</span>
          )}
        </div>
        {isExpanded && (
          <div
            onClick={(e) => {
              e.stopPropagation();
              setIsCreating(true);
            }}
            className="p-1 rounded hover:bg-card-bg text-text-secondary hover:text-primary transition-colors"
            title={t("tags.createTag")}
          >
            <Plus size={12} />
          </div>
        )}
      </button>

      {isExpanded && (
        <div className="px-2 pb-2">
          {/* Create New Tag Form */}
          {isCreating && (
            <div className="mb-2 p-2 bg-background rounded border border-border">
              <div className="flex items-center gap-2 mb-2">
                <div className="relative">
                  <button
                    onClick={() => setShowColorPicker(!showColorPicker)}
                    className="w-6 h-6 rounded border border-border"
                    style={{ backgroundColor: newTagColor }}
                  />
                  {showColorPicker && (
                    <div
                      className="absolute left-0 top-full mt-1 p-2 bg-card-bg border border-border rounded-lg shadow-lg z-50"
                      style={{ width: '160px' }}
                    >
                      <div className="grid grid-cols-7 gap-1">
                        {TAG_COLORS.map((color) => (
                          <button
                            key={color}
                            onClick={(e) => {
                              e.stopPropagation();
                              setNewTagColor(color);
                              setShowColorPicker(false);
                            }}
                            className={cn(
                              "w-4 h-4 rounded shrink-0",
                              newTagColor === color && "ring-2 ring-offset-1 ring-primary"
                            )}
                            style={{ backgroundColor: color }}
                          />
                        ))}
                      </div>
                    </div>
                  )}
                </div>
                <input
                  type="text"
                  value={newTagName}
                  onChange={(e) => setNewTagName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreateTag();
                    if (e.key === "Escape") setIsCreating(false);
                  }}
                  placeholder={t("tags.tagName")}
                  className="flex-1 px-2 py-1 text-xs bg-card-bg border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                  autoFocus
                />
              </div>
              <div className="flex items-center gap-1 justify-end">
                <button
                  onClick={() => setIsCreating(false)}
                  className="px-2 py-1 text-xs text-text-secondary hover:text-text-primary transition-colors"
                >
                  {t("common.cancel")}
                </button>
                <button
                  onClick={handleCreateTag}
                  disabled={!newTagName.trim()}
                  className="px-2 py-1 text-xs bg-primary text-white rounded hover:bg-primary/90 transition-colors disabled:opacity-50"
                >
                  {t("tags.createTag")}
                </button>
              </div>
            </div>
          )}

          {/* Tags List */}
          {tags.length === 0 && !isCreating ? (
            <div className="text-xs text-text-secondary italic py-2 px-1">
              {t("tags.noTags")}
            </div>
          ) : (
            <div className="space-y-1 max-h-32 overflow-y-auto">
              {tags.map((tag) => {
                const isActive = tagFilter === tag.id;
                const isEditing = editingTagId === tag.id;

                if (isEditing) {
                  return (
                    <div key={tag.id} className="flex items-center gap-1 p-1 bg-background rounded">
                      <span
                        className="w-3 h-3 rounded-full shrink-0"
                        style={{ backgroundColor: tag.color }}
                      />
                      <input
                        type="text"
                        defaultValue={tag.name}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            handleUpdateTag(tag.id, (e.target as HTMLInputElement).value);
                          }
                          if (e.key === "Escape") {
                            setEditingTagId(null);
                          }
                        }}
                        className="flex-1 px-1 py-0.5 text-xs bg-card-bg border border-border rounded text-text-primary focus:outline-none focus:border-primary min-w-0"
                        autoFocus
                      />
                      <button
                        onClick={(e) => {
                          const input = (e.currentTarget.previousElementSibling as HTMLInputElement);
                          handleUpdateTag(tag.id, input.value);
                        }}
                        className="p-1 text-success hover:bg-card-bg rounded transition-colors"
                      >
                        <Check size={12} />
                      </button>
                      <button
                        onClick={() => setEditingTagId(null)}
                        className="p-1 text-text-secondary hover:text-text-primary hover:bg-card-bg rounded transition-colors"
                      >
                        <X size={12} />
                      </button>
                    </div>
                  );
                }

                return (
                  <div
                    key={tag.id}
                    className={cn(
                      "flex items-center gap-1 group rounded transition-colors",
                      isActive
                        ? "ring-1 ring-offset-1 ring-offset-card-bg"
                        : "hover:bg-background"
                    )}
                    style={{
                      backgroundColor: isActive ? `${tag.color}30` : `${tag.color}15`,
                      ...(isActive ? { ringColor: tag.color } : {}),
                    }}
                  >
                    <button
                      onClick={() => setTagFilter(isActive ? null : tag.id)}
                      className="flex-1 flex items-center gap-1.5 px-2 py-1 text-xs min-w-0"
                      style={{ color: tag.color }}
                    >
                      <span
                        className="w-2 h-2 rounded-full shrink-0"
                        style={{ backgroundColor: tag.color }}
                      />
                      <span className="truncate">{tag.name}</span>
                      {isActive && <X size={10} className="shrink-0" />}
                    </button>
                    <div className="flex items-center opacity-0 group-hover:opacity-100 transition-opacity pr-1">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          setEditingTagId(tag.id);
                        }}
                        className="p-1 text-text-secondary hover:text-text-primary rounded transition-colors"
                        title={t("tags.editTag")}
                      >
                        <Pencil size={10} />
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteTag(tag.id);
                        }}
                        className="p-1 text-text-secondary hover:text-error rounded transition-colors"
                        title={t("tags.deleteTag")}
                      >
                        <Trash2 size={10} />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
