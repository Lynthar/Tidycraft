import { useState } from "react";
import { X, Plus, Pencil, Trash2, Check } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useTagsStore } from "../stores/tagsStore";
import { cn } from "../lib/utils";
import type { Tag } from "../types/asset";

const PRESET_COLORS = [
  "#ef4444", // red
  "#f97316", // orange
  "#eab308", // yellow
  "#22c55e", // green
  "#14b8a6", // teal
  "#3b82f6", // blue
  "#8b5cf6", // violet
  "#ec4899", // pink
  "#6b7280", // gray
];

interface TagManagerProps {
  isOpen: boolean;
  onClose: () => void;
}

export function TagManager({ isOpen, onClose }: TagManagerProps) {
  const { t } = useTranslation();
  const { tags, createTag, updateTag, deleteTag } = useTagsStore();
  const [isCreating, setIsCreating] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newColor, setNewColor] = useState(PRESET_COLORS[0]);
  const [editName, setEditName] = useState("");
  const [editColor, setEditColor] = useState("");

  if (!isOpen) return null;

  const handleCreate = async () => {
    if (!newName.trim()) return;
    await createTag(newName.trim(), newColor);
    setNewName("");
    setNewColor(PRESET_COLORS[0]);
    setIsCreating(false);
  };

  const handleStartEdit = (tag: Tag) => {
    setEditingId(tag.id);
    setEditName(tag.name);
    setEditColor(tag.color);
  };

  const handleSaveEdit = async () => {
    if (!editingId || !editName.trim()) return;
    await updateTag(editingId, editName.trim(), editColor);
    setEditingId(null);
  };

  const handleDelete = async (tagId: string) => {
    if (window.confirm(t("tags.confirmDelete"))) {
      await deleteTag(tagId);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-full max-w-md">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold">{t("tags.manageTitle")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4 max-h-96 overflow-auto">
          {/* Tag List */}
          <div className="space-y-2">
            {tags.map((tag) => (
              <div
                key={tag.id}
                className="flex items-center gap-2 p-2 rounded hover:bg-background group"
              >
                {editingId === tag.id ? (
                  <>
                    <div className="flex gap-1">
                      {PRESET_COLORS.map((c) => (
                        <button
                          key={c}
                          onClick={() => setEditColor(c)}
                          className={cn(
                            "w-5 h-5 rounded-full border-2",
                            editColor === c ? "border-white" : "border-transparent"
                          )}
                          style={{ backgroundColor: c }}
                        />
                      ))}
                    </div>
                    <input
                      type="text"
                      value={editName}
                      onChange={(e) => setEditName(e.target.value)}
                      className="flex-1 px-2 py-1 text-sm bg-background border border-border rounded"
                      autoFocus
                    />
                    <button
                      onClick={handleSaveEdit}
                      className="p-1 rounded hover:bg-primary/20 text-primary"
                    >
                      <Check size={16} />
                    </button>
                    <button
                      onClick={() => setEditingId(null)}
                      className="p-1 rounded hover:bg-background text-text-secondary"
                    >
                      <X size={16} />
                    </button>
                  </>
                ) : (
                  <>
                    <span
                      className="w-4 h-4 rounded-full shrink-0"
                      style={{ backgroundColor: tag.color }}
                    />
                    <span className="flex-1 text-sm">{tag.name}</span>
                    <button
                      onClick={() => handleStartEdit(tag)}
                      className="p-1 rounded hover:bg-background text-text-secondary opacity-0 group-hover:opacity-100 transition-opacity"
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      onClick={() => handleDelete(tag.id)}
                      className="p-1 rounded hover:bg-red-500/20 text-red-400 opacity-0 group-hover:opacity-100 transition-opacity"
                    >
                      <Trash2 size={14} />
                    </button>
                  </>
                )}
              </div>
            ))}

            {tags.length === 0 && !isCreating && (
              <p className="text-center text-text-secondary py-4">
                {t("tags.noTags")}
              </p>
            )}

            {/* Create New Tag */}
            {isCreating ? (
              <div className="flex items-center gap-2 p-2 bg-background rounded">
                <div className="flex gap-1">
                  {PRESET_COLORS.map((c) => (
                    <button
                      key={c}
                      onClick={() => setNewColor(c)}
                      className={cn(
                        "w-5 h-5 rounded-full border-2",
                        newColor === c ? "border-white" : "border-transparent"
                      )}
                      style={{ backgroundColor: c }}
                    />
                  ))}
                </div>
                <input
                  type="text"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder={t("tags.tagName")}
                  className="flex-1 px-2 py-1 text-sm bg-card-bg border border-border rounded"
                  autoFocus
                  onKeyDown={(e) => e.key === "Enter" && handleCreate()}
                />
                <button
                  onClick={handleCreate}
                  className="p-1 rounded hover:bg-primary/20 text-primary"
                >
                  <Check size={16} />
                </button>
                <button
                  onClick={() => {
                    setIsCreating(false);
                    setNewName("");
                  }}
                  className="p-1 rounded hover:bg-background text-text-secondary"
                >
                  <X size={16} />
                </button>
              </div>
            ) : (
              <button
                onClick={() => setIsCreating(true)}
                className="flex items-center gap-2 w-full p-2 text-sm text-text-secondary hover:text-text-primary hover:bg-background rounded transition-colors"
              >
                <Plus size={16} />
                {t("tags.createTag")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
