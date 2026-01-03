import { useState, useEffect, useRef } from "react";
import { X, AlertCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

interface RenameDialogProps {
  isOpen: boolean;
  onClose: () => void;
  assetPath: string;
  currentName: string;
  onComplete: () => void;
}

export function RenameDialog({
  isOpen,
  onClose,
  assetPath,
  currentName,
  onComplete,
}: RenameDialogProps) {
  const { t } = useTranslation();
  const [newName, setNewName] = useState(currentName);
  const [isRenaming, setIsRenaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Extract name without extension
  const lastDotIndex = currentName.lastIndexOf(".");
  const nameWithoutExt = lastDotIndex > 0 ? currentName.substring(0, lastDotIndex) : currentName;
  const extension = lastDotIndex > 0 ? currentName.substring(lastDotIndex) : "";

  useEffect(() => {
    if (isOpen) {
      setNewName(nameWithoutExt);
      setError(null);
      // Focus and select text after a short delay
      setTimeout(() => {
        if (inputRef.current) {
          inputRef.current.focus();
          inputRef.current.select();
        }
      }, 50);
    }
  }, [isOpen, nameWithoutExt]);

  const handleRename = async () => {
    const trimmedName = newName.trim();
    if (!trimmedName) {
      setError(t("rename.emptyName", "Name cannot be empty"));
      return;
    }

    if (trimmedName === nameWithoutExt) {
      onClose();
      return;
    }

    // Check for invalid characters
    if (/[<>:"/\\|?*]/.test(trimmedName)) {
      setError(t("rename.invalidChars", "Name contains invalid characters"));
      return;
    }

    setIsRenaming(true);
    setError(null);

    try {
      const newFullName = trimmedName + extension;
      await invoke("rename_file", {
        oldPath: assetPath,
        newName: newFullName,
      });
      onComplete();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsRenaming(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !isRenaming) {
      handleRename();
    } else if (e.key === "Escape") {
      onClose();
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-[400px] max-w-[90vw]">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold">{t("contextMenu.rename")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4">
          <div className="mb-4">
            <label className="block text-sm text-text-secondary mb-1">
              {t("rename.currentName", "Current name")}
            </label>
            <div className="text-sm text-text-primary bg-background px-3 py-2 rounded truncate">
              {currentName}
            </div>
          </div>

          <div className="mb-4">
            <label className="block text-sm text-text-secondary mb-1">
              {t("rename.newName", "New name")}
            </label>
            <div className="flex items-center gap-1">
              <input
                ref={inputRef}
                type="text"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={handleKeyDown}
                disabled={isRenaming}
                className="flex-1 px-3 py-2 text-sm bg-background border border-border rounded
                           text-text-primary focus:outline-none focus:border-primary transition-colors
                           disabled:opacity-50"
                placeholder={t("rename.enterName", "Enter new name...")}
              />
              {extension && (
                <span className="text-sm text-text-secondary">{extension}</span>
              )}
            </div>
          </div>

          {error && (
            <div className="flex items-center gap-2 p-2 mb-4 bg-error/10 border border-error/30 rounded text-sm text-error">
              <AlertCircle size={14} />
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-border">
          <button
            onClick={onClose}
            disabled={isRenaming}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors disabled:opacity-50"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={handleRename}
            disabled={isRenaming || !newName.trim()}
            className="px-4 py-2 text-sm bg-primary text-white rounded hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            {isRenaming ? t("batchRename.renaming") : t("contextMenu.rename")}
          </button>
        </div>
      </div>
    </div>
  );
}
