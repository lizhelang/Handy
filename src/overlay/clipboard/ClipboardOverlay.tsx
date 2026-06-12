import React, { useEffect, useRef, useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlignJustify,
  ArrowLeft,
  CircleHelp,
  FileText,
  Image,
  Info,
  Search,
  Settings,
  Star,
  X,
} from "lucide-react";
import { useClipboardStore } from "@/stores/clipboardStore";
import type {
  ClipboardItem,
  ClipboardSettings,
  ClipboardStats,
} from "@/lib/types/clipboard";
import { getClipboardItemLabel } from "@/components/clipboard/utils";
import "./ClipboardOverlay.css";

const APP_NAME = "HANDY";
type OverlayContentFilter = "all" | "text" | "image" | "file";
type OverlayPanel = "list" | "help" | "about" | "settings";

const cn = (...classes: Array<string | false | null | undefined>) =>
  classes.filter(Boolean).join(" ");

const PinToTopIcon: React.FC = () => (
  <svg viewBox="0 0 1024 1024" aria-hidden="true" focusable="false">
    <path
      d="M512 375.04a42.666667 42.666667 0 0 1 42.666667 42.666667v469.333333a42.666667 42.666667 0 0 1-85.333334 0v-469.333333a42.666667 42.666667 0 0 1 42.666667-42.666667z"
      fill="currentColor"
    />
    <path
      d="M511.829333 359.082667a42.666667 42.666667 0 0 1 27.434667 10.026666l264.277333 222.165334a42.666667 42.666667 0 0 1-54.912 65.322666l-236.842666-199.082666-236.8 199.082666a42.666667 42.666667 0 1 1-54.912-65.322666l264.277333-222.165334a42.666667 42.666667 0 0 1 27.477333-10.026666zM202.581333 94.378667h618.666667a42.666667 42.666667 0 0 1 0 85.333333h-618.666667a42.666667 42.666667 0 0 1 0-85.333333z"
      fill="currentColor"
    />
  </svg>
);

const RecordPinIcon: React.FC = () => (
  <svg viewBox="0 0 1024 1024" aria-hidden="true" focusable="false">
    <path
      d="M672 192l160 160-128 128 64 192-160 160-192-192L256 800l-32-32 160-160L192 416l160-160 192 64z"
      fill="currentColor"
    />
  </svg>
);

const escapeRegExp = (value: string) =>
  value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const normalizeSearchValue = (value: string, caseSensitive: boolean) =>
  caseSensitive ? value : value.toLowerCase();

const matchesWholeWord = (
  value: string,
  query: string,
  caseSensitive: boolean,
) => {
  const flags = caseSensitive ? "u" : "iu";
  const pattern = new RegExp(
    `(^|[^\\p{L}\\p{N}_])${escapeRegExp(query)}(?=$|[^\\p{L}\\p{N}_])`,
    flags,
  );
  return pattern.test(value);
};

const itemMatchesFilter = (
  item: ClipboardItem,
  contentFilter: OverlayContentFilter,
) => {
  if (contentFilter === "all") return true;
  if (contentFilter === "text") {
    return item.content_type === "text" || item.content_type === "richtext";
  }
  return item.content_type === contentFilter;
};

const itemMatchesSearch = (
  item: ClipboardItem,
  query: string,
  caseSensitive: boolean,
  wholeWord: boolean,
) => {
  const trimmedQuery = query.trim();
  if (!trimmedQuery) return true;

  const haystack = [item.content_preview, item.full_text, item.source_app]
    .filter(Boolean)
    .join("\n");

  if (wholeWord) {
    return matchesWholeWord(haystack, trimmedQuery, caseSensitive);
  }

  return normalizeSearchValue(haystack, caseSensitive).includes(
    normalizeSearchValue(trimmedQuery, caseSensitive),
  );
};

const getImageUrl = (path?: string) => {
  if (!path) return null;

  try {
    return convertFileSrc(path);
  } catch {
    return null;
  }
};

