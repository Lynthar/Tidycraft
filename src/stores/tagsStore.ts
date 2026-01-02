import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Tag, AssetTagsMap } from "../types/asset";

interface TagsState {
  // State
  tags: Tag[];
  assetTags: AssetTagsMap;
  isLoading: boolean;
  tagFilter: string | null; // Filter by tag ID

  // Actions
  loadTags: () => Promise<void>;
  createTag: (name: string, color: string) => Promise<Tag>;
  updateTag: (tagId: string, name?: string, color?: string) => Promise<void>;
  deleteTag: (tagId: string) => Promise<void>;
  addTagToAsset: (assetPath: string, tagId: string) => Promise<void>;
  removeTagFromAsset: (assetPath: string, tagId: string) => Promise<void>;
  addTagToAssets: (assetPaths: string[], tagId: string) => Promise<void>;
  setTagFilter: (tagId: string | null) => void;
  getAssetTags: (assetPath: string) => Tag[];
}

export const useTagsStore = create<TagsState>((set, get) => ({
  tags: [],
  assetTags: {},
  isLoading: false,
  tagFilter: null,

  loadTags: async () => {
    set({ isLoading: true });
    try {
      const [tags, assetTags] = await Promise.all([
        invoke<Tag[]>("get_all_tags"),
        invoke<AssetTagsMap>("get_all_asset_tags"),
      ]);
      set({ tags, assetTags, isLoading: false });
    } catch (err) {
      console.error("Failed to load tags:", err);
      set({ isLoading: false });
    }
  },

  createTag: async (name: string, color: string) => {
    const tag = await invoke<Tag>("create_tag", { name, color });
    set((state) => ({ tags: [...state.tags, tag] }));
    return tag;
  },

  updateTag: async (tagId: string, name?: string, color?: string) => {
    await invoke<Tag>("update_tag", { tagId, name, color });
    set((state) => ({
      tags: state.tags.map((t) =>
        t.id === tagId
          ? { ...t, name: name ?? t.name, color: color ?? t.color }
          : t
      ),
    }));
  },

  deleteTag: async (tagId: string) => {
    await invoke("delete_tag", { tagId });
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
    await invoke("add_tag_to_asset", { assetPath, tagId });
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
    await invoke("remove_tag_from_asset", { assetPath, tagId });
    const { assetTags } = get();
    set({
      assetTags: {
        ...assetTags,
        [assetPath]: (assetTags[assetPath] || []).filter((t) => t.id !== tagId),
      },
    });
  },

  addTagToAssets: async (assetPaths: string[], tagId: string) => {
    await invoke("add_tag_to_assets", { assetPaths, tagId });
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
    set({ tagFilter: tagId });
  },

  getAssetTags: (assetPath: string) => {
    return get().assetTags[assetPath] || [];
  },
}));
