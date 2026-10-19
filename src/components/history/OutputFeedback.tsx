import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
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
let liveResultSeen = false;

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
      liveResultSeen = true;
      // 标准粘贴已派发不等于失败；保存失败单独说明，不冒充插入成功。
      if (result.state === "dispatched" && result.history_saved !== false)
        return;
      if (!rememberResult(result)) return;
      if (result.state === "confirmed" && result.history_saved !== false) {
        if (activeNotices.delete(result.operation_id)) showActiveNotice();
        return;
      }
      // 同一提示汇总实时故障，未知结果不能被另一条普通拒绝覆盖。
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
        if (!active || startupSummaryClaimed || liveResultSeen) return;
        startupSummaryClaimed = true;
        const page = await listUnresolvedOutputs();
        if (active && !liveResultSeen && page.items.length) {
          toast.info(t("unifiedHistory.outputNotices.startupSummary"), {
            id: notificationId,
            duration: 8000,
            closeButton: true,
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
