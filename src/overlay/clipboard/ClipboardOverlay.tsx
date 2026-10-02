import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  AlignJustify,
  ArrowLeft,
  CircleHelp,
  FileText,
  Mic,
  Image,
  Info,
  Pencil,
  Search,
  Settings,
  Star,
  X,
} from "lucide-react";
import { commands } from "@/bindings";
import { InputiaMark } from "@/components/icons";
import type { UnifiedOutputResult } from "@/bindings";
import { useUnifiedHistoryStore } from "@/stores/unifiedHistoryStore";
import {
  useUnifiedOutputStore,
  outputNeedsAcknowledgement,
  type OutputAction,
} from "@/stores/unifiedOutputStore";
import {
  useSharedClipboard,
  useHistoryImage,
  type OverlayHistoryItem,
} from "./useSharedClipboard";
import type {
  ClipboardSettings,
  ClipboardStats,
  ClipboardContentTypeFilter,
} from "@/lib/types/clipboard";
import {
  getClipboardFilePaths,
  getClipboardItemBodyText,
  getClipboardItemLabel,
  getClipboardTypeLabel,
} from "@/components/clipboard/utils";
import "./ClipboardOverlay.css";

type OverlayContentFilter = "all" | "text" | "image" | "file";
type OverlayPanel = "list" | "help" | "about" | "settings";
const COPY_FEEDBACK_TIMEOUT_MS = 1500;
const UNLIMITED_CLIPBOARD_RECORDS = 0;
const MIN_LIMITED_CLIPBOARD_RECORDS = 1;
const DEFAULT_LIMITED_CLIPBOARD_RECORDS = 500;

const cn = (...classes: Array<string | false | null | undefined>) =>
  classes.filter(Boolean).join(" ");

