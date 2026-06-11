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

// --- Learning mode mirrors of Rust llm::learning structs ---

export type AiTagCategory = "type" | "style" | "mood" | "subject" | "other";

export interface AiInferredConventions {
  naming: string;
  directories: string;
  existing_tag_meanings: Record<string, string>;
}

export interface AiNewTagHint {
  label: string;
  category: AiTagCategory;
  confidence: number;
}

export interface AiSampleTagSet {
  asset_path: string;
  matched_existing: string[];
  suggested_new: AiNewTagHint[];
}

export interface AiTagGap {
  label: string;
  category: AiTagCategory;
  reason: string;
}

/** Tagged union mirroring Rust `LearnedRule` (serde tag = "kind"). */
export type AiLearnedRule =
  | { kind: "filename_token"; pattern: string; tags: string[]; confidence: number }
  | { kind: "path_prefix"; pattern: string; tags: string[]; confidence: number }
  | { kind: "path_segment"; pattern: string; tags: string[]; confidence: number }
  | { kind: "filename_regex"; pattern: string; tags: string[]; confidence: number };

export interface AiLearningResult {
  inferred_conventions: AiInferredConventions;
  sample_tags: AiSampleTagSet[];
  tag_gaps: AiTagGap[];
  rules: AiLearnedRule[];
  usage: { input_tokens: number; output_tokens: number; cached: boolean };
}

/** On-disk shape from `tidycraft.ai.toml` — mirrors Rust `AiRulesDoc`. */
export interface AiRulesDoc {
  last_learned: string;
  prompt_version: number;
  sampling_depth: number;
  provider_used: string;
  model_used: string;
  rules: AiLearnedRule[];
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

  /** Learning-setup modal: theme/goal + sampling depth + cost preview. */
  learnSetupOpen: boolean;
  /** Review panel for an LLM learning result. `learnReviewData` carries
   *  either a fresh result (just-finished learning run) or a loaded
   *  AiRulesDoc rehydrated into a synthetic LearningResult so "Review
   *  rules" works without re-running the call. */
  learnReviewOpen: boolean;
  learnReviewData: AiLearningResult | null;

  /** Dependency-graph modal. `depGraphAssetPath` is the asset whose local
   *  graph to show, snapshotted at trigger time. */
  depGraphOpen: boolean;
  depGraphAssetPath: string | null;

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

  setLearnSetupOpen: (open: boolean) => void;
  setLearnReviewOpen: (open: boolean, data?: AiLearningResult) => void;
  setDepGraphOpen: (open: boolean, assetPath?: string) => void;
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
  learnSetupOpen: false,
  learnReviewOpen: false,
  learnReviewData: null,
  depGraphOpen: false,
  depGraphAssetPath: null,
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
  setLearnSetupOpen: (open) => set({ learnSetupOpen: open }),
  setLearnReviewOpen: (open, data) =>
    set({
      learnReviewOpen: open,
      learnReviewData: open ? data ?? null : null,
    }),
  setDepGraphOpen: (open, assetPath) =>
    set({
      depGraphOpen: open,
      depGraphAssetPath: open ? assetPath ?? null : null,
    }),
}));

/// True when a blocking, backdrop-covered overlay is open (command palette,
/// settings, tag manager, the AI analyze/result modals, the learning modals,
/// or the dependency graph). Global window-level key handlers (Del, Ctrl+1/2/3,
/// rescan, …) consult this so they don't fire underneath a modal.
///
/// Deliberately EXCLUDES `aiPanelOpen` — the AI Tag panel is a floating side
/// panel with no backdrop, so the asset list behind it stays interactive.
/// AssetList's own file-op dialogs (rename / batch / delete / move) are
/// component-local state and are checked by AssetList separately.
export function isBlockingOverlayOpen(): boolean {
  const s = useUiStore.getState();
  return (
    s.cmdkOpen ||
    s.settingsOpen ||
    s.tagManagerOpen ||
    s.aiAnalyzeOpen ||
    s.aiResultOpen ||
    s.learnSetupOpen ||
    s.learnReviewOpen ||
    s.depGraphOpen
  );
}
