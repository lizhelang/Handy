import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { listUnresolvedOutputs } from "@/lib/unresolvedOutputs";
import type { UnifiedHistoryActionResult } from "@/lib/types/unifiedHistory";

type OutputResult = {
  operation_id: string;
  state: UnifiedHistoryActionResult["status"];
};

export function OutputFeedback({
  onOpenHistory,
}: {
  onOpenHistory: () => void;
}) {
  const { t } = useTranslation();
  const openHistory = useRef(onOpenHistory);
  openHistory.current = onOpenHistory;

  useEffect(() => {
    let active = true;
    const action = {
      label: t("sidebar.history"),
      onClick: () => openHistory.current(),
    };
    const unlisten = listen<OutputResult>("voice-output-result", (event) => {
      const { operation_id, state } = event.payload;
      if (
        !active ||
        !["pending_target", "uncertain", "rejected"].includes(state)
      )
        return;
      // 新输出单独提示；历史结果留在持久收件箱，不伪装成持续失败。
      toast.warning(t(`unifiedHistory.feedback.${state}`), {
        id: operation_id,
        duration: Infinity,
        action,
      });
    });
    void unlisten
      .then(async () => {
        const page = await listUnresolvedOutputs();
        if (active && page.items.length) {
          toast.info(t("unifiedHistory.outputNotices.startupSummary"), {
            id: "inputia-historical-output-summary",
            duration: 8000,
            action,
          });
        }
      })
      .catch(() => {
        // 历史页保留可重试的查询错误，不将失败查询当作空收件箱。
      });
    return () => {
      active = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [t]);

  return null;
}