const isEditableKeyboardTarget = (target: EventTarget | null) => {
  if (!(target instanceof HTMLElement)) return false;

  if (target.isContentEditable) return true;

  return Boolean(
    target.closest("input, textarea, select, [contenteditable='true']"),
  );
};

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
  item: OverlayHistoryItem,
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
  item: OverlayHistoryItem,
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
  const {
    items: sharedItems,
    settings,
    stats,
    error,
    mutate,
    clearHistory,
    updateSettings,
  } = useSharedClipboard();
  const history = useUnifiedHistoryStore();
  const output = useUnifiedOutputStore();
  const [searchQuery, setSearchQuery] = useState("");
  const search = useCallback(
    (query: string, _filter?: ClipboardContentTypeFilter) =>
      setSearchQuery(query),
    [],
  );
  const toggleFavorite = (id: string) =>
    mutate(id, {
      starred: !sharedItems.find((item) => item.id === id)?.is_favorite,
    });
  const togglePin = (id: string) =>
    mutate(id, {
      pinned: !sharedItems.find((item) => item.id === id)?.is_pinned,
    });
  const updateTitle = (id: string, title: string) =>
    mutate(id, { title: title || null, clear_title: !title });
  const [deleteCandidate, setDeleteCandidate] =
    useState<OverlayHistoryItem | null>(null);
  const [deleting, setDeleting] = useState(false);
  const deleteItem = (id: string) => {
    setPreviewHeld(false);
    setDeleteCandidate(sharedItems.find((item) => item.id === id) ?? null);
  };
  const { hasMore, loading: isLoading } = history;
  const isSearching = isLoading;
  const loadMore = useCallback(
    () => useUnifiedHistoryStore.getState().load(true),
    [],
  );
  const [feedback, setFeedback] = useState<string | null>(null);
  const [selectedItemId, setSelectedItemId] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [contentFilter, setContentFilter] =
    useState<OverlayContentFilter>("all");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [windowPinned, setWindowPinned] = useState(false);
  const [activePanel, setActivePanel] = useState<OverlayPanel>("list");
  const [previewHeld, setPreviewHeld] = useState(false);
  const overlayRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const copyFeedbackTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    history.setFilters({
      source: "all",
      search: searchQuery,
      starredOnly: favoritesOnly,
      contentType:
        contentFilter === "file"
          ? "files"
          : contentFilter === "text"
            ? "all"
            : contentFilter,
    });
  }, [history.setFilters, searchQuery, favoritesOnly, contentFilter]);

  useEffect(() => {
    overlayRef.current?.focus();
  }, []);

  const filteredItems = useMemo(() => {
    const orderedItems = [...sharedItems].sort((left, right) => {
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
    sharedItems,
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
      await invoke("set_clipboard_overlay_pinned", { pinned: nextValue });
    } catch {
      setWindowPinned(!nextValue);
    }
  }, [windowPinned]);

  const handleSetPanel = useCallback((panel: OverlayPanel) => {
    setPreviewHeld(false);
    setActivePanel(panel);
    if (panel === "list") {
      requestAnimationFrame(() => overlayRef.current?.focus());
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

  const showCopyFeedback = useCallback((id: string) => {
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

  const hideAfterConfirm = useCallback(() => {
    if (!windowPinned)
      void invoke("hide_clipboard_overlay").catch(() =>
        setFeedback("unifiedHistory.feedback.failed"),
      );
  }, [windowPinned]);

  const performOutput = useCallback(
    async (
      item: OverlayHistoryItem,
      action: OutputAction,
      plainText = false,
      receiptOnly = false,
    ) => {
      setSelectedItemId(item.id);
      const attempt = useUnifiedOutputStore
        .getState()
        .begin(item.id, item.original.revision, action, receiptOnly);
      if (!attempt) {
        setFeedback("unifiedHistory.feedback.uncertain");
        return;
      }
      setFeedback("unifiedHistory.loading");
      let status:
        | "confirmed"
        | "dispatched"
        | "pending_target"
        | "uncertain"
        | "rejected"
        | "failed" = "uncertain";
      try {
        let receipt: UnifiedOutputResult | null;
        if (receiptOnly) {
          const result = await commands.getUnifiedOutputReceipt(
            attempt.operationId,
          );
          if (result.status === "error") throw new Error(result.error);
          receipt = result.data;
        } else if (plainText) {
          receipt = await invoke<UnifiedOutputResult>(
            "copy_unified_history_item_as_text",
            {
              itemId: item.id,
              expectedRevision: attempt.revision,
              operationId: attempt.operationId,
            },
          );
        } else {
          const result = await (
            action === "copy"
              ? commands.copyUnifiedHistoryItem
              : commands.insertUnifiedHistoryItem
          )(item.id, attempt.revision, attempt.operationId);
          if (result.status === "error") throw new Error(result.error);
          receipt = result.data;
        }
        if (receipt && receipt.operation_id === attempt.operationId) {
          switch (receipt.status) {
            case "confirmed":
            case "dispatched":
            case "pending_target":
            case "uncertain":
            case "rejected":
            case "failed":
              status = receipt.status;
          }
        }
      } catch {
        status = "uncertain";
      }
      useUnifiedOutputStore.getState().finish(attempt, status);
      setFeedback(
        status === "confirmed"
          ? action === "copy"
            ? "unifiedHistory.feedback.copied"
            : "unifiedHistory.feedback.inserted"
          : `unifiedHistory.feedback.${status}`,
      );
      if (status === "confirmed") {
        showCopyFeedback(item.id);
        hideAfterConfirm();
      }
    },
    [hideAfterConfirm, showCopyFeedback],
  );

  const handleConfirmItem = useCallback(
    (item: OverlayHistoryItem) => {
      if (!settings) {
        setFeedback("unifiedHistory.feedback.failed");
        return;
      }
      void performOutput(
        item,
        settings.confirm_mode === "paste" ? "insert" : "copy",
      );
    },
    [performOutput, settings],
  );

  const handleConfirmItemAsPlainText = useCallback(
    (item: OverlayHistoryItem) => {
      void performOutput(item, "copy", item.content_type !== "image");
    },
    [performOutput],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (deleteCandidate) {
        e.preventDefault();
        return;
      }
      const isEditableTarget = isEditableKeyboardTarget(e.target);

      switch (e.key) {
        case "ArrowDown":
        case "j":
          if (isEditableTarget) {
            break;
          }
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
          if (isEditableTarget) {
            break;
          }
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
        case "/":
          if (!e.metaKey && !e.ctrlKey && !e.altKey && !isEditableTarget) {
            e.preventDefault();
            searchRef.current?.focus();
            searchRef.current?.select();
          }
          break;
        case "1":
        case "2":
        case "3":
        case "4":
        case "5":
          if (
            activePanel === "list" &&
            !e.metaKey &&
            !e.ctrlKey &&
            !e.altKey &&
            !isEditableTarget
          ) {
            e.preventDefault();
            const item = filteredItems[Number(e.key) - 1];
            if (item) {
              handleConfirmItem(item);
            }
          }
          break;
        case "Enter":
          if (activePanel !== "list" || isEditableTarget) {
            break;
          }
          e.preventDefault();
          if (selectedItem) {
            if (e.shiftKey) {
              handleConfirmItemAsPlainText(selectedItem);
            } else {
              handleConfirmItem(selectedItem);
            }
          }
          break;
        case "f":
        case "F":
          if (
            activePanel === "list" &&
            !e.metaKey &&
            !e.ctrlKey &&
            !e.altKey &&
            !isEditableTarget
          ) {
            e.preventDefault();
            if (selectedItem) {
              toggleFavorite(selectedItem.id);
            }
          }
          break;
        case "p":
        case "P":
          if (
            activePanel === "list" &&
            !e.metaKey &&
            !e.ctrlKey &&
            !e.altKey &&
            !isEditableTarget
          ) {
            e.preventDefault();
            if (selectedItem) {
              togglePin(selectedItem.id);
            }
          }
          break;
        case "d":
        case "D":
        case "Delete":
          if (
            activePanel === "list" &&
            !e.metaKey &&
            !e.ctrlKey &&
            !e.altKey &&
            !isEditableTarget
          ) {
            e.preventDefault();
            if (selectedItem) {
              deleteItem(selectedItem.id);
            }
          }
          break;
        case " ":
          if (
            activePanel === "list" &&
            !e.metaKey &&
            !e.ctrlKey &&
            !e.altKey &&
            !isEditableTarget &&
            selectedItem
          ) {
            e.preventDefault();
            setPreviewHeld(true);
          }
          break;
        case "Escape":
          e.preventDefault();
          setPreviewHeld(false);
          if (activePanel !== "list") {
            handleSetPanel("list");
          } else if (searchQuery) {
            search("", contentFilter as ClipboardContentTypeFilter);
          } else {
            hideAfterConfirm();
          }
          break;
      }
    },
    [
      filteredItems,
      selectedItem,
      handleConfirmItem,
      handleConfirmItemAsPlainText,
      toggleFavorite,
      togglePin,
      deleteItem,
      deleteCandidate,
      search,
      searchQuery,
      activePanel,
      handleSetPanel,
      hideAfterConfirm,
      contentFilter,
    ],
  );

  const handleKeyUp = useCallback((e: React.KeyboardEvent) => {
    if (e.key === " ") {
      e.preventDefault();
      setPreviewHeld(false);
    }
  }, []);

  useEffect(() => {
    const releasePreview = () => setPreviewHeld(false);
    const releaseHiddenPreview = () => {
      if (document.hidden) releasePreview();
    };

    window.addEventListener("blur", releasePreview);
    document.addEventListener("visibilitychange", releaseHiddenPreview);
    return () => {
      window.removeEventListener("blur", releasePreview);
      document.removeEventListener("visibilitychange", releaseHiddenPreview);
    };
  }, []);

  useEffect(() => {
    if (selectedItemId === null) return;

    const selected = listRef.current?.querySelector<HTMLElement>(
      `[data-clipboard-item-id="${CSS.escape(selectedItemId)}"]`,
    );
    selected?.scrollIntoView({ block: "nearest" });
  }, [selectedItemId, filteredItemIdKey]);

  return (
    <div className="clipboard-overlay-stage">
      <div
        ref={overlayRef}
        className="clipboard-overlay"
        onKeyDown={handleKeyDown}
        onKeyUp={handleKeyUp}
        tabIndex={-1}
      >
        <div
          className="clipboard-overlay-topbar"
          data-tauri-drag-region
          onMouseDown={handleStartDrag}
        >
          <div className="clipboard-overlay-brand flex items-center gap-2">
            <InputiaMark />
            {t("settings.clipboard.title")}
          </div>
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

        {(error || feedback) && (
          <div role="status">{t(error ?? feedback ?? "")}</div>
        )}
        {deleteCandidate && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              zIndex: 50,
              display: "grid",
              placeItems: "center",
              background: "rgba(0,0,0,0.65)",
              padding: 24,
            }}
            onKeyDown={(event) => {
              event.stopPropagation();
              if (event.key === "Escape" && !deleting) {
                setDeleteCandidate(null);
                overlayRef.current?.focus();
              }
              if (event.key === "Tab") {
                const buttons =
                  event.currentTarget.querySelectorAll<HTMLButtonElement>(
                    "button:not(:disabled)",
                  );
                if (!buttons.length) {
                  event.preventDefault();
                  return;
                }
                if (event.shiftKey && document.activeElement === buttons[0]) {
                  event.preventDefault();
                  buttons[buttons.length - 1].focus();
                } else if (
                  !event.shiftKey &&
                  document.activeElement === buttons[buttons.length - 1]
                ) {
                  event.preventDefault();
                  buttons[0].focus();
                }
              }
            }}
          >
            <div
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="delete-record-title"
              aria-describedby="delete-record-description"
              style={{
                background: "var(--color-mid-gray, #282828)",
                color: "var(--color-text, white)",
                borderRadius: 16,
                padding: 20,
              }}
            >
              <strong id="delete-record-title">
                {t("settings.clipboard.overlay.deleteRecordTitle")}
              </strong>
              <p id="delete-record-description">
                {t("settings.clipboard.overlay.deleteRecordDescription")}
              </p>
              <button
                autoFocus
                disabled={deleting}
                onClick={() => {
                  setDeleteCandidate(null);
                  overlayRef.current?.focus();
                }}
              >
                {t("unifiedHistory.cancel")}
              </button>
              <button
                disabled={deleting}
                onClick={async () => {
                  setDeleting(true);
                  await mutate(
                    deleteCandidate.id,
                    undefined,
                    deleteCandidate.original,
                  );
                  setDeleting(false);
                  setDeleteCandidate(null);
                  overlayRef.current?.focus();
                }}
              >
                {t("settings.clipboard.overlay.deleteRecordConfirm")}
              </button>
            </div>
          </div>
        )}
        {activePanel === "list" && selectedItem && (
          <div className="clipboard-overlay-controls">
            {(["copy", "insert"] as const).map((action) => {
              const attempt = Object.values(output.attempts).find(
                (value) =>
                  value.itemId === selectedItem.id && value.action === action,
              );
              return (
                <React.Fragment key={action}>
                  <button
                    disabled={
                      Object.values(output.attempts).some(
                        (value) => value.status === "inflight",
                      ) ||
                      outputNeedsAcknowledgement(attempt) ||
                      output.storageFailed
                    }
                    onClick={() => void performOutput(selectedItem, action)}
                  >
                    {t(`unifiedHistory.${action}`)}
                  </button>
                  {outputNeedsAcknowledgement(attempt) && (
                    <>
                      <button
                        onClick={() =>
                          void performOutput(selectedItem, action, false, true)
                        }
                      >
                        {t("unifiedHistory.checkOutputReceipt")}
                      </button>
                      <button
                        onClick={() => {
                          output.acknowledge(selectedItem.id, action);
                          setFeedback(null);
                        }}
                      >
                        {t(
                          action === "copy"
                            ? "unifiedHistory.allowAnotherCopy"
                            : "unifiedHistory.allowAnotherInsert",
                        )}
                      </button>
                    </>
                  )}
                </React.Fragment>
              );
            })}
          </div>
        )}
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
              {hasMore && (
                <button disabled={isLoading} onClick={() => void loadMore()}>
                  {t("unifiedHistory.loadMore")}
                </button>
              )}
            </div>

            <div className="clipboard-overlay-bottom-fade" />
            {previewHeld && selectedItem ? (
              <OverlayQuickPreview item={selectedItem} />
            ) : null}
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
    </div>
  );
};

const OverlayQuickPreview: React.FC<{ item: OverlayHistoryItem }> = ({
  item,
}) => {
  const { t } = useTranslation();
  const [imageError, setImageError] = useState(false);
  const itemText = getClipboardItemBodyText(item);
  const resolvedImage = useHistoryImage(item);
  const imageUrl = imageError ? null : resolvedImage;
  const title = item.title?.trim() || getClipboardItemLabel(t, item);
  const typeLabel = getClipboardTypeLabel(t, item.content_type);

  return (
    <div aria-live="polite" className="clipboard-overlay-preview">
      <div className="clipboard-overlay-preview-header">
        <span className="clipboard-overlay-preview-type">{typeLabel}</span>
        {title ? (
          <strong className="clipboard-overlay-preview-title">{title}</strong>
        ) : null}
      </div>
      {imageUrl ? (
        <img
          className="clipboard-overlay-preview-image"
          src={imageUrl}
          alt={title || getClipboardItemLabel(t, item)}
          onError={() => setImageError(true)}
        />
      ) : (
        <pre className="clipboard-overlay-preview-text">{itemText}</pre>
      )}
    </div>
  );
};

interface OverlayItemProps {
  item: OverlayHistoryItem;
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
    settings?.max_records ?? UNLIMITED_CLIPBOARD_RECORDS,
  );
  const isUnlimited = maxRecordsDraft === UNLIMITED_CLIPBOARD_RECORDS;

  useEffect(() => {
    setMaxRecordsDraft(settings?.max_records ?? UNLIMITED_CLIPBOARD_RECORDS);
  }, [settings?.max_records]);

  const saveMaxRecords = useCallback(() => {
    const nextMaxRecords = isUnlimited
      ? UNLIMITED_CLIPBOARD_RECORDS
      : Math.max(
          MIN_LIMITED_CLIPBOARD_RECORDS,
          Math.floor(Number.isFinite(maxRecordsDraft) ? maxRecordsDraft : 0),
        );

    setMaxRecordsDraft(nextMaxRecords);
    void onUpdateMaxRecords(nextMaxRecords);
  }, [isUnlimited, maxRecordsDraft, onUpdateMaxRecords]);

  const limitLabel =
    settings?.max_records === UNLIMITED_CLIPBOARD_RECORDS
      ? t("settings.clipboard.overlay.unlimited")
      : (settings?.max_records ?? UNLIMITED_CLIPBOARD_RECORDS);

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
          <div className="clipboard-overlay-setting-row">
            <div className="clipboard-overlay-setting-copy">
              <span>{t("settings.clipboard.settings.maxRecords")}</span>
              <small>
                {t("settings.clipboard.settings.maxRecordsDescription")}
              </small>
            </div>
            <div className="clipboard-overlay-limit-controls">
              <label className="clipboard-overlay-checkbox-row">
                <input
                  type="checkbox"
                  checked={isUnlimited}
                  onChange={(event) => {
                    const nextValue = event.target.checked
                      ? UNLIMITED_CLIPBOARD_RECORDS
                      : Math.max(
                          MIN_LIMITED_CLIPBOARD_RECORDS,
                          settings?.max_records ||
                            DEFAULT_LIMITED_CLIPBOARD_RECORDS,
                        );
                    setMaxRecordsDraft(nextValue);
                    void onUpdateMaxRecords(nextValue);
                  }}
                />
                <span>{t("settings.clipboard.settings.unlimitedRecords")}</span>
              </label>
              <input
                type="number"
                min={MIN_LIMITED_CLIPBOARD_RECORDS}
                value={isUnlimited ? "" : maxRecordsDraft}
                disabled={isUnlimited}
                placeholder={t("settings.clipboard.overlay.unlimited")}
                onChange={(event) =>
                  setMaxRecordsDraft(Number(event.target.value))
                }
                onBlur={saveMaxRecords}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.currentTarget.blur();
                  }
                }}
              />
            </div>
          </div>

          <p>{t("settings.clipboard.overlay.clipboardOnlyScope")}</p>
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
            <kbd>/</kbd>
            <span>{t("settings.clipboard.overlay.helpSearch")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>↑</kbd>
            <kbd>↓</kbd>
            <span>{t("settings.clipboard.overlay.helpMove")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>1-5</kbd>
            <span>{t("settings.clipboard.overlay.helpQuickSelect")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>{t("settings.clipboard.overlay.keyEnter")}</kbd>
            <span>{t("settings.clipboard.overlay.helpCopy")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>{t("settings.clipboard.overlay.keyShiftEnter")}</kbd>
            <span>{t("settings.clipboard.overlay.helpPlainTextCopy")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>F</kbd>
            <span>{t("settings.clipboard.overlay.helpFavorite")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>P</kbd>
            <span>{t("settings.clipboard.overlay.helpPin")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>D</kbd>
            <kbd>{t("settings.clipboard.overlay.keyDelete")}</kbd>
            <span>{t("settings.clipboard.overlay.helpDelete")}</span>
          </div>
          <div className="clipboard-overlay-help-line">
            <kbd>{t("settings.clipboard.overlay.keySpace")}</kbd>
            <span>{t("settings.clipboard.overlay.helpPreview")}</span>
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
            <span>{limitLabel}</span>
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
  const itemText = getClipboardItemBodyText(item);
  const filePaths = getClipboardFilePaths(item);
  const derivedTitle =
    item.content_type === "file" && filePaths.length > 0
      ? getClipboardItemLabel(t, item)
      : "";
  const fileCountLabel =
    item.content_type === "file" && filePaths.length > 1
      ? t("settings.clipboard.overlay.fileCount", { count: filePaths.length })
      : "";
  const customTitle = item.title?.trim();
  const displayTitle =
    customTitle ||
    (item.is_favorite
      ? derivedTitle || getDefaultItemTitle(itemText)
      : derivedTitle);
  const resolvedImage = useHistoryImage(item);
  const imageUrl = imageError ? null : resolvedImage;
  const isLongItem = itemText.length > 56;
  const shouldShowTitleLine =
    isEditingTitle || item.is_favorite || item.content_type === "file";

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
            <div className="clipboard-overlay-item-title">
              <span>{displayTitle}</span>
              {fileCountLabel ? (
                <span className="clipboard-overlay-file-count">
                  {fileCountLabel}
                </span>
              ) : null}
            </div>
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
          {item.original.source_kind === "voice" && (
            <Mic aria-label={t("unifiedHistory.sources.voice")} size={12} />
          )}
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
