import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Tag, AssetTagsMap } from "../types/asset";
import {
  useProjectStore,
  registerTagFilterBridge,
  registerTagsSyncBridge,
} from "./projectStore";
import { useToastStore } from "./toastStore";
import i18n from "../i18n";

interface TagsState {
  // State
  tags: Tag[];
  assetTags: AssetTagsMap;
  isLoading: boolean;
  tagFilter: string | null; // Single tag filter (deprecated, kept for compatibility)
  tagFilters: string[]; // Multiple tag filters

  // Actions
  loadTags: () => Promise<void>;
  createTag: (name: string, color: string) => Promise<Tag | null>;
  /** Update one or more tag fields. Pass `null` to `description` to
   *  explicitly clear it; pass `undefined` (omit) to leave unchanged.
   *  `name` / `color` follow the same omit-to-keep convention. */
  updateTag: (
    tagId: string,
    name?: string,
    color?: string,
    description?: string | null
  ) => Promise<void>;
  deleteTag: (tagId: string) => Promise<void>;
  addTagToAsset: (assetPath: string, tagId: string) => Promise<void>;
  removeTagFromAsset: (assetPath: string, tagId: string) => Promise<void>;
  addTagToAssets: (assetPaths: string[], tagId: string) => Promise<void>;
  setTagFilter: (tagId: string | null) => void;
  toggleTagFilter: (tagId: string, multiSelect?: boolean) => void;
  clearTagFilters: () => void;
  getAssetTags: (assetPath: string) => Tag[];
}

// Tags are scoped to the currently active project on the backend; this store
// mirrors that project's tags. Switching projects means re-loading.
const activeProjectId = (): string | null =>
  useProjectStore.getState().activeProjectId;

/// Run a tag mutation against the backend, reporting failure instead of
/// throwing. Every write action's declared type is `Promise<void>` or
/// `Promise<Tag | null>`, but a failed `invoke` used to reject straight through
/// them — and every call site does a bare `await`, so a failed delete became an
/// unhandled rejection, no toast, and an edit form stuck in its pending state.
/// `null` means "it didn't happen"; callers must not patch the mirror.
async function tagWrite<T>(op: () => Promise<T>): Promise<T | null> {
  try {
    return await op();
  } catch (err) {
    useToastStore.getState().push({
      kind: "error",
      message: i18n.t("tags.writeFailed", { reason: String(err) }),
    });
    return null;
  }
}

