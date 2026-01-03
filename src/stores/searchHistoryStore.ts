import { create } from "zustand";
import { persist } from "zustand/middleware";

const MAX_HISTORY_ITEMS = 10;

interface SearchHistoryState {
  history: string[];
  addToHistory: (query: string) => void;
  removeFromHistory: (query: string) => void;
  clearHistory: () => void;
}

export const useSearchHistoryStore = create<SearchHistoryState>()(
  persist(
    (set, get) => ({
      history: [],

      addToHistory: (query: string) => {
        const trimmedQuery = query.trim();
        if (!trimmedQuery) return;

        const { history } = get();
        // Remove duplicate if exists
        const filtered = history.filter((h) => h !== trimmedQuery);
        // Add to beginning and limit size
        const newHistory = [trimmedQuery, ...filtered].slice(0, MAX_HISTORY_ITEMS);
        set({ history: newHistory });
      },

      removeFromHistory: (query: string) => {
        const { history } = get();
        set({ history: history.filter((h) => h !== query) });
      },

      clearHistory: () => {
        set({ history: [] });
      },
    }),
    {
      name: "tidycraft-search-history",
    }
  )
);
