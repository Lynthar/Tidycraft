import { useState, useRef, useEffect } from "react";
import { Filter, X, ChevronDown, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useProjectStore } from "../stores/projectStore";

export function AdvancedFiltersPanel() {
  const { t } = useTranslation();
  const { advancedFilters, setAdvancedFilters, resetAdvancedFilters, scanResult } = useProjectStore();
  const [isOpen, setIsOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  // Get unique extensions from scan result
  const availableExtensions = scanResult
    ? [...new Set(scanResult.assets.map((a) => a.extension.toLowerCase()))].sort()
    : [];

  // Check if any filters are active
  const hasActiveFilters =
    advancedFilters.minSize !== null ||
    advancedFilters.maxSize !== null ||
    advancedFilters.minWidth !== null ||
    advancedFilters.maxWidth !== null ||
    advancedFilters.minHeight !== null ||
    advancedFilters.maxHeight !== null ||
    advancedFilters.extensions.length > 0;

  // Close panel when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handleSizeChange = (field: "minSize" | "maxSize", value: string) => {
    const numValue = value ? parseFloat(value) * 1024 * 1024 : null; // Convert MB to bytes
    setAdvancedFilters({ [field]: numValue });
  };

  const handleDimensionChange = (
    field: "minWidth" | "maxWidth" | "minHeight" | "maxHeight",
    value: string
  ) => {
    const numValue = value ? parseInt(value, 10) : null;
    setAdvancedFilters({ [field]: numValue });
  };

  const toggleExtension = (ext: string) => {
    const current = advancedFilters.extensions;
    const newExtensions = current.includes(ext)
      ? current.filter((e) => e !== ext)
      : [...current, ext];
    setAdvancedFilters({ extensions: newExtensions });
  };

  const handleReset = () => {
    resetAdvancedFilters();
  };

  return (
    <div className="relative" ref={panelRef}>
      <button
        onClick={() => setIsOpen(!isOpen)}
        className={`flex items-center gap-1.5 px-2.5 py-1.5 text-sm rounded border transition-colors ${
          hasActiveFilters
            ? "bg-primary/10 border-primary text-primary"
            : "bg-background border-border text-text-secondary hover:text-text-primary hover:border-primary/50"
        }`}
      >
        <Filter size={14} />
        <span>{t("filters.advanced", "Filters")}</span>
        {hasActiveFilters && (
          <span className="ml-1 px-1.5 py-0.5 text-xs bg-primary text-white rounded-full">
            {[
              advancedFilters.minSize !== null || advancedFilters.maxSize !== null ? 1 : 0,
              advancedFilters.minWidth !== null ||
              advancedFilters.maxWidth !== null ||
              advancedFilters.minHeight !== null ||
              advancedFilters.maxHeight !== null
                ? 1
                : 0,
              advancedFilters.extensions.length > 0 ? 1 : 0,
            ].reduce((a, b) => a + b, 0)}
          </span>
        )}
        <ChevronDown size={12} className={`transition-transform ${isOpen ? "rotate-180" : ""}`} />
      </button>

      {isOpen && (
        <div className="absolute right-0 top-full mt-2 w-80 bg-card-bg border border-border rounded-lg shadow-xl z-50">
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b border-border">
            <h3 className="font-medium text-sm">{t("filters.advancedFilters", "Advanced Filters")}</h3>
            <div className="flex items-center gap-2">
              {hasActiveFilters && (
                <button
                  onClick={handleReset}
                  className="flex items-center gap-1 px-2 py-1 text-xs text-text-secondary hover:text-text-primary transition-colors"
                >
                  <RotateCcw size={12} />
                  {t("filters.reset", "Reset")}
                </button>
              )}
              <button
                onClick={() => setIsOpen(false)}
                className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary"
              >
                <X size={14} />
              </button>
            </div>
          </div>

          <div className="p-4 space-y-4 max-h-[400px] overflow-y-auto">
            {/* File Size */}
            <div>
              <label className="block text-xs text-text-secondary uppercase mb-2">
                {t("filters.fileSize", "File Size (MB)")}
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  placeholder={t("filters.min", "Min")}
                  value={advancedFilters.minSize ? (advancedFilters.minSize / 1024 / 1024).toFixed(2) : ""}
                  onChange={(e) => handleSizeChange("minSize", e.target.value)}
                  className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                />
                <span className="text-text-secondary">-</span>
                <input
                  type="number"
                  placeholder={t("filters.max", "Max")}
                  value={advancedFilters.maxSize ? (advancedFilters.maxSize / 1024 / 1024).toFixed(2) : ""}
                  onChange={(e) => handleSizeChange("maxSize", e.target.value)}
                  className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                />
              </div>
            </div>

            {/* Image Dimensions */}
            <div>
              <label className="block text-xs text-text-secondary uppercase mb-2">
                {t("filters.dimensions", "Image Dimensions (px)")}
              </label>
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs text-text-secondary w-8">{t("filters.width", "W")}:</span>
                  <input
                    type="number"
                    placeholder={t("filters.min", "Min")}
                    value={advancedFilters.minWidth ?? ""}
                    onChange={(e) => handleDimensionChange("minWidth", e.target.value)}
                    className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                  />
                  <span className="text-text-secondary">-</span>
                  <input
                    type="number"
                    placeholder={t("filters.max", "Max")}
                    value={advancedFilters.maxWidth ?? ""}
                    onChange={(e) => handleDimensionChange("maxWidth", e.target.value)}
                    className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                  />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-text-secondary w-8">{t("filters.height", "H")}:</span>
                  <input
                    type="number"
                    placeholder={t("filters.min", "Min")}
                    value={advancedFilters.minHeight ?? ""}
                    onChange={(e) => handleDimensionChange("minHeight", e.target.value)}
                    className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                  />
                  <span className="text-text-secondary">-</span>
                  <input
                    type="number"
                    placeholder={t("filters.max", "Max")}
                    value={advancedFilters.maxHeight ?? ""}
                    onChange={(e) => handleDimensionChange("maxHeight", e.target.value)}
                    className="flex-1 px-2 py-1.5 text-sm bg-background border border-border rounded text-text-primary focus:outline-none focus:border-primary"
                  />
                </div>
              </div>
            </div>

            {/* Extensions */}
            <div>
              <label className="block text-xs text-text-secondary uppercase mb-2">
                {t("filters.extensions", "File Extensions")}
              </label>
              <div className="flex flex-wrap gap-1.5 max-h-32 overflow-y-auto">
                {availableExtensions.map((ext) => (
                  <button
                    key={ext}
                    onClick={() => toggleExtension(ext)}
                    className={`px-2 py-1 text-xs rounded transition-colors ${
                      advancedFilters.extensions.includes(ext)
                        ? "bg-primary text-white"
                        : "bg-background text-text-secondary hover:text-text-primary hover:bg-background/80"
                    }`}
                  >
                    .{ext}
                  </button>
                ))}
                {availableExtensions.length === 0 && (
                  <span className="text-xs text-text-secondary italic">
                    {t("filters.noExtensions", "No files scanned")}
                  </span>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
