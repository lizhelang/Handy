import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { produce } from "immer";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  ClipboardItem,
  ClipboardPageResult,
  ClipboardStats,
  ClipboardSettings,
  ClipboardViewMode,
  ClipboardSortOrder,
  ClipboardContentTypeFilter,
  ClipboardUpdatePayload,
} from "@/lib/types/clipboard";

const PAGE_SIZE = 20;

interface ClipboardStore {
  items: Record<number, ClipboardItem>;
  itemOrder: number[];

  searchQuery: string;
  contentTypeFilter: ClipboardContentTypeFilter;
  viewMode: ClipboardViewMode;
  sortOrder: ClipboardSortOrder;
  selectedId: number | null;
  previewItem: ClipboardItem | null;

  page: number;
  hasMore: boolean;
  isLoading: boolean;
  isSearching: boolean;

  stats: ClipboardStats | null;
  settings: ClipboardSettings | null;
  initialized: boolean;

  initialize: () => Promise<void>;
  loadItems: (reset?: boolean) => Promise<void>;
  loadMore: () => Promise<void>;
  search: (query: string) => Promise<void>;
  toggleFavorite: (id: number) => Promise<void>;
  togglePin: (id: number) => Promise<void>;
  deleteItem: (id: number) => Promise<void>;
  clearHistory: (keepPinned: boolean) => Promise<void>;
  copyItem: (id: number) => Promise<void>;
  setViewMode: (mode: ClipboardViewMode) => void;
  setSortOrder: (order: ClipboardSortOrder) => void;
  setContentTypeFilter: (filter: ClipboardContentTypeFilter) => void;
  setSelectedId: (id: number | null) => void;
  showPreview: (id: number) => void;
  hidePreview: () => void;
  refreshStats: () => Promise<void>;
  updateSettings: (settings: Partial<ClipboardSettings>) => Promise<void>;
}

