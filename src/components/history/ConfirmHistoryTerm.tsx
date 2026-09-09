import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

export function ConfirmHistoryTerm({
  itemId,
  revision,
}: {
  itemId: string;
  revision: number;
}) {
  const { t } = useTranslation();
  const [term, setTerm] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<"saved" | "replayed" | "failed" | null>(
    null,
  );
  const attempt = useRef<{ term: string; id: string } | null>(null);

  const save = async () => {
    if (!confirmed || busy || term.trim().length < 2) return;
    if (!attempt.current || attempt.current.term !== term) {
      attempt.current = { term, id: crypto.randomUUID() };
    }
    setBusy(true);
    setStatus(null);
    try {
      const result = await invoke<string>("confirm_unified_history_term", {
        itemId,
        expectedRevision: revision,
        operationId: attempt.current.id,
        term,
        confirmed,
      });
      setStatus(
        result === "replay"
          ? "replayed"
          : ["applied", "already_contributed"].includes(result)
            ? "saved"
            : "failed",
      );
    } catch {
      // 回执未知时保留原操作身份，不自动提交另一笔贡献。
      setStatus("failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <details className="rounded-lg border border-text/15 p-3 text-sm">
      <summary className="cursor-pointer font-medium">
        {t("confirmHistoryTerm.title")}
      </summary>
      <div className="mt-3 space-y-3">
        <p className="text-text/60">{t("confirmHistoryTerm.description")}</p>
        <label className="block space-y-1">
          <span>{t("confirmHistoryTerm.term")}</span>
          <input
            value={term}
            maxLength={64}
            disabled={busy}
            onChange={(event) => {
              setTerm(event.target.value);
              setStatus(null);
              setConfirmed(false);
            }}
            className="w-full rounded-md border border-text/20 bg-transparent px-3 py-2 focus:outline-none focus:ring-1 focus:ring-logo-primary"
          />
        </label>
        <label className="flex items-start gap-2">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={busy}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          <span>{t("confirmHistoryTerm.consent")}</span>
        </label>
        <button
          type="button"
          disabled={
            busy ||
            !confirmed ||
            term.trim().length < 2 ||
            status === "saved" ||
            status === "replayed"
          }
          onClick={() => void save()}
          className="rounded-md border border-text/15 px-3 py-1.5 disabled:opacity-40 hover:bg-text/5"
        >
          {t(busy ? "confirmHistoryTerm.saving" : "confirmHistoryTerm.save")}
        </button>
        {status && (
          <p role={status === "failed" ? "alert" : "status"}>
            {t(`confirmHistoryTerm.${status}`)}
          </p>
        )}
      </div>
    </details>
  );
}
