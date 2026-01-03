import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ColumnId =
  | "name"
  | "type"
  | "size"
  | "dimensions"
  | "vertices"
  | "faces"
  | "duration"
  | "sampleRate"
  | "extension"
  | "tags";

export interface ColumnConfig {
  id: ColumnId;
  visible: boolean;
  width: number;
}

interface ColumnState {
  columns: ColumnConfig[];
  setColumnVisible: (id: ColumnId, visible: boolean) => void;
  setColumnWidth: (id: ColumnId, width: number) => void;
  resetColumns: () => void;
  moveColumn: (fromIndex: number, toIndex: number) => void;
}

const DEFAULT_COLUMNS: ColumnConfig[] = [
  { id: "name", visible: true, width: 0 }, // flex
  { id: "type", visible: true, width: 96 },
  { id: "size", visible: true, width: 96 },
  { id: "dimensions", visible: true, width: 128 },
  { id: "tags", visible: true, width: 120 },
  { id: "vertices", visible: false, width: 96 },
  { id: "faces", visible: false, width: 96 },
  { id: "duration", visible: false, width: 80 },
  { id: "sampleRate", visible: false, width: 96 },
  { id: "extension", visible: false, width: 80 },
];

// Version for migration - increment when DEFAULT_COLUMNS changes
const COLUMNS_VERSION = 2;

export const useColumnStore = create<ColumnState>()(
  persist(
    (set) => ({
      columns: DEFAULT_COLUMNS,

      setColumnVisible: (id, visible) =>
        set((state) => ({
          columns: state.columns.map((col) =>
            col.id === id ? { ...col, visible } : col
          ),
        })),

      setColumnWidth: (id, width) =>
        set((state) => ({
          columns: state.columns.map((col) =>
            col.id === id ? { ...col, width } : col
          ),
        })),

      resetColumns: () => set({ columns: DEFAULT_COLUMNS }),

      moveColumn: (fromIndex, toIndex) =>
        set((state) => {
          const newColumns = [...state.columns];
          const [removed] = newColumns.splice(fromIndex, 1);
          newColumns.splice(toIndex, 0, removed);
          return { columns: newColumns };
        }),
    }),
    {
      name: "tidycraft-columns",
      version: COLUMNS_VERSION,
      migrate: (persistedState: unknown, version: number) => {
        // If version is outdated, reset to defaults to include new columns like tags
        if (version < COLUMNS_VERSION) {
          return { columns: DEFAULT_COLUMNS };
        }
        return persistedState as ColumnState;
      },
    }
  )
);
