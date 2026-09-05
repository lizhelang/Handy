import { create } from "zustand";
import { commands, events } from "@/bindings";
import type { UnifiedHistoryQuery, UnifiedHistoryRevision } from "@/bindings";
import type {
  UnifiedHistoryItem,
  UnifiedHistorySource,
  UnifiedHistoryContentType,
} from "@/lib/types/unifiedHistory";

const PAGE_SIZE = 40;
type Filters = {
  search: string;
  source: UnifiedHistorySource;
  contentType: UnifiedHistoryContentType;
  starredOnly: boolean;
};
interface UnifiedHistoryState extends Filters {
  items: UnifiedHistoryItem[];
  selectedId: string | null;
  previewOpen: boolean;
  revisions: UnifiedHistoryRevision[];
  loading: boolean;
  revisionsLoading: boolean;
  error: "load" | "refresh" | null;
  subscriptionError: boolean;
  revisionsError: boolean;
  hasMore: boolean;
  setFilters: (filters: Partial<Filters>) => void;
  load: (append?: boolean) => Promise<void>;
  refresh: () => Promise<void>;
  select: (id: string | null) => void;
  closePreview: () => void;
  loadRevisions: () => Promise<void>;
  subscribe: () => () => void;
}

// Each webview owns this module and its subscription lifecycle independently.
export const createUnifiedHistoryStore = () => {
  let requestId = 0;
  let revisionRequestId = 0;
  let subscribers = 0;
  let subscriptionId = 0;
  let unlisten: (() => void) | null = null;
  let subscribing = false;
  const attach = () => {
    if (subscribing || unlisten || subscribers === 0) return;
    subscribing = true;
    const id = ++subscriptionId;
    void events.unifiedHistoryUpdate
      .listen(() => {
        if (id === subscriptionId) void store.getState().load();
      })
      .then((stop) => {
        if (id !== subscriptionId || subscribers === 0) stop();
        else {
          unlisten = stop;
          store.setState({ subscriptionError: false });
          void store.getState().load();
        }
      })
      .catch(() => {
        if (id === subscriptionId) store.setState({ subscriptionError: true });
      })
      .finally(() => {
        if (id === subscriptionId) subscribing = false;
      });
  };
  const store = create<UnifiedHistoryState>((set, get) => ({
    search: "",
    source: "all",
    contentType: "all",
    starredOnly: false,
    items: [],
    selectedId: null,
    previewOpen: false,
    revisions: [],
    loading: false,
    revisionsLoading: false,
    error: null,
    subscriptionError: false,
    revisionsError: false,
    hasMore: false,
    setFilters: (filters) => {
      ++requestId;
      ++revisionRequestId;
      set({
        ...filters,
        items: [],
        selectedId: null,
        previewOpen: false,
        revisions: [],
        revisionsLoading: false,
      });
      void get().load();
    },
    load: async (append = false) => {
      if (append && (get().loading || !get().hasMore)) return;
      const id = ++requestId;
      const { search, source, contentType, starredOnly, items } = get();
      const query: UnifiedHistoryQuery = {
        search: search.trim() || null,
        source_kind: source === "all" ? null : source,
        content_type: contentType === "all" ? null : contentType,
        starred_only: starredOnly,
        limit: PAGE_SIZE,
        offset: append ? items.length : 0,
      };
      set({ loading: true, error: null });
      try {
        const result = await commands.getUnifiedHistory(query);
        if (id !== requestId) return;
        if (result.status === "error") throw new Error("load");
        const next = append
          ? [
              ...items,
              ...result.data.filter(
                (item) =>
                  !items.some((existing) => existing.item_id === item.item_id),
              ),
            ]
          : result.data;
        const selectedId = next.some(
          (item) => item.item_id === get().selectedId,
        )
          ? get().selectedId
          : null;
        if (selectedId === null) ++revisionRequestId;
        set({
          items: next,
          selectedId,
          previewOpen: selectedId !== null && get().previewOpen,
          hasMore: result.data.length === PAGE_SIZE,
          ...(selectedId === null
            ? { revisions: [], revisionsLoading: false, revisionsError: false }
            : {}),
        });
        if (selectedId && get().previewOpen) void get().loadRevisions();
      } catch {
        if (id === requestId) set({ error: "load" });
      } finally {
        if (id === requestId) set({ loading: false });
      }
    },
    refresh: async () => {
      attach();
      try {
        const result = await commands.refreshUnifiedHistory();
        if (result.status === "error") throw new Error("refresh");
        await get().load();
      } catch {
        set({ error: "refresh" });
      }
    },
    select: (id) => {
      ++revisionRequestId;
      set({
        selectedId: id,
        previewOpen: id !== null,
        revisions: [],
        revisionsError: false,
        revisionsLoading: false,
      });
      if (id) void get().loadRevisions();
    },
    closePreview: () => {
      ++revisionRequestId;
      set({ previewOpen: false, revisions: [], revisionsLoading: false });
    },
    loadRevisions: async () => {
      const selectedId = get().selectedId;
      if (!selectedId) return;
      const id = ++revisionRequestId;
      set({ revisionsLoading: true, revisionsError: false });
      try {
        const result = await commands.getUnifiedHistoryRevisions(selectedId);
        if (id !== revisionRequestId) return;
        if (result.status === "error") throw new Error("revisions");
        set({ revisions: result.data });
      } catch {
        if (id === revisionRequestId) set({ revisionsError: true });
      } finally {
        if (id === revisionRequestId) set({ revisionsLoading: false });
      }
    },
    subscribe: () => {
      subscribers += 1;
      if (subscribers === 1) {
        attach();
        void get().load();
      }
      let disposed = false;
      return () => {
        if (disposed) return;
        disposed = true;
        subscribers -= 1;
        if (subscribers === 0) {
          ++subscriptionId;
          subscribing = false;
          ++requestId;
          ++revisionRequestId;
          unlisten?.();
          unlisten = null;
          set({
            items: [],
            selectedId: null,
            previewOpen: false,
            revisions: [],
            loading: false,
            revisionsLoading: false,
          });
        }
      };
    },
  }));
  return store;
};

export const useUnifiedHistoryStore = createUnifiedHistoryStore();
