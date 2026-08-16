import { useRef, useEffect } from "react";
import { Clock, X, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSearchHistoryStore } from "../stores/searchHistoryStore";

interface SearchHistoryProps {
  isVisible: boolean;
  onSelect: (query: string) => void;
  onClose: () => void;
  searchQuery: string;
}

export function SearchHistory({ isVisible, onSelect, onClose, searchQuery }: SearchHistoryProps) {
  const { t } = useTranslation();
  const { history, removeFromHistory, clearHistory } = useSearchHistoryStore();
  const panelRef = useRef<HTMLDivElement>(null);

  // Filter history based on current search query (trimmed, matching how
  // the asset filter itself treats the query).
  const filteredHistory = searchQuery.trim()
    ? history.filter((h) => h.toLowerCase().includes(searchQuery.trim().toLowerCase()))
    : history;

  // Close when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    if (isVisible) {
      document.addEventListener("mousedown", handleClickOutside);
    }
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isVisible, onClose]);

  if (!isVisible || filteredHistory.length === 0) return null;

  return (
    <div
      ref={panelRef}
      className="absolute left-0 right-0 top-full mt-1 bg-panel border border-line rounded-lg shadow-lg z-50 overflow-hidden"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-line bg-base">
        <span className="text-xs text-ink-2 flex items-center gap-1.5">
          <Clock size={12} />
          {t("search.recentSearches", "Recent Searches")}
        </span>
        <button
          onClick={(e) => {
            e.stopPropagation();
            clearHistory();
          }}
          className="text-xs text-ink-2 hover:text-err transition-colors flex items-center gap-1"
        >
          <Trash2 size={12} />
          {t("search.clearAll", "Clear")}
        </button>
      </div>

      {/* History Items */}
      <div className="max-h-48 overflow-y-auto">
        {filteredHistory.map((query, index) => (
          <div
            key={`${query}-${index}`}
            className="flex items-center gap-2 px-3 py-2 hover:bg-base transition-colors cursor-pointer group"
            onClick={() => onSelect(query)}
          >
            <Clock size={12} className="text-ink-2 shrink-0" />
            <span className="flex-1 text-sm text-ink truncate">{query}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                removeFromHistory(query);
              }}
              className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-panel text-ink-2 hover:text-err transition-all"
            >
              <X size={12} />
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
