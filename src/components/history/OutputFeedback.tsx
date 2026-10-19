import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  acknowledgeOutputNotice,
  listUnresolvedOutputs,
  listenForOutputResults,
  type OutputResult,
} from "@/lib/unresolvedOutputs";

const notificationId = "inputia-output-attention";
const seenResults = new Map<
  string,
  Pick<OutputResult, "state" | "history_saved">
>();
const activeNotices = new Map<string, OutputResult>();
let startupSummaryClaimed = false;

function rememberResult(result: OutputResult) {
  const previous = seenResults.get(result.operation_id);
  if (
    (previous?.state === "confirmed" && result.state !== "confirmed") ||
    (previous?.state === result.state &&
      previous.history_saved === result.history_saved)
  )
    return false;
  seenResults.delete(result.operation_id);
  seenResults.set(result.operation_id, {
    state: result.state,
    history_saved: result.history_saved,
  });
  // 去重只保存本窗口的近期操作标识；持久结果仍以历史查询为准。
  if (seenResults.size > 512) {
    const oldest = seenResults.keys().next().value;
    if (oldest) seenResults.delete(oldest);
  }
  return true;
}

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
      onClick: () => {
        activeNotices.clear();
        openHistory.current();
      },
    };
    const showActiveNotice = () => {
      const notices = [...activeNotices.values()];
      const notice =
        notices.find((item) => item.state === "uncertain") ??
        notices[notices.length - 1];
      if (!notice) {
        toast.dismiss(notificationId);
        return;
      }
      const savingIssueOnly = ["dispatched", "confirmed"].includes(
        notice.state,
      );
      const message = savingIssueOnly
        ? t("unifiedHistory.feedback.historyNotSaved")
        : notice.history_saved === false && notice.state === "failed"
          ? t("unifiedHistory.feedback.failedWithoutHistory")
          : notice.history_saved === false &&
              ["pending_target", "rejected"].includes(notice.state)
            ? t("errors.pasteFailed")
            : t(`unifiedHistory.feedback.${notice.state}`);
      toast.warning(message, {
        id: notificationId,
        duration: 10000,
        closeButton: true,
        description:
          !savingIssueOnly &&
          notices.some((item) => item.history_saved === false)
            ? t("unifiedHistory.feedback.historyNotSaved")
            : undefined,
        action: notice.history_saved === false ? undefined : action,
        onDismiss: () => activeNotices.clear(),
        onAutoClose: () => activeNotices.clear(),
      });
    };
    const unlisten = listenForOutputResults((result) => {
      if (!active) return;
      // 已保存在历史记录中的普通未插入结果不再反复弹窗提醒。
      if (result.history_saved !== false) {
        if (
          result.state === "pending_target" ||
          result.state === "rejected" ||
          result.state === "uncertain"
        ) {
          void acknowledgeOutputNotice({
            operation_id: result.operation_id,
            item_id: result.operation_id,
            state: result.state,
          }).catch(() => {});
        }
        if (activeNotices.delete(result.operation_id)) showActiveNotice();
        return;
      }
      if (!rememberResult(result)) return;
      const previous = activeNotices.get(result.operation_id);
      activeNotices.delete(result.operation_id);
      activeNotices.set(result.operation_id, {
        ...result,
        state:
          result.state === "dispatched" && previous?.state === "uncertain"
            ? "uncertain"
            : result.state,
      });
      showActiveNotice();
    });
    void unlisten
      .then(async () => {
        if (!active || startupSummaryClaimed) return;
        startupSummaryClaimed = true;
        const page = await listUnresolvedOutputs();
        for (const item of page.items) {
          await acknowledgeOutputNotice(item).catch(() => {});
        }
      })
      .catch(() => {});
    return () => {
      active = false;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [t]);

  return null;
}
