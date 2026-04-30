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

export type AssetViewMode = "list" | "grid";

export interface ColumnConfig {
  id: ColumnId;
  visible: boolean;
  width: number;
}

interface ColumnState {
  columns: ColumnConfig[];
  viewMode: AssetViewMode;
  setColumnVisible: (id: ColumnId, visible: boolean) => void;
  setColumnWidth: (id: ColumnId, width: number) => void;
  resetColumns: () => void;
  moveColumn: (fromIndex: number, toIndex: number) => void;
  setViewMode: (mode: AssetViewMode) => void;
  toggleViewMode: () => void;
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

// Version for migration - increment when DEFAULT_COLUMNS changes or when
// new persisted fields are added (e.g. viewMode in v3).
const COLUMNS_VERSION = 3;

const DEFAULT_VIEW_MODE: AssetViewMode = "list";

export const useColumnStore = create<ColumnState>()(
  persist(
    (set, get) => ({
      columns: DEFAULT_COLUMNS,
      viewMode: DEFAULT_VIEW_MODE,

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

      setViewMode: (mode) => set({ viewMode: mode }),

      toggleViewMode: () =>
        set({ viewMode: get().viewMode === "list" ? "grid" : "list" }),
    }),
    {
      name: "tidycraft-columns",
      version: COLUMNS_VERSION,
      migrate: (persistedState: unknown, version: number) => {
        // v3 added viewMode. Older persisted blobs may have either an
        // outdated columns shape (pre-v2) or no viewMode field — both
        // resolve to: keep what we can, fill the rest with defaults.
        const prev = (persistedState as Partial<ColumnState>) ?? {};
        const columns =
          version < 2 || !Array.isArray(prev.columns)
            ? DEFAULT_COLUMNS
            : prev.columns;
        const viewMode: AssetViewMode =
          prev.viewMode === "grid" ? "grid" : DEFAULT_VIEW_MODE;
        return { columns, viewMode } as ColumnState;
      },
    }
  )
);
