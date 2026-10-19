import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { z } from "zod";

const noticeStateSchema = z.enum(["pending_target", "uncertain", "rejected"]);
const noticeSchema = z.object({
  operation_id: z.string().min(1),
  item_id: z.string().min(1),
  state: noticeStateSchema,
});
const pageSchema = z.object({
  items: z.array(noticeSchema).max(100),
  next_cursor: z.string().nullable(),
});
export type UnresolvedOutputNotice = z.infer<typeof noticeSchema>;

const outputResultSchema = z.object({
  operation_id: z.string().min(1),
  history_saved: z.boolean().optional(),
  state: z.enum([
    "pending_target",
    "uncertain",
    "rejected",
    "confirmed",
    "dispatched",
    "failed",
  ]),
});
export type OutputResult = z.infer<typeof outputResultSchema>;

export function listenForOutputResults(
  onResult: (result: OutputResult) => void,
) {
  return listen<unknown>("voice-output-result", (event) => {
    const parsed = outputResultSchema.safeParse(event.payload);
    if (parsed.success) onResult(parsed.data);
  });
}

export async function listUnresolvedOutputs(cursor: string | null = null) {
  return pageSchema.parse(
    await invoke("list_unresolved_unified_outputs", { cursor, limit: 50 }),
  );
}

export async function acknowledgeOutputNotice(notice: UnresolvedOutputNotice) {
  await invoke("acknowledge_unified_output_notice", {
    operationId: notice.operation_id,
    expectedState: notice.state,
  });
}
