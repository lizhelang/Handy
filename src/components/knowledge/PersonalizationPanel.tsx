import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

interface BackfillSummary {
  imported?: number;
  skipped?: number;
  revoked?: number;
  existing?: number;
  tracked_sources?: number;
  max_sources?: number;
  processed?: number;
  has_more?: boolean;
  next_cursor?: string | null;
  source?: string;
  warnings?: string[];
}
interface PersonalizationStatus {
  enabled: boolean;
  epoch: number;
  terms_partial?: boolean;
  counts: { terms: number; events: number; imports: number };
  learned_terms: { text: string; score: number }[];
  backfill_summary?: BackfillSummary | null;
}
const request = (action: string, payload: Record<string, unknown> = {}) =>
  invoke<PersonalizationStatus>("knowledge_request", {
    action: `personalization_${action}`,
    payload,
  });
const button =
  "rounded-lg border border-mid-gray/30 px-3 py-2 text-sm hover:bg-mid-gray/10 disabled:opacity-40 disabled:cursor-not-allowed";

export function PersonalizationPanel() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<PersonalizationStatus>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [source, setSource] = useState("typed");
  const [limit, setLimit] = useState(200);
  const [confirm, setConfirm] = useState<{
    kind: "backfill" | "clear" | "forget";
    text?: string;
    cursor?: string;
  }>();
  const run = async (operation: () => Promise<PersonalizationStatus>) => {
    setBusy(true);
    setError("");
    try {
      setStatus(await operation());
      setConfirm(undefined);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    let active = true;
    request("status")
      .then((value) => {
        if (active) setStatus(value);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
    };
  }, []);
  return (
    <section
      aria-label={t("personalization.title")}
      className="rounded-xl border border-mid-gray/20 p-4"
    >
      <h2 className="font-medium">{t("personalization.title")}</h2>
      <p className="my-2 text-sm opacity-65">
        {t("personalization.description")}
      </p>
      {error && (
        <p role="alert" className="my-2 text-sm text-red-500">
          {t("knowledge.error", { error })}
        </p>
      )}
      {busy && (
        <p role="status" className="my-2 text-sm opacity-65">
          {t("knowledge.working")}
        </p>
      )}
      {!status ? (
        <button
          className={button}
          disabled={busy}
          onClick={() => void run(() => request("status"))}
        >
          {t(error ? "personalization.retry" : "knowledge.loading")}
        </button>
      ) : (
        <>
          <label className="my-3 flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={status.enabled}
              disabled={busy}
              onChange={(event) =>
                void run(() =>
                  request("enabled", { enabled: event.target.checked }),
                )
              }
            />
            {t("personalization.enabled")}
          </label>
          <p className="text-xs opacity-65">
            {t("personalization.counts", status.counts)}
          </p>
          {status.terms_partial && (
            <p role="status" className="mt-2 text-xs text-amber-600">
              {t("personalization.partialTerms")}
            </p>
          )}
          <div className="mt-4 flex flex-wrap items-center gap-2">
            <label className="text-sm">
              {t("personalization.source")}
              <select
                className="ms-2 rounded-lg border border-mid-gray/30 bg-transparent p-2"
                value={source}
                disabled={busy || !!confirm}
                onChange={(event) => setSource(event.target.value)}
              >
                {["typed", "voice", "clipboard"].map((value) => (
                  <option key={value} value={value}>
                    {t(`personalization.sources.${value}`)}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-sm">
              {t("personalization.limit")}
              <select
                className="ms-2 rounded-lg border border-mid-gray/30 bg-transparent p-2"
                value={limit}
                disabled={busy || !!confirm}
                onChange={(event) => setLimit(Number(event.target.value))}
              >
                {[10, 20, 200, 500].map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </label>
            <button
              className={button}
              disabled={busy || !status.enabled}
              onClick={() => setConfirm({ kind: "backfill" })}
            >
              {t("personalization.backfill")}
            </button>
          </div>
          <p className="mt-2 text-xs opacity-65">
            {t("personalization.backfillHelp")}
          </p>
          {status.backfill_summary && (
            <div className="mt-3 space-y-1 text-sm" role="status">
              {(
                [
                  "imported",
                  "skipped",
                  "revoked",
                  "existing",
                  "tracked_sources",
                  "processed",
                  "max_sources",
                ] as const
              ).map((key) =>
                typeof status.backfill_summary?.[key] === "number" ? (
                  <p key={key}>
                    {t(`personalization.summaryFields.${key}`, {
                      count: status.backfill_summary[key],
                    })}
                  </p>
                ) : null,
              )}
              {status.backfill_summary.has_more && (
                <p>{t("personalization.moreHistory")}</p>
              )}
              {status.backfill_summary.has_more &&
                status.backfill_summary.next_cursor && (
                  <button
                    className={button}
                    disabled={busy || !status.enabled}
                    onClick={() => {
                      setSource(status.backfill_summary?.source ?? source);
                      setConfirm({
                        kind: "backfill",
                        cursor:
                          status.backfill_summary?.next_cursor ?? undefined,
                      });
                    }}
                  >
                    {t("personalization.continueBackfill")}
                  </button>
                )}
              {status.backfill_summary.warnings?.map((warning, index) => (
                <p key={index} className="text-amber-600">
                  {warning}
                </p>
              ))}
            </div>
          )}
          <ul
            className="my-3 max-h-64 divide-y divide-mid-gray/15 overflow-y-auto"
            aria-label={t("personalization.terms")}
          >
            {status.learned_terms.map((term) => (
              <li
                key={term.text}
                className="flex items-center justify-between gap-2 py-2"
              >
                <span className="min-w-0 break-words text-sm">{term.text}</span>
                <button
                  className={button}
                  disabled={busy}
                  aria-label={t("personalization.forgetTerm", {
                    text: term.text,
                  })}
                  onClick={() =>
                    setConfirm({ kind: "forget", text: term.text })
                  }
                >
                  {t("personalization.forget")}
                </button>
              </li>
            ))}
          </ul>
          {status.learned_terms.length === 0 && (
            <p className="my-3 text-sm opacity-60">
              {t("personalization.empty")}
            </p>
          )}
          <button
            className={button}
            disabled={busy}
            onClick={() => setConfirm({ kind: "clear" })}
          >
            {t("personalization.clear")}
          </button>
          {confirm && (
            <div
              role="group"
              aria-label={t("personalization.confirm")}
              className="mt-3 rounded-lg border border-mid-gray/30 p-3"
            >
              <p className="mb-3 text-sm">
                {confirm.kind === "backfill"
                  ? t("personalization.confirmBackfill", {
                      source: t(`personalization.sources.${source}`),
                      count: limit,
                    })
                  : confirm.kind === "forget"
                    ? t("personalization.confirmForget", { text: confirm.text })
                    : t("personalization.confirmClear")}
              </p>
              <div className="flex gap-2">
                <button
                  className={button}
                  disabled={busy}
                  onClick={() =>
                    void run(() => {
                      if (confirm.kind === "backfill")
                        setStatus((current) =>
                          current
                            ? { ...current, backfill_summary: null }
                            : current,
                        );
                      return confirm.kind === "backfill"
                        ? request("backfill", {
                            source,
                            limit,
                            ...(confirm.cursor
                              ? { cursor: confirm.cursor }
                              : {}),
                          })
                        : confirm.kind === "forget"
                          ? request("forget", { text: confirm.text })
                          : request("clear");
                    })
                  }
                >
                  {t("personalization.confirm")}
                </button>
                <button
                  className={button}
                  disabled={busy}
                  onClick={() => setConfirm(undefined)}
                >
                  {t("knowledge.cancel")}
                </button>
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}
