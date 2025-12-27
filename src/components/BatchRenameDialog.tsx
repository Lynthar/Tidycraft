import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { X, RefreshCw, Check, AlertCircle } from "lucide-react";

type RenameOperationType =
  | "FindReplace"
  | "AddPrefix"
  | "AddSuffix"
  | "RemovePrefix"
  | "RemoveSuffix"
  | "ToLowercase"
  | "ToUppercase"
  | "ToTitleCase";

interface RenamePreview {
  original_path: string;
  original_name: string;
  new_name: string;
  will_change: boolean;
}

interface BatchRenameResult {
  success_count: number;
  error_count: number;
  errors: string[];
}

interface BatchRenameDialogProps {
  isOpen: boolean;
  onClose: () => void;
  selectedPaths: string[];
  onComplete: () => void;
}

export function BatchRenameDialog({
  isOpen,
  onClose,
  selectedPaths,
  onComplete,
}: BatchRenameDialogProps) {
  const { t } = useTranslation();
  const [operationType, setOperationType] = useState<RenameOperationType>("FindReplace");
  const [findText, setFindText] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const [prefixSuffix, setPrefixSuffix] = useState("");
  const [previews, setPreviews] = useState<RenamePreview[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [result, setResult] = useState<BatchRenameResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Build the operation object for the backend
  const buildOperation = () => {
    switch (operationType) {
      case "FindReplace":
        return { FindReplace: { find: findText, replace: replaceText } };
      case "AddPrefix":
        return { AddPrefix: { prefix: prefixSuffix } };
      case "AddSuffix":
        return { AddSuffix: { suffix: prefixSuffix } };
      case "RemovePrefix":
        return { RemovePrefix: { prefix: prefixSuffix } };
      case "RemoveSuffix":
        return { RemoveSuffix: { suffix: prefixSuffix } };
      case "ToLowercase":
        return "ToLowercase";
      case "ToUppercase":
        return "ToUppercase";
      case "ToTitleCase":
        return "ToTitleCase";
    }
  };

  // Generate preview whenever inputs change
  useEffect(() => {
    if (!isOpen || selectedPaths.length === 0) return;

    const debounceTimer = setTimeout(async () => {
      try {
        const operation = buildOperation();
        const result = await invoke<RenamePreview[]>("preview_batch_rename", {
          paths: selectedPaths,
          operation,
        });
        setPreviews(result);
        setError(null);
      } catch (err) {
        setError(String(err));
        setPreviews([]);
      }
    }, 300);

    return () => clearTimeout(debounceTimer);
  }, [isOpen, selectedPaths, operationType, findText, replaceText, prefixSuffix]);

  const handleExecute = async () => {
    setIsLoading(true);
    setError(null);

    try {
      const operation = buildOperation();
      const result = await invoke<BatchRenameResult>("execute_batch_rename", {
        paths: selectedPaths,
        operation,
      });
      setResult(result);

      if (result.success_count > 0) {
        onComplete();
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleClose = () => {
    setResult(null);
    setError(null);
    setPreviews([]);
    setFindText("");
    setReplaceText("");
    setPrefixSuffix("");
    onClose();
  };

  const changedCount = previews.filter((p) => p.will_change).length;

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-card-bg border border-border rounded-lg w-[600px] max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-lg font-semibold">
            {t("batchRename.title", "Batch Rename")} ({selectedPaths.length} files)
          </h2>
          <button
            onClick={handleClose}
            className="p-1 rounded hover:bg-background text-text-secondary"
          >
            <X size={20} />
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-auto p-4 space-y-4">
          {/* Operation Type */}
          <div>
            <label className="block text-sm font-medium mb-2">
              {t("batchRename.operation", "Operation")}
            </label>
            <select
              value={operationType}
              onChange={(e) => setOperationType(e.target.value as RenameOperationType)}
              className="w-full h-9 px-3 bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
            >
              <option value="FindReplace">{t("batchRename.findReplace", "Find & Replace")}</option>
              <option value="AddPrefix">{t("batchRename.addPrefix", "Add Prefix")}</option>
              <option value="AddSuffix">{t("batchRename.addSuffix", "Add Suffix")}</option>
              <option value="RemovePrefix">{t("batchRename.removePrefix", "Remove Prefix")}</option>
              <option value="RemoveSuffix">{t("batchRename.removeSuffix", "Remove Suffix")}</option>
              <option value="ToLowercase">{t("batchRename.toLowercase", "To Lowercase")}</option>
              <option value="ToUppercase">{t("batchRename.toUppercase", "To Uppercase")}</option>
              <option value="ToTitleCase">{t("batchRename.toTitleCase", "To Title Case")}</option>
            </select>
          </div>

          {/* Operation-specific inputs */}
          {operationType === "FindReplace" && (
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium mb-2">
                  {t("batchRename.find", "Find")}
                </label>
                <input
                  type="text"
                  value={findText}
                  onChange={(e) => setFindText(e.target.value)}
                  placeholder={t("batchRename.findPlaceholder", "Text to find...")}
                  className="w-full h-9 px-3 bg-background border border-border rounded text-text-primary placeholder:text-text-secondary focus:outline-none focus:border-primary"
                />
              </div>
              <div>
                <label className="block text-sm font-medium mb-2">
                  {t("batchRename.replace", "Replace")}
                </label>
                <input
                  type="text"
                  value={replaceText}
                  onChange={(e) => setReplaceText(e.target.value)}
                  placeholder={t("batchRename.replacePlaceholder", "Replace with...")}
                  className="w-full h-9 px-3 bg-background border border-border rounded text-text-primary placeholder:text-text-secondary focus:outline-none focus:border-primary"
                />
              </div>
            </div>
          )}

          {(operationType === "AddPrefix" ||
            operationType === "AddSuffix" ||
            operationType === "RemovePrefix" ||
            operationType === "RemoveSuffix") && (
            <div>
              <label className="block text-sm font-medium mb-2">
                {operationType.includes("Prefix")
                  ? t("batchRename.prefix", "Prefix")
                  : t("batchRename.suffix", "Suffix")}
              </label>
              <input
                type="text"
                value={prefixSuffix}
                onChange={(e) => setPrefixSuffix(e.target.value)}
                placeholder={
                  operationType.includes("Prefix")
                    ? t("batchRename.prefixPlaceholder", "Enter prefix...")
                    : t("batchRename.suffixPlaceholder", "Enter suffix...")
                }
                className="w-full h-9 px-3 bg-background border border-border rounded text-text-primary placeholder:text-text-secondary focus:outline-none focus:border-primary"
              />
            </div>
          )}

          {/* Preview */}
          <div>
            <label className="block text-sm font-medium mb-2">
              {t("batchRename.preview", "Preview")} ({changedCount} {t("batchRename.willChange", "will change")})
            </label>
            <div className="max-h-48 overflow-auto bg-background border border-border rounded">
              {previews.length === 0 ? (
                <div className="p-4 text-center text-text-secondary">
                  {t("batchRename.noPreview", "No preview available")}
                </div>
              ) : (
                <table className="w-full text-sm">
                  <thead className="bg-card-bg sticky top-0">
                    <tr>
                      <th className="text-left p-2 border-b border-border">
                        {t("batchRename.original", "Original")}
                      </th>
                      <th className="text-left p-2 border-b border-border">
                        {t("batchRename.newName", "New Name")}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {previews.slice(0, 50).map((preview, index) => (
                      <tr
                        key={index}
                        className={preview.will_change ? "bg-primary/10" : ""}
                      >
                        <td className="p-2 border-b border-border truncate max-w-[200px]">
                          {preview.original_name}
                        </td>
                        <td className="p-2 border-b border-border truncate max-w-[200px]">
                          {preview.will_change ? (
                            <span className="text-primary">{preview.new_name}</span>
                          ) : (
                            <span className="text-text-secondary">{preview.new_name}</span>
                          )}
                        </td>
                      </tr>
                    ))}
                    {previews.length > 50 && (
                      <tr>
                        <td colSpan={2} className="p-2 text-center text-text-secondary">
                          ... and {previews.length - 50} more
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              )}
            </div>
          </div>

          {/* Error */}
          {error && (
            <div className="flex items-center gap-2 p-3 bg-error/10 border border-error/30 rounded text-error text-sm">
              <AlertCircle size={16} />
              {error}
            </div>
          )}

          {/* Result */}
          {result && (
            <div
              className={`flex items-center gap-2 p-3 rounded text-sm ${
                result.error_count > 0
                  ? "bg-warning/10 border border-warning/30 text-warning"
                  : "bg-green-500/10 border border-green-500/30 text-green-400"
              }`}
            >
              <Check size={16} />
              <span>
                {result.success_count} {t("batchRename.renamed", "renamed")}
                {result.error_count > 0 && `, ${result.error_count} ${t("batchRename.failed", "failed")}`}
              </span>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 p-4 border-t border-border">
          <button
            onClick={handleClose}
            className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary transition-colors"
          >
            {t("common.cancel", "Cancel")}
          </button>
          <button
            onClick={handleExecute}
            disabled={isLoading || changedCount === 0}
            className="flex items-center gap-2 px-4 py-2 text-sm bg-primary text-white rounded hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isLoading ? (
              <>
                <RefreshCw size={14} className="animate-spin" />
                {t("batchRename.renaming", "Renaming...")}
              </>
            ) : (
              <>
                <Check size={14} />
                {t("batchRename.rename", "Rename")} ({changedCount})
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
