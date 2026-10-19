import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { GripVertical, X } from "lucide-react";
import { commands } from "@/bindings";
import InputiaMark from "../icons/InputiaMark";

type Props = {
  open: boolean;
  onClose: () => void;
};

export function AccessibilityDragAssist({ open, onClose }: Props) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);

  const run = useCallback(async (action: "assist" | "app" | "settings") => {
    setBusy(true);
    setStatus(null);
    try {
      const result = await commands.openInputiaPermissionHelp(action);
      if (result.status === "error") {
        setStatus("accessibilityDragAssist.failed");
        return;
      }
      if (action === "assist") setStatus("accessibilityDragAssist.opened");
      if (action === "app") setStatus("accessibilityDragAssist.revealed");
    } catch {
      setStatus("accessibilityDragAssist.failed");
    } finally {
      setBusy(false);
    }
  }, []);

  const startDrag = useCallback(async () => {
    setStatus(null);
    try {
      const result = await commands.beginInputiaAppBundleDrag();
      if (result.status === "error") {
        // 拖拽失败时仍把应用露出来，避免用户卡在“点加号找应用”。
        await run("app");
        setStatus("accessibilityDragAssist.dragFallback");
      }
    } catch {
      await run("app");
      setStatus("accessibilityDragAssist.dragFallback");
    }
  }, [run]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("accessibilityDragAssist.title")}
        className="w-full max-w-sm rounded-xl border border-mid-gray/30 bg-background p-4 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="mb-3 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-base font-semibold text-text">
              {t("accessibilityDragAssist.title")}
            </h2>
            <p className="mt-1 text-sm text-text/65">
              {t("accessibilityDragAssist.description")}
            </p>
          </div>
          <button
            type="button"
            className="rounded-md p-1 text-text/50 hover:bg-mid-gray/15 hover:text-text"
            aria-label={t("common.close")}
            onClick={onClose}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <button
          type="button"
          disabled={busy}
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            void startDrag();
          }}
          className="flex w-full cursor-grab flex-col items-center gap-3 rounded-xl border border-dashed border-logo-primary/50 bg-logo-primary/10 px-4 py-6 active:cursor-grabbing disabled:opacity-50"
          aria-label={t("accessibilityDragAssist.dragHandle")}
        >
          <div className="flex items-center gap-2 text-text/55">
            <GripVertical className="h-4 w-4" />
            <span className="text-xs">
              {t("accessibilityDragAssist.dragHint")}
            </span>
          </div>
          <InputiaMark size={56} className="text-accent-text" />
          <span className="text-sm font-medium text-text">
            {t("accessibilityDragAssist.appLabel")}
          </span>
        </button>

        <ol className="mt-4 list-decimal space-y-1.5 ps-5 text-sm text-text/70">
          <li>{t("accessibilityDragAssist.stepOpen")}</li>
          <li>{t("accessibilityDragAssist.stepDrag")}</li>
          <li>{t("accessibilityDragAssist.stepToggle")}</li>
        </ol>

        <div className="mt-4 flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy}
            className="rounded-lg bg-logo-primary px-3 py-2 text-sm font-medium text-white hover:bg-logo-primary/90 disabled:opacity-50"
            onClick={() => void run("assist")}
          >
            {t("accessibilityDragAssist.openAssist")}
          </button>
          <button
            type="button"
            disabled={busy}
            className="rounded-lg border border-mid-gray/30 px-3 py-2 text-sm text-text/80 hover:bg-mid-gray/10 disabled:opacity-50"
            onClick={() => void run("app")}
          >
            {t("accessibilityDragAssist.revealApp")}
          </button>
        </div>

        {status && (
          <p role="status" className="mt-3 text-sm text-text/60">
            {t(status)}
          </p>
        )}
      </div>
    </div>
  );
}
