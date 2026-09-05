import type { UnifiedHistoryItem } from "@/bindings";

export type { UnifiedHistoryItem, UnifiedHistoryRevision } from "@/bindings";
export type UnifiedHistorySource = "all" | "voice" | "clipboard";
export type UnifiedHistoryContentType =
  | "all"
  | "text"
  | "image"
  | "files"
  | "html"
  | "rtf";
export type UnifiedHistoryPatch = {
  starred?: boolean;
  pinned?: boolean;
  title?: string | null;
  text?: string;
};
export type UnifiedHistoryActionResult = {
  status:
    | "confirmed"
    | "dispatched"
    | "pending_target"
    | "uncertain"
    | "rejected"
    | "failed";
};

export interface UnifiedHistoryProps {
  onCopy: (item: UnifiedHistoryItem) => Promise<UnifiedHistoryActionResult>;
  onInsert: (item: UnifiedHistoryItem) => Promise<UnifiedHistoryActionResult>;
  /** Resolve only after the source mutation and index synchronization succeed. */
  onUpdate: (
    item: UnifiedHistoryItem,
    patch: UnifiedHistoryPatch,
  ) => Promise<void>;
  /** Resolve a managed attachment to a permitted URL. Never use asset_ref as a path. */
  resolveAsset: (item: UnifiedHistoryItem) => Promise<string | null>;
}