export const useTagsStore = create<TagsState>((set, get) => ({
  tags: [],
  assetTags: {},
  isLoading: false,
  tagFilter: null,
  tagFilters: [],

  loadTags: async () => {
    const projectId = activeProjectId();
    if (!projectId) {
      set({ tags: [], assetTags: {}, isLoading: false });
      return;
    }
    set({ isLoading: true });
    try {
      const [tags, assetTags] = await Promise.all([
        invoke<Tag[]>("get_all_tags", { projectId }),
        invoke<AssetTagsMap>("get_all_asset_tags", { projectId }),
      ]);
      // Drop the result if the user switched projects mid-flight — otherwise a
      // slow response for project A lands after project B's load and overwrites
      // B's tags (same snapshot-and-check pattern as StatsDashboard).
      if (activeProjectId() !== projectId) return;
      // Prune filters whose tag ids don't exist in the loaded set — after a
      // project switch the previous project's filter ids are dead here, and
      // a dead id in an AND filter silently hides every asset.
      set((state) => {
        const valid = new Set(tags.map((t) => t.id));
        const tagFilters = state.tagFilters.filter((id) => valid.has(id));
        return {
          tags,
          assetTags,
          isLoading: false,
          tagFilters,
          tagFilter:
            state.tagFilter && valid.has(state.tagFilter)
              ? state.tagFilter
              : tagFilters[0] ?? null,
        };
      });
    } catch (err) {
      console.error("Failed to load tags:", err);
      if (activeProjectId() === projectId) set({ isLoading: false });
    }
  },

  createTag: async (name: string, color: string) => {
    const projectId = activeProjectId();
    if (!projectId) return null;
    const tag = await tagWrite(() =>
      invoke<Tag>("create_tag", { projectId, name, color })
    );
    if (!tag) return null;
    // The backend write above targeted the snapshot projectId and stays
    // valid; the in-memory mirror, however, belongs to whatever project is
    // active NOW. If the user switched projects mid-flight, skip the mirror
    // update (same snapshot-and-check as loadTags) — otherwise this project's
    // tag shows up as a phantom in the other project's UI until its next
    // loadTags. Switching back re-loads the truth from disk. Same guard in
    // every mutation action below.
    if (activeProjectId() !== projectId) return tag;
    set((state) => ({ tags: [...state.tags, tag] }));
    return tag;
  },

  updateTag: async (
    tagId: string,
    name?: string,
    color?: string,
    description?: string | null
  ) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    // Backend accepts `Option<Option<String>>` for description, mapping:
    //   undefined → don't send the field (leave unchanged)
    //   null / "" → clear the description
    //   string    → set to Some(s)
    // IMPORTANT: plain serde deserializes JSON `null` to the OUTER `None`
    // ("leave unchanged"), NOT `Some(None)` — so sending `null` is a silent
    // no-op and the description could never actually be cleared. We send an
    // empty string instead; the backend (`tags.rs::update_tag`) normalizes a
    // blank string to `None`. An omitted field still means "leave unchanged".
    const payload: Record<string, unknown> = { projectId, tagId, name, color };
    if (description !== undefined) {
      payload.description = description ?? ""; // null → "" so the clear lands
    }
    if ((await tagWrite(() => invoke<Tag>("update_tag", payload))) === null) return;
    if (activeProjectId() !== projectId) return; // mid-flight project switch — see createTag
    set((state) => ({
      tags: state.tags.map((t) =>
        t.id === tagId
          ? {
              ...t,
              name: name ?? t.name,
              color: color ?? t.color,
              description:
                description === undefined
                  ? t.description
                  : description ?? undefined,
            }
          : t
      ),
    }));
  },

  deleteTag: async (tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    if ((await tagWrite(() => invoke("delete_tag", { projectId, tagId }))) === null) return;
    if (activeProjectId() !== projectId) return; // mid-flight project switch — see createTag
    set((state) => {
      // Also prune the id from the active filters — a deleted tag left in an
      // AND filter can never match, so it would silently hide every asset
      // with no pill left to un-click.
      const tagFilters = state.tagFilters.filter((id) => id !== tagId);
      return {
        tags: state.tags.filter((t) => t.id !== tagId),
        assetTags: Object.fromEntries(
          Object.entries(state.assetTags).map(([path, tags]) => [
            path,
            tags.filter((t) => t.id !== tagId),
          ])
        ),
        tagFilters,
        tagFilter:
          state.tagFilter === tagId ? tagFilters[0] ?? null : state.tagFilter,
      };
    });
  },

  addTagToAsset: async (assetPath: string, tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    if (
      (await tagWrite(() =>
        invoke("add_tag_to_asset", { projectId, assetPath, tagId })
      )) === null
    )
      return;
    if (activeProjectId() !== projectId) return; // mid-flight project switch — see createTag
    const { tags, assetTags } = get();
    const tag = tags.find((t) => t.id === tagId);
    if (tag) {
      const currentTags = assetTags[assetPath] || [];
      if (!currentTags.some((t) => t.id === tagId)) {
        set({
          assetTags: {
            ...assetTags,
            [assetPath]: [...currentTags, tag],
          },
        });
      }
    }
  },

  removeTagFromAsset: async (assetPath: string, tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    if (
      (await tagWrite(() =>
        invoke("remove_tag_from_asset", { projectId, assetPath, tagId })
      )) === null
    )
      return;
    if (activeProjectId() !== projectId) return; // mid-flight project switch — see createTag
    const { assetTags } = get();
    set({
      assetTags: {
        ...assetTags,
        [assetPath]: (assetTags[assetPath] || []).filter((t) => t.id !== tagId),
      },
    });
  },

  addTagToAssets: async (assetPaths: string[], tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    if (
      (await tagWrite(() =>
        invoke("add_tag_to_assets", { projectId, assetPaths, tagId })
      )) === null
    )
      return;
    if (activeProjectId() !== projectId) return; // mid-flight project switch — see createTag
    const { tags, assetTags } = get();
    const tag = tags.find((t) => t.id === tagId);
    if (tag) {
      const newAssetTags = { ...assetTags };
      for (const path of assetPaths) {
        const currentTags = newAssetTags[path] || [];
        if (!currentTags.some((t) => t.id === tagId)) {
          newAssetTags[path] = [...currentTags, tag];
        }
      }
      set({ assetTags: newAssetTags });
    }
  },

  setTagFilter: (tagId: string | null) => {
    set({ tagFilter: tagId, tagFilters: tagId ? [tagId] : [] });
  },

  toggleTagFilter: (tagId: string, multiSelect = false) => {
    const { tagFilters } = get();
    if (multiSelect) {
      // Multi-select: toggle this tag in the list
      if (tagFilters.includes(tagId)) {
        const newFilters = tagFilters.filter((id) => id !== tagId);
        set({ tagFilters: newFilters, tagFilter: newFilters[0] || null });
      } else {
        set({ tagFilters: [...tagFilters, tagId], tagFilter: tagId });
      }
    } else {
      // Single select: if already selected, deselect; otherwise select only this
      if (tagFilters.length === 1 && tagFilters[0] === tagId) {
        set({ tagFilters: [], tagFilter: null });
      } else {
        set({ tagFilters: [tagId], tagFilter: tagId });
      }
    }
  },

  clearTagFilters: () => {
    set({ tagFilters: [], tagFilter: null });
  },

  getAssetTags: (assetPath: string) => {
    return get().assetTags[assetPath] || [];
  },
}));

// Re-load tags whenever the active project changes.
useProjectStore.subscribe((state, prev) => {
  if (state.activeProjectId !== prev.activeProjectId) {
    useTagsStore.getState().loadTags();
  }
});

// Give projectStore.locateAsset a way to clear tag filters that would hide
// the located asset (it can't import this store — see the bridge's comment).
// The exclusion test mirrors AssetList's AND semantics: an asset is visible
// only when it carries EVERY active filter tag.
registerTagFilterBridge({
  clearIfFiltering: (path: string) => {
    const { tagFilters, assetTags, clearTagFilters } = useTagsStore.getState();
    if (tagFilters.length === 0) return;
    const tagIds = (assetTags[path] || []).map((t) => t.id);
    if (!tagFilters.every((id) => tagIds.includes(id))) {
      clearTagFilters();
    }
  },
});

// The watcher migrates tag bindings across external renames (and reaps them
// on deletes) backend-side; re-pull the mirror so chips and filters follow
// without a project switch. Fired only for the active project.
registerTagsSyncBridge({
  reloadTags: () => {
    void useTagsStore.getState().loadTags();
  },
});
