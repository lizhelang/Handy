import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import {
  Clipboard,
  Copy,
  File,
  Image,
  Mic,
  Pin,
  Search,
  Star,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { ConfirmHistoryTerm } from "./ConfirmHistoryTerm";
import { useUnifiedHistoryStore } from "@/stores/unifiedHistoryStore";
import {
  getOutputAttempt,
  outputNeedsAcknowledgement,
  useUnifiedOutputStore,
} from "@/stores/unifiedOutputStore";
import type {
  UnifiedHistoryItem,
  UnifiedHistoryProps,
  UnifiedHistorySource,
  UnifiedHistoryContentType,
  UnifiedHistoryActionResult,
  UnifiedHistoryPatch,
} from "@/lib/types/unifiedHistory";

const buttonClass =
  "rounded-md border border-text/15 px-3 py-1.5 text-sm hover:bg-text/5 disabled:opacity-40 disabled:cursor-not-allowed";
const inputClass =
  "min-w-0 rounded-md border border-text/15 bg-transparent px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-logo-primary";

function fileNames(item: UnifiedHistoryItem): string[] {
  if (item.content_type !== "files" || !item.text) return [];
  try {
    const paths: unknown = JSON.parse(item.text);
    return Array.isArray(paths)
      ? paths
          .filter((path): path is string => typeof path === "string")
          .map((path) => path.split(/[\\/]/).filter(Boolean).pop() || path)
      : [];
  } catch {
    return [];
  }
}

function ItemIcon({ item }: { item: UnifiedHistoryItem }) {
  const Icon =
    item.content_type === "files"
      ? File
      : item.content_type === "image"
        ? Image
        : item.source_kind === "voice"
          ? Mic
          : Clipboard;
  return (
    <Icon size={17} aria-hidden="true" className="shrink-0 text-text/50" />
  );
}

function Attachment({
  item,
  resolveAsset,
}: Pick<UnifiedHistoryProps, "resolveAsset"> & { item: UnifiedHistoryItem }) {
  const { t } = useTranslation();
  const [url, setUrl] = useState<string | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  useEffect(() => {
    let disposed = false;
    let resolved: string | null = null;
    setUrl(null);
    setUnavailable(false);
    if (
      item.asset_ref &&
      (item.source_kind === "voice" || item.content_type === "image")
    ) {
      void resolveAsset(item)
        .then((value) => {
          resolved = value;
          if (disposed) {
            if (value?.startsWith("blob:")) URL.revokeObjectURL(value);
            return;
          }
          setUrl(value);
          setUnavailable(!value);
        })
        .catch(() => {
          if (!disposed) setUnavailable(true);
        });
    }
    return () => {
      disposed = true;
      if (resolved?.startsWith("blob:")) URL.revokeObjectURL(resolved);
    };
  }, [item, resolveAsset]);
  if (!item.asset_ref) return null;
  if (unavailable)
    return (
      <p role="status" className="text-sm text-text/60">
        {t("unifiedHistory.assetUnavailable")}
      </p>
    );
  if (!url) return null;
  return item.content_type === "image" ? (
    <img
      src={url}
      alt={t("unifiedHistory.imagePreview")}
      onError={() => setUnavailable(true)}
      className="max-h-64 max-w-full rounded-md object-contain"
    />
  ) : (
    <audio
      controls
      src={url}
      aria-label={t("unifiedHistory.recording")}
      onError={() => setUnavailable(true)}
      className="w-full"
    />
  );
}

export function UnifiedHistory({
  onCopy,
  onInsert,
  onUpdate,
  resolveAsset,
  onRetranscribe,
  onOpenRecordings,
  onCheckOutputReceipt,
}: UnifiedHistoryProps & {
  onCheckOutputReceipt?: (
    operationId: string,
  ) => Promise<UnifiedHistoryActionResult | null>;
}) {
  const { t, i18n } = useTranslation();
  const state = useUnifiedHistoryStore();
  const [localBusy, setBusy] = useState(false);
  const outputs = useUnifiedOutputStore();
  const busy =
    localBusy ||
    Object.values(outputs.attempts).some(
      (attempt) => attempt.status === "inflight",
    );
  const busyRef = useRef(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [editingItem, setEditingItem] = useState<UnifiedHistoryItem | null>(
    null,
  );
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const selected = state.items.find(
    (item) => item.item_id === state.selectedId,
  );
  const insertAttempt = selected
    ? getOutputAttempt(selected.item_id, "insert")
    : undefined;
  const copyAttempt = selected
    ? getOutputAttempt(selected.item_id, "copy")
    : undefined;
  const feedbackMessage =
    feedback ||
    (outputs.storageFailed
      ? "failed"
      : outputNeedsAcknowledgement(insertAttempt)
        ? insertAttempt?.status
        : outputNeedsAcknowledgement(copyAttempt)
          ? copyAttempt?.status
          : null);
  useEffect(() => state.subscribe(), [state.subscribe]);
  useEffect(() => {
    setEditing(false);
    setFeedback(null);
  }, [state.selectedId]);

  const output = async (
    item: UnifiedHistoryItem,
    action: "copy" | "insert",
    receiptOnly = false,
  ) => {
    if (busyRef.current) return;
    const attempt = outputs.begin(
      item.item_id,
      item.revision,
      action,
      receiptOnly,
    );
    if (!attempt) return;
    setFeedback(null);
    try {
      const result: UnifiedHistoryActionResult = receiptOnly
        ? ((await onCheckOutputReceipt?.(attempt.operationId)) ?? {
            status: "uncertain",
          })
        : await (action === "copy" ? onCopy(item) : onInsert(item));
      outputs.finish(attempt, result.status);
      setFeedback(
        result.status === "confirmed"
          ? action === "copy"
            ? "copied"
            : "inserted"
          : result.status,
      );
    } catch {
      outputs.finish(attempt, "uncertain");
      setFeedback("uncertain");
    }
  };
  const update = async (
    item: UnifiedHistoryItem,
    patch: UnifiedHistoryPatch,
  ) => {
    if (busyRef.current || busy) return;
    busyRef.current = true;
    setBusy(true);
    setFeedback(null);
    try {
      await onUpdate(item, patch);
      setEditing(false);
      await state.load();
      setFeedback("updated");
    } catch {
      setFeedback("updateFailed");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const recordingAction = async (
    action: () => Promise<void>,
    reload = false,
  ) => {
    if (busyRef.current || busy) return;
    busyRef.current = true;
    setBusy(true);
    setFeedback(null);
    try {
      await action();
      if (reload) {
        await state.load();
        setFeedback("updated");
      }
    } catch {
      setFeedback("updateFailed");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };
  const handleKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (
      event.repeat &&
      (event.key === "Enter" ||
        (event.key === " " && (event.target as HTMLElement).closest("button")))
    ) {
      event.preventDefault();
      return;
    }
    if (
      event.nativeEvent.isComposing ||
      event.keyCode === 229 ||
      event.repeat ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey
    )
      return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (editing) setEditing(false);
      else state.closePreview();
      return;
    }
    const target = event.target as HTMLElement;
    if (
      target.closest(
        "textarea, select, button, [contenteditable=true], [data-history-editor]",
      )
    )
      return;
    if (event.key === "Enter" && selected) {
      event.preventDefault();
      void output(selected, "insert");
    }
    if (target.closest("input")) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const index = state.items.findIndex(
        (item) => item.item_id === state.selectedId,
      );
      const next =
        index < 0
          ? 0
          : Math.max(
              0,
              Math.min(
                state.items.length - 1,
                index + (event.key === "ArrowDown" ? 1 : -1),
              ),
            );
      if (state.items[next]) state.select(state.items[next].item_id);
    }
  };

  return (
    <section
      aria-label={t("unifiedHistory.heading")}
      className="flex h-full min-h-0 flex-col gap-4 text-text"
      onKeyDown={handleKeys}
    >
      <header className="flex items-center justify-between gap-3">
        <h1 className="text-lg font-semibold">{t("unifiedHistory.heading")}</h1>
        {onOpenRecordings && (
          <button
            className={buttonClass}
            disabled={busy}
            onClick={() => void recordingAction(onOpenRecordings)}
          >
            {t("settings.history.openFolder")}
          </button>
        )}
        <button
          className={buttonClass}
          disabled={state.loading}
          onClick={() => void state.refresh()}
        >
          {t("unifiedHistory.refresh")}
        </button>
      </header>
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative min-w-40 flex-1">
          <Search
            size={16}
            aria-hidden="true"
            className="absolute left-3 top-3 text-text/45"
          />
          <input
            type="search"
            aria-label={t("unifiedHistory.search")}
            placeholder={t("unifiedHistory.search")}
            value={state.search}
            onChange={(event) =>
              state.setFilters({ search: event.target.value })
            }
            className={`${inputClass} w-full pl-9`}
          />
        </label>
        <select
          aria-label={t("unifiedHistory.sourceFilter")}
          className={inputClass}
          value={state.source}
          onChange={(event) =>
            state.setFilters({
              source: event.target.value as UnifiedHistorySource,
            })
          }
        >
          {(["all", "voice", "clipboard"] as const).map((source) => (
            <option key={source} value={source}>
              {t(`unifiedHistory.sources.${source}`)}
            </option>
          ))}
        </select>
        <select
          aria-label={t("unifiedHistory.typeFilter")}
          className={inputClass}
          value={state.contentType}
          onChange={(event) =>
            state.setFilters({
              contentType: event.target.value as UnifiedHistoryContentType,
            })
          }
        >
          {(["all", "text", "image", "files", "html", "rtf"] as const).map(
            (type) => (
              <option key={type} value={type}>
                {t(`unifiedHistory.types.${type}`)}
              </option>
            ),
          )}
        </select>
        <button
          aria-pressed={state.starredOnly}
          className={`${buttonClass} inline-flex items-center gap-1.5 ${state.starredOnly ? "text-accent-text" : ""}`}
          onClick={() => state.setFilters({ starredOnly: !state.starredOnly })}
        >
          <Star size={14} aria-hidden="true" />
          {t("unifiedHistory.favorites")}
        </button>
      </div>
      {(state.error || state.subscriptionError) && (
        <div role="alert" className="text-sm text-red-600">
          {t(`unifiedHistory.errors.${state.error || "subscription"}`)}{" "}
          <button className="underline" onClick={() => void state.refresh()}>
            {t("unifiedHistory.retry")}
          </button>
        </div>
      )}
      {feedbackMessage && (
        <p role="status" aria-live="polite" className="text-sm text-text/70">
          {t(`unifiedHistory.feedback.${feedbackMessage}`)}
        </p>
      )}
      <div className="flex min-h-0 flex-1 flex-wrap gap-4">
        <div
          className="min-w-56 flex-1 overflow-y-auto"
          aria-busy={state.loading}
        >
          <ul
            aria-label={t("unifiedHistory.entries")}
            className="divide-y divide-text/10 rounded-md border border-text/10"
          >
            {state.items.map((item) => (
              <li
                key={item.item_id}
                tabIndex={0}
                aria-current={
                  item.item_id === state.selectedId ? "true" : undefined
                }
                onFocus={(event) => {
                  if (
                    event.target === event.currentTarget &&
                    state.selectedId !== item.item_id
                  )
                    state.select(item.item_id);
                }}
                onClick={() => state.select(item.item_id)}
                onDoubleClick={(event) => {
                  if (!(event.target as HTMLElement).closest("button"))
                    void output(item, "insert");
                }}
                className={`cursor-default p-3 outline-none focus:ring-1 focus:ring-inset focus:ring-logo-primary ${item.item_id === state.selectedId ? "bg-logo-primary/10" : "hover:bg-text/5"}`}
              >
                <div className="flex items-start gap-2.5">
                  <ItemIcon item={item} />
                  <div className="min-w-0 flex-1">
                    <p className="line-clamp-2 whitespace-pre-wrap break-words text-sm">
                      {item.title ||
                        (item.content_type === "files"
                          ? fileNames(item).join(", ")
                          : item.text) ||
                        t(`unifiedHistory.types.${item.content_type}`)}
                    </p>
                    <p className="mt-1 text-xs text-text/50">
                      {t(`unifiedHistory.sources.${item.source_kind}`)} ·{" "}
                      {new Date(item.created_at_ms).toLocaleString(
                        i18n.language,
                      )}{" "}
                      · {item.source_app || t("unifiedHistory.unknownSource")}
                    </p>
                  </div>
                  {item.pinned && (
                    <Pin size={13} aria-label={t("unifiedHistory.pinned")} />
                  )}
                  <button
                    aria-label={
                      item.starred
                        ? t("unifiedHistory.unfavorite")
                        : t("unifiedHistory.favorite")
                    }
                    aria-pressed={item.starred}
                    disabled={busy}
                    onClick={(event) => {
                      event.stopPropagation();
                      void update(item, { starred: !item.starred });
                    }}
                    className="rounded p-1 text-text/50 hover:text-accent-text"
                  >
                    <Star
                      size={15}
                      fill={item.starred ? "currentColor" : "none"}
                    />
                  </button>
                </div>
              </li>
            ))}
          </ul>
          {state.loading && (
            <p role="status" className="p-4 text-sm text-text/50">
              {t("unifiedHistory.loading")}
            </p>
          )}
          {!state.loading && !state.error && state.items.length === 0 && (
            <p className="p-8 text-center text-sm text-text/50">
              {t("unifiedHistory.empty")}
            </p>
          )}
          {state.hasMore && (
            <button
              className={`${buttonClass} mt-3 w-full`}
              disabled={state.loading}
              onClick={() => void state.load(true)}
            >
              {t("unifiedHistory.loadMore")}
            </button>
          )}
        </div>
        {(!selected || !state.previewOpen) && (
          <div className="min-w-56 flex-1 rounded-md border border-text/10 p-8 text-center text-sm text-text/45">
            {t("unifiedHistory.previewHint")}
          </div>
        )}
        {selected && state.previewOpen && (
          <aside
            aria-label={t("unifiedHistory.preview")}
            className="min-w-56 flex-1 space-y-4 overflow-y-auto rounded-md border border-text/10 p-4"
          >
            <div className="flex items-center justify-between gap-2">
              <h2 className="text-sm font-semibold">
                {t("unifiedHistory.preview")}
              </h2>
              <button
                aria-label={t("unifiedHistory.closePreview")}
                onClick={state.closePreview}
              >
                <X size={16} />
              </button>
            </div>
            <Attachment
              key={`${selected.item_id}:${selected.revision}`}
              item={selected}
              resolveAsset={resolveAsset}
            />
            {editing ? (
              <div data-history-editor className="space-y-2">
                <label className="block text-sm">
                  {t("unifiedHistory.title")}
                  <input
                    aria-label={t("unifiedHistory.title")}
                    className={`${inputClass} mt-1 w-full`}
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                  />
                </label>
                {selected.content_type === "text" && (
                  <label className="block text-sm">
                    {t("unifiedHistory.body")}
                    <textarea
                      aria-label={t("unifiedHistory.body")}
                      className={`${inputClass} mt-1 min-h-32 w-full`}
                      value={text}
                      onChange={(event) => setText(event.target.value)}
                    />
                  </label>
                )}
                <div className="flex gap-2">
                  <button
                    disabled={busy}
                    className={buttonClass}
                    onClick={() =>
                      editingItem &&
                      void update(editingItem, {
                        title: title.trim() || null,
                        ...(editingItem.content_type === "text" &&
                        text !== editingItem.text
                          ? { text }
                          : {}),
                      })
                    }
                  >
                    {t("unifiedHistory.save")}
                  </button>
                  <button
                    className={buttonClass}
                    disabled={busy}
                    onClick={() => setEditing(false)}
                  >
                    {t("unifiedHistory.cancel")}
                  </button>
                </div>
              </div>
            ) : (
              <>
                <h3 className="break-words text-sm font-medium">
                  {selected.title}
                </h3>
                <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed">
                  {selected.content_type === "files"
                    ? fileNames(selected).join("\n")
                    : selected.text}
                </pre>
              </>
            )}
            <div className="flex flex-wrap gap-2">
              {onRetranscribe &&
                selected.source_kind === "voice" &&
                selected.asset_ref && (
                  <button
                    className={buttonClass}
                    disabled={busy}
                    onClick={() =>
                      void recordingAction(() => onRetranscribe(selected), true)
                    }
                  >
                    {t("settings.history.retranscribe")}
                  </button>
                )}
              {(["insert", "copy"] as const).map(
                (action) =>
                  outputNeedsAcknowledgement(
                    getOutputAttempt(selected.item_id, action),
                  ) && (
                    <span key={action} className="contents">
                      {onCheckOutputReceipt && (
                        <button
                          className={buttonClass}
                          disabled={busy || outputs.storageFailed}
                          onClick={() => void output(selected, action, true)}
                        >
                          {t("unifiedHistory.checkOutputReceipt")}
                        </button>
                      )}
                      <button
                        className={buttonClass}
                        disabled={busy}
                        onClick={() => {
                          outputs.acknowledge(selected.item_id, action);
                          setFeedback(null);
                        }}
                      >
                        {t(
                          action === "insert"
                            ? "unifiedHistory.allowAnotherInsert"
                            : "unifiedHistory.allowAnotherCopy",
                        )}
                      </button>
                    </span>
                  ),
              )}
              <button
                className={`${buttonClass} border-logo-primary/40`}
                disabled={
                  busy ||
                  outputs.storageFailed ||
                  outputNeedsAcknowledgement(insertAttempt)
                }
                onClick={() => void output(selected, "insert")}
              >
                {t("unifiedHistory.insert")}
              </button>
              <button
                className={`${buttonClass} inline-flex items-center gap-1.5`}
                disabled={
                  busy ||
                  outputs.storageFailed ||
                  outputNeedsAcknowledgement(copyAttempt)
                }
                onClick={() => void output(selected, "copy")}
              >
                <Copy size={14} aria-hidden="true" />
                {t("unifiedHistory.copy")}
              </button>
              <button
                className={buttonClass}
                disabled={busy}
                onClick={() => {
                  setTitle(selected.title || "");
                  setText(selected.text || "");
                  setEditing(true);
                  setEditingItem(selected);
                }}
              >
                {t("unifiedHistory.edit")}
              </button>
              <button
                className={buttonClass}
                disabled={busy}
                aria-pressed={selected.pinned}
                onClick={() =>
                  void update(selected, { pinned: !selected.pinned })
                }
              >
                {t(
                  selected.pinned
                    ? "unifiedHistory.unpin"
                    : "unifiedHistory.pin",
                )}
              </button>
            </div>
            {selected.content_type === "text" && (
              <ConfirmHistoryTerm
                key={`${selected.item_id}:${selected.revision}`}
                itemId={selected.item_id}
                revision={selected.revision}
              />
            )}
            <details>
              <summary className="cursor-pointer text-sm font-medium">
                {t("unifiedHistory.revisions")}
              </summary>
              {state.revisionsLoading && (
                <p className="mt-2 text-sm">{t("unifiedHistory.loading")}</p>
              )}
              {state.revisionsError && (
                <p role="alert" className="mt-2 text-sm">
                  {t("unifiedHistory.errors.revisions")}{" "}
                  <button
                    className="underline"
                    onClick={() => void state.loadRevisions()}
                  >
                    {t("unifiedHistory.retry")}
                  </button>
                </p>
              )}
              <ol className="mt-3 space-y-3">
                {state.revisions.map((revision) => (
                  <li
                    key={revision.revision}
                    className="border-l-2 border-text/10 pl-3"
                  >
                    <p className="text-xs text-text/50">
                      {t("unifiedHistory.revision", {
                        number: revision.revision,
                      })}
                    </p>
                    <pre className="mt-1 whitespace-pre-wrap break-words font-sans text-sm">
                      {revision.text ?? t("unifiedHistory.attachmentRevision")}
                    </pre>
                  </li>
                ))}
              </ol>
            </details>
          </aside>
        )}
      </div>
    </section>
  );
}
