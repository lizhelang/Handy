import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { z } from "zod";

const attachmentLease = z
  .object({
    lease_id: z.string(),
    instance_id: z.string(),
    attachment_id: z.string().nullable(),
    purpose: z.enum(["active", "export", "update"]),
  })
  .strict();
const acquiredAttachment = z
  .object({
    lease: attachmentLease,
    path: z.string().min(1),
    revision: z.number().int().nonnegative(),
  })
  .strict();

export interface AudioSource {
  url: string;
  release: () => void;
}

// 取消记录先于迟到 acquire 落盘；失败保留重试，不能把失败当作已释放。
const pendingCancellations = new Map<
  string,
  { running: boolean; attempts: number }
>();
function cancelOperation(operationId: string) {
  const pending = pendingCancellations.get(operationId) ?? {
    running: false,
    attempts: 0,
  };
  pendingCancellations.set(operationId, pending);
  if (pending.running) return;
  pending.running = true;
  void invoke("release_history_attachment_operation", { operationId })
    .then(() => {
      pendingCancellations.delete(operationId);
    })
    .catch(() => {
      pending.running = false;
      pending.attempts += 1;
      setTimeout(
        () => cancelOperation(operationId),
        Math.min(30_000, 1_000 * 2 ** Math.min(pending.attempts, 5)),
      );
    });
}

/** 一次播放持有一次 Active lease。取消、错误、迟到成功均通过相同 operationId 收口。 */
export async function acquireHistoryAudio(
  id: number,
  osType: string | null,
  signal: AbortSignal,
): Promise<AudioSource | null> {
  const operationId = crypto.randomUUID();
  let url: string | undefined;
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    signal.removeEventListener("abort", release);
    if (url?.startsWith("blob:")) URL.revokeObjectURL(url);
    cancelOperation(operationId);
  };
  signal.addEventListener("abort", release, { once: true });
  if (signal.aborted) {
    release();
    return null;
  }
  try {
    const result = acquiredAttachment.parse(
      await invoke("acquire_history_attachment", {
        id,
        expectedRevision: null,
        operationId,
      }),
    );
    if (signal.aborted) {
      release();
      return null;
    }
    if (osType === "linux") {
      const bytes = await readFile(result.path);
      if (signal.aborted) {
        release();
        return null;
      }
      url = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
    } else {
      url = convertFileSrc(result.path, "asset");
    }
    return { url, release };
  } catch (error) {
    release();
    if (signal.aborted) return null;
    throw error;
  }
}
