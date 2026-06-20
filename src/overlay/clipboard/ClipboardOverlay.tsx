import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlignJustify,
  ArrowLeft,
  CircleHelp,
  FileText,
  Image,
  Info,
  Pencil,
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
  ClipboardContentTypeFilter,
} from "@/lib/types/clipboard";
import { getClipboardItemLabel } from "@/components/clipboard/utils";
import "./ClipboardOverlay.css";

const APP_NAME = "HANDY";
type OverlayContentFilter = "all" | "text" | "image" | "file";
type OverlayPanel = "list" | "help" | "about" | "settings";
const COPY_FEEDBACK_TIMEOUT_MS = 1500;

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

const getDefaultItemTitle = (value: string) =>
  Array.from(value.replace(/\s+/g, " ").trim()).slice(0, 6).join("");

const itemMatchesSearch = (
  item: ClipboardItem,
  query: string,
  caseSensitive: boolean,
  wholeWord: boolean,
) => {
  const trimmedQuery = query.trim();
  if (!trimmedQuery) return true;

  const haystack = [
    item.title,
    item.content_preview,
    item.full_text,
    item.source_app,
  ]
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
  const updateTitle = useClipboardStore((s) => s.updateTitle);
  const deleteItem = useClipboardStore((s) => s.deleteItem);
  const clearHistory = useClipboardStore((s) => s.clearHistory);
  const settings = useClipboardStore((s) => s.settings);
  const stats = useClipboardStore((s) => s.stats);
  const updateSettings = useClipboardStore((s) => s.updateSettings);
  const loadItems = useClipboardStore((s) => s.loadItems);
  const loadFavorites = useClipboardStore((s) => s.loadFavorites);
  const loadMore = useClipboardStore((s) => s.loadMore);
  const hasMore = useClipboardStore((s) => s.hasMore);
  const isLoading = useClipboardStore((s) => s.isLoading);
  const isSearching = useClipboardStore((s) => s.isSearching);

  const [selectedItemId, setSelectedItemId] = useState<number | null>(null);
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
  const copyInFlightRef = useRef<Set<number>>(new Set());
  const copyFeedbackTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    if (!initialized) initialize();
  }, [initialized, initialize]);

  useEffect(() => {
    if (!initialized) return;

    const overlayContentType = contentFilter as ClipboardContentTypeFilter;
    if (searchQuery.trim()) {
      void search(searchQuery, overlayContentType);
      return;
    }
    if (favoritesOnly) {
      void loadFavorites(overlayContentType);
    } else {
      void loadItems(true, {
        contentType: overlayContentType,
        favoriteOnly: false,
      });
    }
  }, [
    contentFilter,
    favoritesOnly,
    initialized,
    loadFavorites,
    loadItems,
    search,
    searchQuery,
  ]);

  useEffect(() => {
    searchRef.current?.focus();
  }, []);

  const filteredItems = useMemo(() => {
    const orderedItems = itemOrder
      .map((id) => items[id])
      .filter(Boolean)
      .sort((left, right) => {
        if (left.is_pinned === right.is_pinned) return 0;
        return left.is_pinned ? -1 : 1;
      });

    return orderedItems.filter(
      (item) =>
        itemMatchesFilter(item, contentFilter) &&
        (!favoritesOnly || item.is_favorite) &&
        itemMatchesSearch(item, searchQuery, caseSensitive, wholeWord),
    );
  }, [
    caseSensitive,
    contentFilter,
    favoritesOnly,
    itemOrder,
    items,
    searchQuery,
    wholeWord,
  ]);

  const filteredItemIdKey = filteredItems.map((item) => item.id).join(",");
  const selectedIndex =
    selectedItemId === null
      ? -1
      : filteredItems.findIndex((item) => item.id === selectedItemId);
  const selectedItem =
    selectedIndex >= 0 ? filteredItems[selectedIndex] : filteredItems[0];

  useEffect(() => {
    setSelectedItemId((currentId) => {
      if (filteredItems.length === 0) return null;
      if (
        currentId !== null &&
        filteredItems.some((item) => item.id === currentId)
      ) {
        return currentId;
      }
      return filteredItems[0].id;
    });
  }, [filteredItemIdKey, filteredItems]);

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

  const handleListScroll = useCallback(() => {
    const list = listRef.current;
    if (!list || !hasMore || isLoading || isSearching) return;

    const remainingScroll =
      list.scrollHeight - list.scrollTop - list.clientHeight;

    if (remainingScroll < 160) {
      void loadMore();
    }
  }, [hasMore, isLoading, isSearching, loadMore]);

  const showCopyFeedback = useCallback((id: number) => {
    if (copyFeedbackTimeoutRef.current !== null) {
      window.clearTimeout(copyFeedbackTimeoutRef.current);
    }

    setCopiedId(id);
    copyFeedbackTimeoutRef.current = window.setTimeout(() => {
      setCopiedId(null);
      copyFeedbackTimeoutRef.current = null;
    }, COPY_FEEDBACK_TIMEOUT_MS);
  }, []);

  useEffect(
    () => () => {
      if (copyFeedbackTimeoutRef.current !== null) {
        window.clearTimeout(copyFeedbackTimeoutRef.current);
      }
    },
    [],
  );

  const copyOverlayItem = useCallback(async (item: ClipboardItem) => {
    await invoke("copy_clipboard_to_system", { id: item.id });
  }, []);

  const handleCopy = useCallback(
    async (item: ClipboardItem) => {
      if (copyInFlightRef.current.has(item.id)) return;

      copyInFlightRef.current.add(item.id);
      showCopyFeedback(item.id);
      try {
        await copyOverlayItem(item);
      } catch {
        setCopiedId((currentId) => (currentId === item.id ? null : currentId));
      } finally {
        copyInFlightRef.current.delete(item.id);
      }
    },
    [copyOverlayItem, showCopyFeedback],
  );

  const hideAfterConfirm = useCallback(() => {
    if (windowPinned) return;

    void getCurrentWindow()
      .hide()
      .catch(() => undefined);
  }, [windowPinned]);

  const handleConfirmItem = useCallback(
    (item: ClipboardItem) => {
      setSelectedItemId(item.id);
      void handleCopy(item);
      hideAfterConfirm();
    },
    [handleCopy, hideAfterConfirm],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
        case "j":
          e.preventDefault();
          setSelectedItemId((currentId) => {
            if (filteredItems.length === 0) return null;
            const currentIndex =
              currentId === null
                ? -1
                : filteredItems.findIndex((item) => item.id === currentId);
            const nextIndex = Math.min(
              (currentIndex >= 0 ? currentIndex : 0) + 1,
              filteredItems.length - 1,
            );
            return filteredItems[nextIndex].id;
          });
          break;
        case "ArrowUp":
        case "k":
          e.preventDefault();
          setSelectedItemId((currentId) => {
            if (filteredItems.length === 0) return null;
            const currentIndex =
              currentId === null
                ? 0
                : filteredItems.findIndex((item) => item.id === currentId);
            const nextIndex = Math.max(
              currentIndex >= 0 ? currentIndex - 1 : 0,
              0,
            );
            return filteredItems[nextIndex].id;
          });
          break;
        case "Enter":
          e.preventDefault();
          if (selectedItem) {
            handleConfirmItem(selectedItem);
          }
          break;
        case "f":
          if (
            !e.metaKey &&
            !e.ctrlKey &&
            document.activeElement !== searchRef.current
          ) {
            e.preventDefault();
            if (selectedItem) {
              toggleFavorite(selectedItem.id);
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
            if (selectedItem) {
              deleteItem(selectedItem.id);
            }
          }
          break;
        case "Escape":
          e.preventDefault();
          if (activePanel !== "list") {
            handleSetPanel("list");
          } else if (searchQuery) {
            search("", contentFilter as ClipboardContentTypeFilter);
          }
          break;
      }
    },
    [
      filteredItems,
      selectedItem,
      handleConfirmItem,
      toggleFavorite,
      togglePin,
      deleteItem,
      search,
      searchQuery,
      activePanel,
      handleSetPanel,
      contentFilter,
    ],
  );

  useEffect(() => {
    if (selectedItemId === null) return;

    const selected = listRef.current?.querySelector<HTMLElement>(
      `[data-clipboard-item-id="${selectedItemId}"]`,
    );
    selected?.scrollIntoView({ block: "nearest" });
  }, [selectedItemId, filteredItemIdKey]);

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
            title={t("settings.clipboard.overlay.pinToTop")}
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
            title={t("settings.clipboard.overlay.panelHelp")}
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
            title={t("settings.clipboard.overlay.panelAbout")}
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
            title={t("settings.clipboard.overlay.panelSettings")}
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
                onChange={(e) =>
                  search(
                    e.target.value,
                    contentFilter as ClipboardContentTypeFilter,
                  )
                }
              />
              {searchQuery && (
                <button
                  className="clipboard-overlay-clear"
                  onClick={() =>
                    search("", contentFilter as ClipboardContentTypeFilter)
                  }
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
                title={t("settings.clipboard.overlay.caseSensitive")}
              >
                {t("settings.clipboard.overlay.caseSensitiveShort")}
              </button>
              <button
                className={cn(
                  "clipboard-overlay-text-toggle",
                  wholeWord && "active",
                )}
                onClick={() => setWholeWord((value) => !value)}
                title={t("settings.clipboard.overlay.wholeWord")}
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

          <div
            className="clipboard-overlay-list"
            ref={listRef}
            onScroll={handleListScroll}
          >
            {filteredItems.length === 0 ? (
              <div className="clipboard-overlay-empty">
                {t("settings.clipboard.emptyTitle")}
              </div>
            ) : (
              filteredItems.map((item, index) => (
                <OverlayItem
                  key={item.id}
                  item={item}
                  isSelected={item.id === selectedItemId}
                  isCopied={copiedId === item.id}
                  onToggleFavorite={() => toggleFavorite(item.id)}
                  onTogglePin={() => togglePin(item.id)}
                  onUpdateTitle={(title) => updateTitle(item.id, title)}
                  onDelete={() => deleteItem(item.id)}
                  onConfirm={() => handleConfirmItem(item)}
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
        />
      )}
    </div>
  );
};

interface OverlayItemProps {
  item: ClipboardItem;
  isSelected: boolean;
  isCopied: boolean;
  onToggleFavorite: () => void;
  onTogglePin: () => void;
  onUpdateTitle: (title: string) => Promise<void>;
  onDelete: () => void;
  onConfirm: () => void;
  index: number;
}

interface OverlayPanelViewProps {
  panel: Exclude<OverlayPanel, "list">;
  stats: ClipboardStats | null;
  settings: ClipboardSettings | null;
  onBack: () => void;
  onClearHistory: (keepPinned: boolean) => Promise<void>;
  onUpdateMaxRecords: (maxRecords: number) => Promise<void>;
}

const OverlayPanelView: React.FC<OverlayPanelViewProps> = ({
  panel,
  stats,
  settings,
  onBack,
  onClearHistory,
  onUpdateMaxRecords,
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
          <div className="clipboard-overlay-stats-grid two-column">
            <span>{stats?.total_items ?? 0}</span>
            <span>{settings?.max_records ?? 0}</span>
            <small>{t("settings.clipboard.overlay.statItems")}</small>
            <small>{t("settings.clipboard.overlay.statLimit")}</small>
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
  onToggleFavorite,
  onTogglePin,
  onUpdateTitle,
  onDelete,
  onConfirm,
  index,
}) => {
  const { t } = useTranslation();
  const [imageError, setImageError] = useState(false);
  const [isEditingTitle, setIsEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(item.title ?? "");
  const itemText = item.full_text || getClipboardItemLabel(t, item);
  const customTitle = item.title?.trim();
  const displayTitle = item.is_favorite
    ? customTitle || getDefaultItemTitle(itemText)
    : "";
  const imageUrl =
    item.content_type === "image" && !imageError
      ? getImageUrl(item.image_path)
      : null;
  const isLongItem = itemText.length > 56;
  const shouldShowTitleLine = isEditingTitle || item.is_favorite;

  useEffect(() => {
    if (!isEditingTitle) {
      setTitleDraft(item.title ?? "");
    }
  }, [isEditingTitle, item.title]);

  const saveTitle = useCallback(() => {
    const nextTitle = titleDraft.trim();
    setIsEditingTitle(false);

    if (nextTitle === (item.title ?? "")) {
      return;
    }

    void onUpdateTitle(nextTitle);
  }, [item.title, onUpdateTitle, titleDraft]);

  const handleTitleButtonClick = useCallback(() => {
    if (isEditingTitle) {
      saveTitle();
      return;
    }

    setTitleDraft(item.title ?? "");
    setIsEditingTitle(true);
  }, [isEditingTitle, item.title, saveTitle]);

  return (
    <div
      className={`clipboard-overlay-item ${isSelected ? "selected" : ""} ${
        isCopied ? "copied" : ""
      } ${isLongItem ? "long" : ""}`}
      data-testid="clipboard-overlay-item"
      data-clipboard-item-id={item.id}
      onClick={(event) => {
        if (event.button !== 0) return;
        onConfirm();
      }}
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
        {shouldShowTitleLine ? (
          isEditingTitle ? (
            <input
              className="clipboard-overlay-title-input"
              value={titleDraft}
              autoFocus
              maxLength={80}
              placeholder={t("settings.clipboard.overlay.titlePlaceholder")}
              onChange={(event) => setTitleDraft(event.target.value)}
              onBlur={saveTitle}
              onPointerDown={(event) => event.stopPropagation()}
              onClick={(event) => event.stopPropagation()}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === "Enter") {
                  event.preventDefault();
                  saveTitle();
                } else if (event.key === "Escape") {
                  event.preventDefault();
                  setTitleDraft(item.title ?? "");
                  setIsEditingTitle(false);
                }
              }}
            />
          ) : displayTitle ? (
            <div className="clipboard-overlay-item-title">{displayTitle}</div>
          ) : (
            <button
              className="clipboard-overlay-item-title empty"
              onPointerDown={(event) => event.stopPropagation()}
              onClick={(event) => {
                event.stopPropagation();
                setIsEditingTitle(true);
              }}
            >
              <Pencil />
              <span>{t("settings.clipboard.overlay.titlePlaceholder")}</span>
            </button>
          )
        ) : null}
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
            onPointerDown={(event) => event.stopPropagation()}
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
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              onTogglePin();
            }}
          >
            <RecordPinIcon />
          </button>
          <button
            className={`clipboard-overlay-title-button ${
              isEditingTitle ? "editing" : ""
            }`}
            title={t("settings.clipboard.overlay.editTitle")}
            onPointerDown={(event) => {
              event.preventDefault();
              event.stopPropagation();
              handleTitleButtonClick();
            }}
            onClick={(event) => {
              event.stopPropagation();
            }}
          >
            <Pencil />
          </button>
        </div>
        <button
          className="clipboard-overlay-delete-button"
          onPointerDown={(event) => event.stopPropagation()}
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