export const useClipboardStore = create<ClipboardStore>()(
  subscribeWithSelector((set, get) => ({
    items: {},
    itemOrder: [],

    searchQuery: "",
    contentTypeFilter: "all",
    viewMode: "grid",
    sortOrder: "newest",
    selectedId: null,
    previewItem: null,

    page: 0,
    hasMore: true,
    isLoading: false,
    isSearching: false,

    stats: null,
    settings: null,
    initialized: false,

    initialize: async () => {
      if (get().initialized) return;
      const [stats, settings] = await Promise.all([
        invoke<ClipboardStats>("get_clipboard_stats"),
        invoke<ClipboardSettings>("get_clipboard_settings"),
      ]);
      set({ stats, settings, initialized: true });

      // Listen for real-time clipboard updates
      listen<ClipboardUpdatePayload>("clipboard-update-payload", (event) => {
        const payload = event.payload;
        set(
          produce((state) => {
            if (payload.action === "added") {
              const { item } = payload;
              state.items[item.id] = item;
              state.itemOrder = state.itemOrder.filter(
                (existingId: number) => existingId !== item.id,
              );
              state.itemOrder.unshift(item.id);
            } else if (payload.action === "deleted") {
              delete state.items[payload.id];
              state.itemOrder = state.itemOrder.filter(
                (existingId: number) => existingId !== payload.id,
              );
              if (state.selectedId === payload.id) {
                state.selectedId = null;
              }
              if (state.previewItem?.id === payload.id) {
                state.previewItem = null;
              }
            }
          }),
        );
      });

      await get().loadItems(true);
    },

    loadItems: async (reset = false) => {
      const { isLoading, page, sortOrder } = get();
      if (isLoading) return;
      if (reset) {
        set({ page: 0, hasMore: true });
      }
      const currentPage = reset ? 0 : page;
      set({ isLoading: true });

      try {
        const result = await invoke<ClipboardPageResult>(
          "get_clipboard_items",
          {
            page: currentPage,
            pageSize: PAGE_SIZE,
            sort: sortOrder,
          },
        );
        set(
          produce((state) => {
            if (reset) {
              state.items = {};
              state.itemOrder = [];
            }
            for (const item of result.items) {
              state.items[item.id] = item;
              if (!state.itemOrder.includes(item.id)) {
                state.itemOrder.push(item.id);
              }
            }
            state.hasMore = result.has_more;
            state.page = currentPage + 1;
          }),
        );
      } finally {
        set({ isLoading: false });
      }
    },

    loadMore: async () => {
      const { hasMore, isLoading, isSearching } = get();
      if (!hasMore || isLoading || isSearching) return;
      await get().loadItems(false);
    },

    search: async (query: string) => {
      set({ searchQuery: query, isSearching: true });
      if (!query.trim()) {
        set({ isSearching: false });
        await get().loadItems(true);
        return;
      }
      try {
        const results = await invoke<ClipboardItem[]>("search_clipboard", {
          query,
          contentType: get().contentTypeFilter,
        });
        set(
          produce((state) => {
            state.items = {};
            state.itemOrder = [];
            for (const item of results) {
              state.items[item.id] = item;
              state.itemOrder.push(item.id);
            }
            state.hasMore = false;
          }),
        );
      } finally {
        set({ isSearching: false });
      }
    },

    toggleFavorite: async (id: number) => {
      const item = get().items[id];
      if (!item) return;
      set(
        produce((state) => {
          state.items[id].is_favorite = !state.items[id].is_favorite;
        }),
      );
      try {
        await invoke("toggle_clipboard_favorite", { id });
        await get().refreshStats();
      } catch {
        set(
          produce((state) => {
            state.items[id].is_favorite = !state.items[id].is_favorite;
          }),
        );
      }
    },

    togglePin: async (id: number) => {
      const item = get().items[id];
      if (!item) return;
      set(
        produce((state) => {
          state.items[id].is_pinned = !state.items[id].is_pinned;
        }),
      );
      try {
        await invoke("toggle_clipboard_pin", { id });
        await get().refreshStats();
      } catch {
        set(
          produce((state) => {
            state.items[id].is_pinned = !state.items[id].is_pinned;
          }),
        );
      }
    },

    deleteItem: async (id: number) => {
      const item = get().items[id];
      if (!item) return;
      set(
        produce((state) => {
          delete state.items[id];
          state.itemOrder = state.itemOrder.filter((oid: number) => oid !== id);
          if (state.selectedId === id) state.selectedId = null;
          if (state.previewItem?.id === id) state.previewItem = null;
        }),
      );
      try {
        await invoke("delete_clipboard_item", { id });
        await get().refreshStats();
      } catch {
        set(
          produce((state) => {
            state.items[id] = item;
            state.itemOrder.push(id);
          }),
        );
      }
    },

    clearHistory: async (keepPinned: boolean) => {
      await invoke("clear_clipboard_history", { keepPinned });
      await get().loadItems(true);
      await get().refreshStats();
    },

    copyItem: async (id: number) => {
      await invoke("copy_clipboard_to_system", { id });
    },

    setViewMode: (mode) => set({ viewMode: mode }),

    setSortOrder: (order) => {
      set({ sortOrder: order });
      get().loadItems(true);
    },

    setContentTypeFilter: (filter) => {
      set({ contentTypeFilter: filter });
      const { searchQuery } = get();
      if (searchQuery.trim()) {
        get().search(searchQuery);
      } else {
        get().loadItems(true);
      }
    },

    setSelectedId: (id) => set({ selectedId: id }),

    showPreview: (id) => {
      const item = get().items[id];
      set({ previewItem: item ?? null });
    },

    hidePreview: () => set({ previewItem: null }),

    refreshStats: async () => {
      const stats = await invoke<ClipboardStats>("get_clipboard_stats");
      set({ stats });
    },

    updateSettings: async (newSettings) => {
      const settings = await invoke<ClipboardSettings>(
        "update_clipboard_settings",
        newSettings,
      );
      set({ settings });
    },
  })),
);
