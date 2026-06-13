export type ClipboardContentType = "text" | "image" | "richtext" | "file";

export interface ClipboardItem {
  id: number;
  content_type: ClipboardContentType;
  content_preview: string;
  content_hash: string;
  full_text?: string;
  image_path?: string;
  source_app?: string;
  is_favorite: boolean;
  is_pinned: boolean;
  created_at: string;
  size_bytes: number;
}

export interface ClipboardStats {
  total_items: number;
  favorites_count: number;
  pinned_count: number;
  total_size_bytes: number;
}

export type ClipboardViewMode = "list" | "grid";
export type ClipboardSortOrder = "newest" | "oldest";
export type ClipboardContentTypeFilter = "all" | ClipboardContentType;

export interface ClipboardSettings {
  max_records: number;
  hotkey: string;
  confirm_mode: "copy" | "paste";
}

export interface ClipboardPageResult {
  items: ClipboardItem[];
  has_more: boolean;
}

export type ClipboardUpdatePayload =
  | { action: "added"; item: ClipboardItem }
  | { action: "updated"; item: ClipboardItem }
  | { action: "deleted"; id: number }
  | { action: "deleted_many"; ids: number[] }
  | { action: "cleared"; keep_pinned: boolean };
