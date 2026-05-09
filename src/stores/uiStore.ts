import { create } from "zustand";

/// Tracks transient UI state for app-level overlays (modals + command palette).
/// Lives in a global store rather than App.tsx local state so that any
/// component can trigger them without prop drilling — CommandPalette in
/// particular needs to open Settings / TagManager from inside an action.

/// Mirrors the backend `llm::TagResponse` struct (see src-tauri/src/llm/mod.rs).
/// Kept inline rather than re-exported from a types/ file because it's only
/// consumed by uiStore + AIResultPanel and the source of truth is Rust.
export interface AiTagResponse {
  suggestions: AiTagSuggestion[];
  usage: {
    input_tokens: number;
    output_tokens: number;
    cached: boolean;
  };
}

export interface AiTagSuggestion {
  asset_path: string;
  tags: AiSuggestedTag[];
}

export interface AiSuggestedTag {
  label: string;
  category: "type" | "style" | "mood" | "subject" | "other";
  confidence: number;
  /** Whether this label matches an existing project tag (`existing`) or
   *  the LLM coined it fresh (`new`). Older cached responses (PROMPT_VERSION
   *  1) lack the field — backend serializes a default of `new` so they
   *  load cleanly; frontend treats undefined the same way. */
  source?: "existing" | "new";
}

interface UiState {
  cmdkOpen: boolean;
  settingsOpen: boolean;
  tagManagerOpen: boolean;
  aiPanelOpen: boolean;

  /** AI Analyze (cost preview + consent) modal. `aiAnalyzePaths` is
   *  the asset selection that triggered it — passed instead of read
   *  from selectionStore directly so multi-select + right-click both
   *  work and the modal sees the snapshot at trigger time. */
  aiAnalyzeOpen: boolean;
  aiAnalyzePaths: string[];

  /** AI Result review panel. Holds the response payload so the panel
   *  doesn't have to re-invoke. Cleared on close. */
  aiResultOpen: boolean;
  aiResultData: AiTagResponse | null;
  aiResultPaths: string[];

  setCmdkOpen: (open: boolean) => void;
  toggleCmdk: () => void;
  setSettingsOpen: (open: boolean) => void;
  setTagManagerOpen: (open: boolean) => void;
  setAiPanelOpen: (open: boolean) => void;

  /** Open with `(true, paths)` from a trigger; close with `(false)`. */
  setAiAnalyzeOpen: (open: boolean, paths?: string[]) => void;
  /** Open with `(true, data, paths)` after a successful suggest call;
   *  close with `(false)`. */
  setAiResultOpen: (
    open: boolean,
    data?: AiTagResponse,
    paths?: string[]
  ) => void;
}

export const useUiStore = create<UiState>((set, get) => ({
  cmdkOpen: false,
  settingsOpen: false,
  tagManagerOpen: false,
  aiPanelOpen: false,
  aiAnalyzeOpen: false,
  aiAnalyzePaths: [],
  aiResultOpen: false,
  aiResultData: null,
  aiResultPaths: [],
  setCmdkOpen: (open) => set({ cmdkOpen: open }),
  toggleCmdk: () => set({ cmdkOpen: !get().cmdkOpen }),
  setSettingsOpen: (open) => set({ settingsOpen: open }),
  setTagManagerOpen: (open) => set({ tagManagerOpen: open }),
  setAiPanelOpen: (open) => set({ aiPanelOpen: open }),
  setAiAnalyzeOpen: (open, paths) =>
    set({
      aiAnalyzeOpen: open,
      aiAnalyzePaths: open ? paths ?? [] : [],
    }),
  setAiResultOpen: (open, data, paths) =>
    set({
      aiResultOpen: open,
      aiResultData: open ? data ?? null : null,
      aiResultPaths: open ? paths ?? [] : [],
    }),
}));
