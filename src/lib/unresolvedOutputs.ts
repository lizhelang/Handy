import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const noticeSchema = z.object({
  operation_id: z.string().min(1),
  item_id: z.string().min(1),
  state: z.enum(["pending_target", "uncertain", "rejected"]),
});
const pageSchema = z.object({
  items: z.array(noticeSchema).max(100),
  next_cursor: z.string().nullable(),
});
export type UnresolvedOutputNotice = z.infer<typeof noticeSchema>;

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
