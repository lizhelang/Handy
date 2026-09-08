import { useCallback, useEffect, useMemo, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import type { UnifiedHistoryItem, UnifiedHistoryPatch } from "@/bindings";
import type {
  ClipboardItem,
  ClipboardSettings,
  ClipboardStats,
} from "@/lib/types/clipboard";
import { useUnifiedHistoryStore } from "@/stores/unifiedHistoryStore";

export type OverlayHistoryItem = Pick<
  ClipboardItem,
  | "title"
  | "content_type"
  | "content_preview"
  | "full_text"
  | "source_app"
  | "is_favorite"
  | "is_pinned"
  | "created_at"
> & { id: string; original: UnifiedHistoryItem };

export function adaptHistoryItem(item: UnifiedHistoryItem): OverlayHistoryItem {
  return {
    id: item.item_id,
    original: item,
    title: item.title,
    content_type:
      item.content_type === "files"
        ? "file"
        : item.content_type === "html" || item.content_type === "rtf"
          ? "richtext"
          : item.content_type === "image"
            ? "image"
            : "text",
    content_preview: item.text ?? "",
    full_text: item.text ?? undefined,
    source_app: item.source_app ?? undefined,
    is_favorite: item.starred,
    is_pinned: item.pinned,
    created_at: new Date(item.created_at_ms).toISOString(),
  };
}

export function useHistoryImage(item: OverlayHistoryItem) {
  const [url, setUrl] = useState<string | null>(null);
  const { item_id, revision, content_type } = item.original;
  useEffect(() => {
    let active = true;
    setUrl(null);
    if (content_type === "image") {
      void commands
        .getUnifiedHistoryAsset(item_id, revision)
        .then((result) => {
          if (active && result.status === "ok" && result.data)
            setUrl(convertFileSrc(result.data));
        })
        .catch(() => {
          if (active) setUrl(null);
        });
    }
    return () => {
      active = false;
    };
  }, [item_id, revision, content_type]);
  return url;
}

export function useSharedClipboard() {
  const history = useUnifiedHistoryStore();
  const [settings, setSettings] = useState<ClipboardSettings | null>(null);
  const [stats, setStats] = useState<ClipboardStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [operations] = useState(() => new Map<string, string>());
  const items = useMemo(
    () => history.items.map(adaptHistoryItem),
    [history.items],
  );
  useEffect(() => history.subscribe(), [history.subscribe]);
  useEffect(() => {
    void Promise.all([
      invoke<ClipboardSettings>("get_clipboard_settings"),
      invoke<ClipboardStats>("get_clipboard_stats"),
    ])
      .then(([nextSettings, nextStats]) => {
        setSettings(nextSettings);
        setStats(nextStats);
      })
      .catch(() => setError("unifiedHistory.feedback.failed"));
  }, []);
  const mutate = useCallback(
    async (
      id: string,
      patch?: Partial<UnifiedHistoryPatch>,
      snapshot?: UnifiedHistoryItem,
    ) => {
      const item = snapshot ?? items.find((item) => item.id === id)?.original;
      if (!item) {
        setError("unifiedHistory.feedback.updateFailed");
        return;
      }
      const key = JSON.stringify([id, item.revision, patch ?? "delete"]);
      const operationId = operations.get(key) ?? crypto.randomUUID();
      operations.set(key, operationId);
      try {
        if (patch) {
          const result = await commands.updateUnifiedHistoryItem(
            id,
            item.revision,
            operationId,
            {
              starred: null,
              pinned: null,
              title: null,
              clear_title: false,
              text: null,
              ...patch,
            },
          );
          if (result.status === "error") throw new Error(result.error);
        } else {
          const deleted = await invoke<boolean>("delete_unified_history_item", {
            itemId: id,
            expectedRevision: item.revision,
            operationId,
          });
          if (!deleted) throw new Error("Delete rejected");
        }
        operations.delete(key);
        setError(null);
        await history.load();
      } catch {
        setError("unifiedHistory.feedback.updateFailed");
      }
    },
    [history.load, operations, items],
  );
  const clearHistory = async (keepPinned: boolean) => {
    try {
      await invoke("clear_clipboard_history", { keepPinned });
      await history.refresh();
      setStats(await invoke<ClipboardStats>("get_clipboard_stats"));
    } catch {
      setError("unifiedHistory.feedback.updateFailed");
    }
  };
  const updateSettings = async (patch: Partial<ClipboardSettings>) => {
    try {
      setSettings(
        await invoke<ClipboardSettings>("update_clipboard_settings", patch),
      );
    } catch {
      setError("unifiedHistory.feedback.updateFailed");
    }
  };
  return {
    items,
    settings,
    stats,
    error:
      error ??
      (history.error
        ? "unifiedHistory.errors.load"
        : history.subscriptionError
          ? "unifiedHistory.errors.subscription"
          : null),
    mutate,
    clearHistory,
    updateSettings,
  };
}
