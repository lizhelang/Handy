import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { useClipboardStore } from "@/stores/clipboardStore";
import { ClipboardStatsBar } from "./ClipboardStats";
import { ClipboardToolbar } from "./ClipboardToolbar";
import { ClipboardList } from "./ClipboardList";
import { ClipboardGrid } from "./ClipboardGrid";
import { ClipboardPreview } from "./ClipboardPreview";
import { Button } from "../ui/Button";

export const ClipboardSettings: React.FC = () => {
  const { t } = useTranslation();
  const initialized = useClipboardStore((s) => s.initialized);
  const viewMode = useClipboardStore((s) => s.viewMode);
  const previewItem = useClipboardStore((s) => s.previewItem);
  const stats = useClipboardStore((s) => s.stats);
  const initialize = useClipboardStore((s) => s.initialize);
  const clearHistory = useClipboardStore((s) => s.clearHistory);
  const hidePreview = useClipboardStore((s) => s.hidePreview);
  const copyItem = useClipboardStore((s) => s.copyItem);
  const toggleFavorite = useClipboardStore((s) => s.toggleFavorite);
  const togglePin = useClipboardStore((s) => s.togglePin);

  const [showClearConfirm, setShowClearConfirm] = useState(false);
  const [keepPinned, setKeepPinned] = useState(true);

  useEffect(() => {
    if (!initialized) {
      initialize();
    }
  }, [initialized, initialize]);

  const handleClear = async () => {
    await clearHistory(keepPinned);
    setShowClearConfirm(false);
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-4">
      <div className="px-4 flex items-center justify-between">
        <h2 className="text-xs font-medium text-mid-gray uppercase tracking-wide">
          {t("settings.clipboard.title")}
        </h2>
        <Button
          onClick={() => setShowClearConfirm(true)}
          variant="danger-ghost"
          size="sm"
          className="flex items-center gap-1.5"
        >
          <Trash2 className="w-3.5 h-3.5" />
          <span>{t("settings.clipboard.clearHistory")}</span>
        </Button>
      </div>

      <ClipboardStatsBar stats={stats} />

      <div className="bg-background border border-mid-gray/20 rounded-lg overflow-visible">
        <ClipboardToolbar />
        <div className="border-t border-mid-gray/10">
          {viewMode === "list" ? <ClipboardList /> : <ClipboardGrid />}
        </div>
      </div>

      {previewItem && (
        <ClipboardPreview
          item={previewItem}
          onClose={hidePreview}
          onCopy={() => copyItem(previewItem.id)}
          onToggleFavorite={() => toggleFavorite(previewItem.id)}
          onTogglePin={() => togglePin(previewItem.id)}
        />
      )}

      {showClearConfirm && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
          onClick={() => setShowClearConfirm(false)}
        >
          <div
            className="bg-background border border-mid-gray/30 rounded-xl shadow-2xl max-w-sm w-full mx-4 p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-medium mb-2">
              {t("settings.clipboard.clearHistory")}
            </h3>
            <p className="text-xs text-text/60 mb-4">
              {t("settings.clipboard.clearConfirm")}
            </p>
            <label className="flex items-center gap-2 mb-4 cursor-pointer">
              <input
                type="checkbox"
                checked={keepPinned}
                onChange={(e) => setKeepPinned(e.target.checked)}
                className="rounded border-mid-gray/40"
              />
              <span className="text-xs text-text/70">
                {t("settings.clipboard.clearKeepPinned")}
              </span>
            </label>
            <div className="flex justify-end gap-2">
              <Button
                onClick={() => setShowClearConfirm(false)}
                variant="secondary"
                size="sm"
              >
                {t("common.cancel")}
              </Button>
              <Button onClick={handleClear} variant="danger" size="sm">
                {t("common.delete")}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
