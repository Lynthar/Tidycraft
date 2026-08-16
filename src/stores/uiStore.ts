import { create } from "zustand";
import { useProjectStore } from "./projectStore";

// Transient UI state for app-level overlays. Global rather than App-local so any
// component can trigger them without prop drilling.

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

export type AiTagCategory = "type" | "style" | "mood" | "subject" | "other";

/// Per-category colour so a card shows at a glance which tags are types, styles
/// or mood. Not tied to CSS vars because both panels apply them as `${color}1F`
/// (12% alpha) backgrounds, which CSS vars do not support directly.
export const AI_CATEGORY_COLORS: Record<AiTagCategory, string> = {
  type: "#3b82f6", // blue
  style: "#a855f7", // purple
  mood: "#f97316", // orange
  subject: "#10b981", // green
  other: "#6b7280", // gray
};

export interface AiSuggestedTag {
  label: string;
  category: AiTagCategory;
  confidence: number;
  /** Whether this label matches an existing project tag or the model coined it.
   *  Older cached responses lack the field; the backend defaults it to `new`. */
  source?: "existing" | "new";
}

// --- Learning mode mirrors of Rust llm::learning structs ---

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

  /** AI Analyze (cost preview + consent) modal. `aiAnalyzePaths` is the asset
   *  selection that triggered it, passed rather than read from selectionStore so
   *  the modal sees the snapshot at trigger time. */
  aiAnalyzeOpen: boolean;
  aiAnalyzePaths: string[];

  /** AI Result review panel. Holds the response payload so the panel
   *  doesn't have to re-invoke. Cleared on close. */
  aiResultOpen: boolean;
  aiResultData: AiTagResponse | null;
  aiResultPaths: string[];

  /** Learning-setup modal: theme/goal + sampling depth + cost preview. */
  learnSetupOpen: boolean;
  /** Review panel for an LLM learning result. `learnReviewData` carries either a
   *  fresh result or a loaded `AiRulesDoc` rehydrated into a synthetic one, so
   *  "Review rules" works without re-running the call. */
  learnReviewOpen: boolean;
  learnReviewData: AiLearningResult | null;

  /** Dependency-graph modal. `depGraphAssetPath` is the asset whose local
   *  graph to show, snapshotted at trigger time. */
  depGraphOpen: boolean;
  depGraphAssetPath: string | null;

  /** True while a fullscreen media lightbox is up. The lightboxes are
   *  AssetPreview-local state and mirror here so `isBlockingOverlayOpen` can gate
   *  global shortcuts. */
  lightboxOpen: boolean;

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
  setLightboxOpen: (open: boolean) => void;
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
  lightboxOpen: false,
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
  setLightboxOpen: (open) => set({ lightboxOpen: open }),
}));

/// Live count of mounted `ModalShell`s, which is how the file-op dialogs held in
/// component-local state become visible to global shortcuts. A plain counter, not
/// zustand state: it is read from key handlers and never rendered.
let openModalShells = 0;

/// Called by `ModalShell` on mount; the returned function releases on unmount.
export function registerModalShell(): () => void {
  openModalShells += 1;
  let released = false;
  return () => {
    if (released) return; // StrictMode double-invokes cleanup in dev
    released = true;
    openModalShells -= 1;
  };
}

/// True when a blocking, backdrop-covered overlay is open. Global window-level
/// key handlers consult this so they don't fire underneath a modal. Excludes
/// `aiPanelOpen`, a floating panel with no backdrop.
export function isBlockingOverlayOpen(): boolean {
  const s = useUiStore.getState();
  return (
    openModalShells > 0 ||
    s.cmdkOpen ||
    s.settingsOpen ||
    s.tagManagerOpen ||
    s.aiAnalyzeOpen ||
    s.aiResultOpen ||
    s.learnSetupOpen ||
    s.learnReviewOpen ||
    s.depGraphOpen ||
    s.lightboxOpen
  );
}

// Dismiss the AI / learning write-flows and the floating AI Tag panel on
// active-project change: their contents were computed for the previous project,
// and every apply path resolves the project id live. Read-only overlays stay.
useProjectStore.subscribe((state, prev) => {
  if (state.activeProjectId === prev.activeProjectId) return;
  const ui = useUiStore.getState();
  if (ui.aiAnalyzeOpen) ui.setAiAnalyzeOpen(false);
  if (ui.aiResultOpen) ui.setAiResultOpen(false);
  if (ui.learnSetupOpen) ui.setLearnSetupOpen(false);
  if (ui.learnReviewOpen) ui.setLearnReviewOpen(false);
  if (ui.aiPanelOpen) ui.setAiPanelOpen(false);
});
