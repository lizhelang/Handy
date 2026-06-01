import React, { useEffect, useRef, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Search, Star, Pin, Copy, Check, X } from "lucide-react";
import { useClipboardStore } from "@/stores/clipboardStore";
import type { ClipboardItem } from "@/lib/types/clipboard";
import {
  formatClipboardRelativeTime,
  getClipboardItemLabel,
} from "@/components/clipboard/utils";
import "./ClipboardOverlay.css";

const ClipboardOverlay: React.FC = () => {
  const { t } = useTranslation();
  const items = useClipboardStore((s) => s.items);
  const itemOrder = useClipboardStore((s) => s.itemOrder);
  const searchQuery = useClipboardStore((s) => s.searchQuery);
  const initialized = useClipboardStore((s) => s.initialized);
  const initialize = useClipboardStore((s) => s.initialize);
  const search = useClipboardStore((s) => s.search);
  const toggleFavorite = useClipboardStore((s) => s.toggleFavorite);
  const togglePin = useClipboardStore((s) => s.togglePin);
  const copyItem = useClipboardStore((s) => s.copyItem);
  const deleteItem = useClipboardStore((s) => s.deleteItem);

  const [selectedIndex, setSelectedIndex] = useState(0);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!initialized) initialize();
  }, [initialized, initialize]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const orderedItems = itemOrder.map((id) => items[id]).filter(Boolean);
  const filteredItems = searchQuery.trim()
    ? orderedItems.filter(
        (item) =>
          item.content_preview
            .toLowerCase()
            .includes(searchQuery.toLowerCase()) ||
          (item.full_text &&
            item.full_text.toLowerCase().includes(searchQuery.toLowerCase())),
      )
    : orderedItems;

  useEffect(() => {
    setSelectedIndex(0);
  }, [searchQuery, itemOrder.length]);

  const handleCopy = useCallback(
    async (id: number) => {
      await copyItem(id);
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1500);
    },
    [copyItem],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
        case "j":
          e.preventDefault();
          setSelectedIndex((i) => Math.min(i + 1, filteredItems.length - 1));
          break;
        case "ArrowUp":
        case "k":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          if (filteredItems[selectedIndex]) {
            handleCopy(filteredItems[selectedIndex].id);
          }
          break;
        case "f":
          if (
            !e.metaKey &&
            !e.ctrlKey &&
            document.activeElement !== searchRef.current
          ) {
            e.preventDefault();
            if (filteredItems[selectedIndex]) {
              toggleFavorite(filteredItems[selectedIndex].id);
            }
          }
          break;
        case "d":
          if (
            !e.metaKey &&
            !e.ctrlKey &&
            document.activeElement !== searchRef.current
          ) {
            e.preventDefault();
            if (filteredItems[selectedIndex]) {
              deleteItem(filteredItems[selectedIndex].id);
            }
          }
          break;
        case "p":
          if (
            !e.metaKey &&
            !e.ctrlKey &&
            document.activeElement !== searchRef.current
          ) {
            e.preventDefault();
            if (filteredItems[selectedIndex]) {
              togglePin(filteredItems[selectedIndex].id);
            }
          }
          break;
        case "Escape":
          e.preventDefault();
          if (searchQuery) {
            search("");
          }
          break;
      }
    },
    [
      filteredItems,
      selectedIndex,
      handleCopy,
      toggleFavorite,
      togglePin,
      deleteItem,
      search,
      searchQuery,
    ],
  );

  useEffect(() => {
    const selected = listRef.current?.children[selectedIndex] as HTMLElement;
    selected?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div className="clipboard-overlay" onKeyDown={handleKeyDown}>
      <div className="clipboard-overlay-header">
        <Search className="clipboard-overlay-search-icon" />
        <input
          ref={searchRef}
          type="text"
          className="clipboard-overlay-search"
          placeholder={t("settings.clipboard.overlay.search")}
          value={searchQuery}
          onChange={(e) => search(e.target.value)}
        />
        {searchQuery && (
          <button
            className="clipboard-overlay-clear"
            onClick={() => search("")}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      <div className="clipboard-overlay-list" ref={listRef}>
        {filteredItems.length === 0 ? (
          <div className="clipboard-overlay-empty">
            {t("settings.clipboard.emptyTitle")}
          </div>
        ) : (
          filteredItems.map((item, index) => (
            <OverlayItem
              key={item.id}
              item={item}
              isSelected={index === selectedIndex}
              isCopied={copiedId === item.id}
              onCopy={() => handleCopy(item.id)}
              onToggleFavorite={() => toggleFavorite(item.id)}
              onTogglePin={() => togglePin(item.id)}
              onSelect={() => setSelectedIndex(index)}
              index={index}
            />
          ))
        )}
      </div>

      <div className="clipboard-overlay-hint">
        {t("settings.clipboard.overlay.hint")}
      </div>
    </div>
  );
};

interface OverlayItemProps {
  item: ClipboardItem;
  isSelected: boolean;
  isCopied: boolean;
  onCopy: () => void;
  onToggleFavorite: () => void;
  onTogglePin: () => void;
  onSelect: () => void;
  index: number;
}

const OverlayItem: React.FC<OverlayItemProps> = ({
  item,
  isSelected,
  isCopied,
  onCopy,
  onToggleFavorite,
  onTogglePin,
  onSelect,
  index,
}) => {
  const { t, i18n } = useTranslation();

  return (
    <div
      className={`clipboard-overlay-item ${isSelected ? "selected" : ""}`}
      onClick={onSelect}
      onDoubleClick={onCopy}
    >
      <div className="clipboard-overlay-item-index">{index + 1}</div>
      <div className="clipboard-overlay-item-content">
        <p className="clipboard-overlay-item-text">
          {getClipboardItemLabel(t, item)}
        </p>
        <div className="clipboard-overlay-item-meta">
          <span>{formatClipboardRelativeTime(item.created_at, i18n.language)}</span>
          {item.is_favorite && (
            <Star className="w-3 h-3 text-logo-primary" fill="currentColor" />
          )}
          {item.is_pinned && (
            <Pin className="w-3 h-3 text-logo-primary" fill="currentColor" />
          )}
        </div>
      </div>
      <div className="clipboard-overlay-item-actions">
        {isCopied ? (
          <Check className="w-4 h-4 text-green-500" />
        ) : (
          <Copy className="w-4 h-4 text-text/30" />
        )}
      </div>
    </div>
  );
};

export default ClipboardOverlay;
