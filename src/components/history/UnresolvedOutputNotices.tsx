import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  acknowledgeOutputNotice,
  listUnresolvedOutputs,
  listenForOutputResults,
  type UnresolvedOutputNotice,
} from "@/lib/unresolvedOutputs";

export function UnresolvedOutputNotices() {
  const { t } = useTranslation();
  const [items, setItems] = useState<UnresolvedOutputNotice[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const request = useRef(0);
  const load = useCallback(async (after: string | null = null) => {
    const id = ++request.current;
    setBusy(true);
    setError(false);
    try {
      const page = await listUnresolvedOutputs(after);
      if (id !== request.current) return;
      setItems((current) =>
        after
          ? [
              ...current,
              ...page.items.filter(
                (item) =>
                  !current.some(
                    (old) => old.operation_id === item.operation_id,
                  ),
              ),
            ]
          : page.items,
      );
      setCursor(page.next_cursor);
    } catch {
      if (id === request.current) setError(true);
    } finally {
      if (id === request.current) setBusy(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      if (!active || refreshTimer !== undefined) return;
      refreshTimer = setTimeout(() => {
        refreshTimer = undefined;
        if (active) void load();
      }, 100);
    };
    const unlisten = listenForOutputResults(refresh);
    void unlisten.then(refresh).catch(refresh);
    return () => {
      active = false;
      clearTimeout(refreshTimer);
      ++request.current;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [load]);

  const dismiss = async (item: UnresolvedOutputNotice) => {
    if (busy) return;
    setBusy(true);
    setError(false);
    try {
      await acknowledgeOutputNotice(item);
      await load();
    } catch {
      setError(true);
      setBusy(false);
    }
  };

  if (!items.length && !error) return null;
  return (
    <section
      className="mb-3 max-h-52 shrink-0 overflow-y-auto rounded-lg border border-mid-gray/30 p-3"
      aria-label={t("unifiedHistory.outputNotices.heading")}
    >
      <details>
        <summary className="cursor-pointer font-medium">
          {t("unifiedHistory.outputNotices.heading")} ({items.length}
          {cursor ? "+" : ""})
        </summary>
        <p className="mb-2 text-xs text-text/60">
          {t("unifiedHistory.outputNotices.historyHelp")}
        </p>
        <p className="mb-2 text-xs text-text/60">
          {t("unifiedHistory.outputNotices.acknowledgeHelp")}
        </p>
        {items.map((item) => (
          <div
            key={item.operation_id}
            className="my-2 flex items-center justify-between gap-3 text-sm"
          >
            <span>{t(`unifiedHistory.feedback.${item.state}`)}</span>
            <button
              type="button"
              className="shrink-0 underline"
              disabled={busy}
              onClick={() => void dismiss(item)}
            >
              {t("unifiedHistory.outputNotices.acknowledge")}
            </button>
          </div>
        ))}
      </details>
      {error && <p role="alert">{t("unifiedHistory.errors.load")}</p>}
      <div className="flex gap-3 text-sm">
        <button type="button" disabled={busy} onClick={() => void load()}>
          {t("unifiedHistory.refresh")}
        </button>
        {cursor && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void load(cursor)}
          >
            {t("unifiedHistory.loadMore")}
          </button>
        )}
      </div>
    </section>
  );
}
