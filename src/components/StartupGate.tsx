import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  readStartupStatus,
  recoverStartup,
  type StartupStatus,
} from "@/lib/startupStatus";

interface Props {
  children: ReactNode;
  initialize: () => Promise<void>;
}

// 这里控制界面挂载；原生 invoke 入口仍须独立检查业务准入。
export function StartupGate({ children, initialize }: Props) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<StartupStatus>({ phase: "starting" });
  const [unconfirmed, setUnconfirmed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [busy, setBusy] = useState(false);
  const [actionFailed, setActionFailed] = useState(false);
  const initialization = useRef<Promise<void> | null>(null);

  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setUnconfirmed(false);
    const check = async () => {
      try {
        const next = await readStartupStatus();
        if (!active) return;
        if (next.phase === "ready") {
          initialization.current ??= initialize();
          const pending = initialization.current;
          try {
            await pending;
          } catch (error) {
            if (initialization.current === pending)
              initialization.current = null;
            throw error;
          }
          if (!active) return;
        }
        setStatus(next);
        if (next.phase === "starting") timer = setTimeout(check, 500);
      } catch {
        if (active) setUnconfirmed(true);
      }
    };
    void check();
    return () => {
      active = false;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [initialize, retry]);

  const perform = async (action: "restart" | "open_data_folder") => {
    if (busy) return;
    setBusy(true);
    setActionFailed(false);
    try {
      await recoverStartup(action);
    } catch {
      setActionFailed(true);
    } finally {
      setBusy(false);
    }
  };

  if (!unconfirmed && status.phase === "ready") return <>{children}</>;
  const recovery = status.phase === "recovery";
  return (
    <main className="min-h-screen flex items-center justify-center p-8">
      <section className="max-w-md space-y-5" aria-live="polite">
        <h1 className="text-xl font-semibold">
          {t(
            recovery || unconfirmed
              ? "startupRecovery.title"
              : "startupRecovery.starting",
          )}
        </h1>
        {(recovery || unconfirmed) && (
          <p className="text-sm leading-relaxed text-text/70">
            {t(
              unconfirmed
                ? "startupRecovery.unconfirmed"
                : status.phase === "recovery" && status.settings_editable
                  ? "startupRecovery.editable"
                  : "startupRecovery.pendingRecovery",
            )}
          </p>
        )}
        {unconfirmed ? (
          <button
            type="button"
            className="px-4 py-2 rounded-lg border border-text/20"
            onClick={() => setRetry((value) => value + 1)}
          >
            {t("startupRecovery.retry")}
          </button>
        ) : recovery ? (
          <div className="flex flex-wrap gap-3">
            <button
              type="button"
              className="px-4 py-2 rounded-lg border border-text/20"
              disabled={busy}
              onClick={() => void perform("restart")}
            >
              {t("startupRecovery.restart")}
            </button>
            <button
              type="button"
              className="px-4 py-2 rounded-lg border border-text/20"
              disabled={busy}
              onClick={() => void perform("open_data_folder")}
            >
              {t("startupRecovery.openFolder")}
            </button>
          </div>
        ) : null}
        {actionFailed && (
          <p role="alert" className="text-sm">
            {t("startupRecovery.actionFailed")}
          </p>
        )}
      </section>
    </main>
  );
}