const formatClipboardTimestamp = (value: string) => {
  const date = new Date(value);

  if (Number.isNaN(date.getTime())) {
    return value;
  }

  const pad = (part: number) => part.toString().padStart(2, "0");

  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(
    date.getSeconds(),
  )}`;
};

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
  const clearHistory = useClipboardStore((s) => s.clearHistory);
  const settings = useClipboardStore((s) => s.settings);
  const stats = useClipboardStore((s) => s.stats);
  const updateSettings = useClipboardStore((s) => s.updateSettings);

  const [selectedIndex, setSelectedIndex] = useState(0);
  const [copiedId, setCopiedId] = useState<number | null>(null);
  const [contentFilter, setContentFilter] =
    useState<OverlayContentFilter>("all");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [windowPinned, setWindowPinned] = useState(false);
  const [activePanel, setActivePanel] = useState<OverlayPanel>("list");
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!initialized) initialize();
  }, [initialized, initialize]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const orderedItems = itemOrder
    .map((id) => items[id])
    .filter(Boolean)
    .sort((left, right) => {
      if (left.is_pinned === right.is_pinned) return 0;
      return left.is_pinned ? -1 : 1;
    });
  const filteredItems = orderedItems.filter(
    (item) =>
      itemMatchesFilter(item, contentFilter) &&
      (!favoritesOnly || item.is_favorite) &&
      itemMatchesSearch(item, searchQuery, caseSensitive, wholeWord),
  );

  useEffect(() => {
    setSelectedIndex(0);
  }, [
    searchQuery,
    itemOrder.length,
    contentFilter,
    favoritesOnly,
    caseSensitive,
    wholeWord,
  ]);

  const handleStartDrag = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      void getCurrentWindow()
        .startDragging()
        .catch(() => undefined);
    },
    [],
  );

  const handleToggleWindowPinned = useCallback(async () => {
    const nextValue = !windowPinned;
    setWindowPinned(nextValue);
    try {
      await getCurrentWindow().setAlwaysOnTop(nextValue);
    } catch {
      setWindowPinned(!nextValue);
    }
  }, [windowPinned]);

  const handleSetPanel = useCallback((panel: OverlayPanel) => {
    setActivePanel(panel);
    if (panel === "list") {
      requestAnimationFrame(() => searchRef.current?.focus());
    }
  }, []);

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
        case "Escape":
          e.preventDefault();
          if (activePanel !== "list") {
            handleSetPanel("list");
          } else if (searchQuery) {
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
      activePanel,
      handleSetPanel,
    ],
  );

  useEffect(() => {
    const selected = listRef.current?.children[selectedIndex] as HTMLElement;
    selected?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div className="clipboard-overlay" onKeyDown={handleKeyDown}>
      <div
        className="clipboard-overlay-topbar"
        data-tauri-drag-region
        onMouseDown={handleStartDrag}
      >
        <div className="clipboard-overlay-brand">{APP_NAME}</div>
        <div
          className="clipboard-overlay-window-actions"
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            className={cn(
              "clipboard-overlay-icon-button",
              windowPinned && "active",
            )}
            onClick={handleToggleWindowPinned}
            title="Pin to top"
          >
            <PinToTopIcon />
          </button>
          <button
            className={cn(
              "clipboard-overlay-icon-button",
              activePanel === "help" && "active",
            )}
            onClick={() =>
              handleSetPanel(activePanel === "help" ? "list" : "help")
            }
            title="Help"
          >
            <CircleHelp />
          </button>
          <button
            className={cn(
              "clipboard-overlay-icon-button",
              activePanel === "about" && "active",
            )}
            onClick={() =>
              handleSetPanel(activePanel === "about" ? "list" : "about")
            }
            title="About"
          >
            <Info />
          </button>
          <button
            className={cn(
              "clipboard-overlay-icon-button",
              activePanel === "settings" && "active",
            )}
            onClick={() =>
              handleSetPanel(activePanel === "settings" ? "list" : "settings")
            }
            title="Settings"
          >
            <Settings />
          </button>
        </div>
      </div>

      {activePanel === "list" ? (
        <>
          <div className="clipboard-overlay-controls">
            <div className="clipboard-overlay-search-pill">
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
                  <X />
                </button>
              )}
              <div className="clipboard-overlay-search-divider" />
              <button
                className={cn(
                  "clipboard-overlay-text-toggle",
                  caseSensitive && "active",
                )}
                onClick={() => setCaseSensitive((value) => !value)}
                title="Case sensitive"
              >
                {t("settings.clipboard.overlay.caseSensitiveShort")}
              </button>
              <button
                className={cn(
                  "clipboard-overlay-text-toggle",
                  wholeWord && "active",
                )}
                onClick={() => setWholeWord((value) => !value)}
                title="Whole word"
              >
                {t("settings.clipboard.overlay.wholeWordShort")}
              </button>
            </div>

            <div className="clipboard-overlay-filter-pill">
              <button
                className={cn(
                  "clipboard-overlay-tool-button",
                  contentFilter === "text" && "active",
                )}
                onClick={() =>
                  setContentFilter((value) =>
                    value === "text" ? "all" : "text",
                  )
                }
                title={t("settings.clipboard.filterText")}
              >
                <AlignJustify />
              </button>
              <button
                className={cn(
                  "clipboard-overlay-tool-button",
                  contentFilter === "image" && "active",
                )}
                onClick={() =>
                  setContentFilter((value) =>
                    value === "image" ? "all" : "image",
                  )
                }
                title={t("settings.clipboard.filterImage")}
              >
                <Image />
              </button>
              <button
                className={cn(
                  "clipboard-overlay-tool-button",
                  contentFilter === "file" && "active",
                )}
                onClick={() =>
                  setContentFilter((value) =>
                    value === "file" ? "all" : "file",
                  )
                }
                title={t("settings.clipboard.filterFiles")}
              >
                <FileText />
              </button>
            </div>

            <button
              className={cn(
                "clipboard-overlay-favorite-filter",
                favoritesOnly && "active",
              )}
              onClick={() => setFavoritesOnly((value) => !value)}
              title={t("settings.clipboard.toggleFavorite")}
            >
              <Star />
            </button>
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
                  onDelete={() => deleteItem(item.id)}
                  onSelect={() => setSelectedIndex(index)}
                  index={index}
                />
              ))
            )}
          </div>

          <div className="clipboard-overlay-bottom-fade" />
        </>
      ) : (
        <OverlayPanelView
          panel={activePanel}
          stats={stats}
          settings={settings}
          onBack={() => handleSetPanel("list")}
          onClearHistory={clearHistory}
          onUpdateMaxRecords={(maxRecords) =>
            updateSettings({ max_records: maxRecords })
          }
          onUpdateConfirmMode={(confirmMode) =>
            updateSettings({ confirm_mode: confirmMode })
          }
        />
      )}
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
  onDelete: () => void;
  onSelect: () => void;
  index: number;
}

interface OverlayPanelViewProps {
  panel: Exclude<OverlayPanel, "list">;
  stats: ClipboardStats | null;
  settings: ClipboardSettings | null;
  onBack: () => void;
  onClearHistory: (keepPinned: boolean) => Promise<void>;
  onUpdateMaxRecords: (maxRecords: number) => Promise<void>;
  onUpdateConfirmMode: (
    confirmMode: ClipboardSettings["confirm_mode"],
  ) => Promise<void>;
}

const OverlayPanelView: React.FC<OverlayPanelViewProps> = ({
  panel,
  stats,
  settings,
  onBack,
  onClearHistory,
  onUpdateMaxRecords,
  onUpdateConfirmMode,
}) => {
  const { t } = useTranslation();
  const [maxRecordsDraft, setMaxRecordsDraft] = useState(
    settings?.max_records ?? 500,
  );

  useEffect(() => {
    setMaxRecordsDraft(settings?.max_records ?? 500);
  }, [settings?.max_records]);

  return (
    <div className="clipboard-overlay-panel">
      <div className="clipboard-overlay-panel-titlebar">
        <button className="clipboard-overlay-panel-back" onClick={onBack}>
          <ArrowLeft />
        </button>
        <span>
          {panel === "settings"
            ? t("settings.clipboard.overlay.panelSettings")
            : panel === "help"
              ? t("settings.clipboard.overlay.panelHelp")
              : t("settings.clipboard.overlay.panelAbout")}
        </span>
      </div>

      {panel === "settings" && (
        <div className="clipboard-overlay-panel-stack">
          <label className="clipboard-overlay-setting-row">
            <span>{t("settings.clipboard.settings.maxRecords")}</span>
            <input
              type="number"
              min={20}
              max={5000}
              value={maxRecordsDraft}
              onChange={(event) =>
                setMaxRecordsDraft(Number(event.target.value))
              }
              onBlur={() =>
                onUpdateMaxRecords(
                  Math.max(20, Math.min(5000, maxRecordsDraft)),
                )
              }
            />
          </label>

          <div className="clipboard-overlay-setting-row">
            <span>{t("settings.clipboard.settings.confirmMode")}</span>
            <div className="clipboard-overlay-segment">
              <button
                className={cn(settings?.confirm_mode !== "paste" && "active")}
                onClick={() => onUpdateConfirmMode("copy")}
              >
                {t("settings.clipboard.settings.confirmModeCopy")}
              </button>
              <button
                className={cn(settings?.confirm_mode === "paste" && "active")}
                onClick={() => onUpdateConfirmMode("paste")}
              >
                {t("settings.clipboard.settings.confirmModePaste")}
              </button>
            </div>
          </div>

          <div className="clipboard-overlay-stats-grid">
            <span>{stats?.total_items ?? 0}</span>
            <span>{stats?.favorites_count ?? 0}</span>
            <span>{stats?.pinned_count ?? 0}</span>
            <small>{t("settings.clipboard.overlay.statTotal")}</small>
            <small>{t("settings.clipboard.overlay.statFavorites")}</small>
            <small>{t("settings.clipboard.overlay.statPinned")}</small>
          </div>

          <div className="clipboard-overlay-danger-actions">
            <button onClick={() => onClearHistory(true)}>
              {t("settings.clipboard.overlay.clearUnpinned")}
            </button>
            <button onClick={() => onClearHistory(false)}>
              {t("settings.clipboard.overlay.clearAll")}
            </button>
          </div>
        </div>
      )}

      {panel === "help" && (
        <div className="clipboard-overlay-panel-stack">
          <div className="clipboard-overlay-help-line">
            <kbd>↑</kbd>
            <kbd>↓</kbd>
            <span>{t("settings.clipboard.overlay.helpMove")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>{t("settings.clipboard.overlay.keyEnter")}</kbd>
            <span>{t("settings.clipboard.overlay.helpCopy")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>F</kbd>
            <span>{t("settings.clipboard.overlay.helpFavorite")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>D</kbd>
            <span>{t("settings.clipboard.overlay.helpDelete")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>{t("settings.clipboard.overlay.keyEscape")}</kbd>
            <span>{t("settings.clipboard.overlay.helpEscape")}</span>
          </div>
        </div>
      )}

      {panel === "about" && (
        <div className="clipboard-overlay-panel-stack">
          <p className="clipboard-overlay-about-copy">
            {t("settings.clipboard.overlay.aboutCopy")}
          </p>
          <div className="clipboard-overlay-stats-grid">
            <span>{stats?.total_items ?? 0}</span>
            <span>{settings?.max_records ?? 0}</span>
            <span>{settings?.hotkey ?? "-"}</span>
            <small>{t("settings.clipboard.overlay.statItems")}</small>
            <small>{t("settings.clipboard.overlay.statLimit")}</small>
            <small>{t("settings.clipboard.overlay.statHotkey")}</small>
          </div>
        </div>
      )}
    </div>
  );
};

const OverlayItem: React.FC<OverlayItemProps> = ({
  item,
  isSelected,
  isCopied,
  onCopy,
  onToggleFavorite,
  onTogglePin,
  onDelete,
  onSelect,
  index,
}) => {
  const { t } = useTranslation();
  const [imageError, setImageError] = useState(false);
  const itemText = item.full_text || getClipboardItemLabel(t, item);
  const imageUrl =
    item.content_type === "image" && !imageError
      ? getImageUrl(item.image_path)
      : null;
  const isLongItem = itemText.length > 56;

  return (
    <div
      className={`clipboard-overlay-item ${isSelected ? "selected" : ""} ${
        isCopied ? "copied" : ""
      } ${isLongItem ? "long" : ""}`}
      onClick={onSelect}
      onDoubleClick={onCopy}
    >
      {imageUrl ? (
        <div className="clipboard-overlay-image-preview">
          <img
            src={imageUrl}
            alt={getClipboardItemLabel(t, item)}
            onError={() => setImageError(true)}
          />
        </div>
      ) : null}
      <div className="clipboard-overlay-item-content">
        <p className="clipboard-overlay-item-text">{itemText}</p>
        <div className="clipboard-overlay-item-meta">
          <span className="clipboard-overlay-item-index">{index + 1}</span>
          <span>{formatClipboardTimestamp(item.created_at)}</span>
        </div>
      </div>
      <div className="clipboard-overlay-item-actions">
        <div className="clipboard-overlay-item-action-row">
          <button
            className={`clipboard-overlay-star-button ${
              item.is_favorite ? "favorited" : ""
            }`}
            onClick={(event) => {
              event.stopPropagation();
              onToggleFavorite();
            }}
          >
            {item.is_favorite ? "★" : "☆"}
          </button>
          <button
            className={`clipboard-overlay-pin-button ${
              item.is_pinned ? "pinned" : ""
            }`}
            onClick={(event) => {
              event.stopPropagation();
              onTogglePin();
            }}
          >
            <RecordPinIcon />
          </button>
        </div>
        <button
          className="clipboard-overlay-delete-button"
          onClick={(event) => {
            event.stopPropagation();
            onDelete();
          }}
        >
          <X />
        </button>
      </div>
    </div>
  );
};

export default ClipboardOverlay;
