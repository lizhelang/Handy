import { useEffect, useRef } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import { UnifiedHistory } from "./UnifiedHistory";
import { UnresolvedOutputNotices } from "./UnresolvedOutputNotices";
import { requireActiveOutputAttempt } from "@/stores/unifiedOutputStore";
import type { UnifiedHistoryActionResult } from "@/lib/types/unifiedHistory";

function actionResult(status: string): UnifiedHistoryActionResult {
  switch (status) {
    case "confirmed":
    case "dispatched":
    case "pending_target":
    case "uncertain":
    case "rejected":
    case "failed":
      return { status };
    default:
      throw new Error("Unknown output receipt");
  }
}

export function UnifiedHistoryPage() {
  const updates = useRef(new Map<string, string>());
  useEffect(() => () => updates.current.clear(), []);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <UnresolvedOutputNotices />
      <div className="min-h-0 flex-1">
        <UnifiedHistory
          onCheckOutputReceipt={async (operationId) => {
            const result = await commands.getUnifiedOutputReceipt(operationId);
            if (result.status === "error") throw new Error(result.error);
            return result.data ? actionResult(result.data.status) : null;
          }}
          onRetranscribe={async (item) => {
            const result = await commands.retranscribeUnifiedHistoryItem(
              item.item_id,
              item.revision,
            );
            if (result.status === "error") throw new Error(result.error);
          }}
          onOpenRecordings={async () => {
            const result = await commands.openRecordingsFolder();
            if (result.status === "error") throw new Error(result.error);
          }}
          onCopy={async (item) => {
            const attempt = requireActiveOutputAttempt(item.item_id, "copy");
            const result = await commands.copyUnifiedHistoryItem(
              item.item_id,
              attempt.revision,
              attempt.operationId,
            );
            if (result.status === "error") throw new Error(result.error);
            return actionResult(result.data.status);
          }}
          onInsert={async (item) => {
            const attempt = requireActiveOutputAttempt(item.item_id, "insert");
            const result = await commands.insertUnifiedHistoryItem(
              item.item_id,
              attempt.revision,
              attempt.operationId,
            );
            if (result.status === "error") throw new Error(result.error);
            return actionResult(result.data.status);
          }}
          onUpdate={async (item, patch) => {
            const key = JSON.stringify([item.item_id, item.revision, patch]);
            let operation = updates.current.get(key);
            if (!operation) {
              operation = crypto.randomUUID();
              if (updates.current.size >= 32) {
                const oldest = updates.current.keys().next().value;
                if (oldest) updates.current.delete(oldest);
              }
              updates.current.set(key, operation);
            }
            const result = await commands.updateUnifiedHistoryItem(
              item.item_id,
              item.revision,
              operation,
              {
                starred: patch.starred ?? null,
                pinned: patch.pinned ?? null,
                title: patch.title ?? null,
                clear_title: patch.title === null,
                text: patch.text ?? null,
              },
            );
            if (result.status === "error") throw new Error(result.error);
          }}
          resolveAsset={async (item) => {
            const result = await commands.getUnifiedHistoryAsset(
              item.item_id,
              item.revision,
            );
            if (result.status === "error") throw new Error(result.error);
            return result.data ? convertFileSrc(result.data) : null;
          }}
        />
      </div>
    </div>
  );
}
