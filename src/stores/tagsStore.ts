import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Tag, AssetTagsMap } from "../types/asset";
import { useProjectStore } from "./projectStore";

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
  updateTag: (tagId: string, name?: string, color?: string) => Promise<void>;
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
      set({ tags, assetTags, isLoading: false });
    } catch (err) {
      console.error("Failed to load tags:", err);
      set({ isLoading: false });
    }
  },

  createTag: async (name: string, color: string) => {
    const projectId = activeProjectId();
    if (!projectId) return null;
    const tag = await invoke<Tag>("create_tag", { projectId, name, color });
    set((state) => ({ tags: [...state.tags, tag] }));
    return tag;
  },

  updateTag: async (tagId: string, name?: string, color?: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    await invoke<Tag>("update_tag", { projectId, tagId, name, color });
    set((state) => ({
      tags: state.tags.map((t) =>
        t.id === tagId
          ? { ...t, name: name ?? t.name, color: color ?? t.color }
          : t
      ),
    }));
  },

  deleteTag: async (tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    await invoke("delete_tag", { projectId, tagId });
    set((state) => ({
      tags: state.tags.filter((t) => t.id !== tagId),
      assetTags: Object.fromEntries(
        Object.entries(state.assetTags).map(([path, tags]) => [
          path,
          tags.filter((t) => t.id !== tagId),
        ])
      ),
    }));
  },

  addTagToAsset: async (assetPath: string, tagId: string) => {
    const projectId = activeProjectId();
    if (!projectId) return;
    await invoke("add_tag_to_asset", { projectId, assetPath, tagId });
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
    await invoke("remove_tag_from_asset", { projectId, assetPath, tagId });
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
    await invoke("add_tag_to_assets", { projectId, assetPaths, tagId });
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
