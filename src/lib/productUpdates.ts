import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const updateReply = z
  .object({
    status: z.enum([
      "disabled",
      "checking",
      "current",
      "available",
      "unavailable",
    ]),
    reason: z.string().nullable(),
    version: z.string().nullable(),
    release_id: z.string().nullable(),
    // 配套安装器预检接线前，目录查询不会向界面发放可安装许可。
    installable: z.literal(false),
  })
  .strict();
export type ProductUpdateReply = z.infer<typeof updateReply>;
export async function checkProductUpdate(): Promise<ProductUpdateReply> {
  return updateReply.parse(await invoke("check_product_update"));
}
