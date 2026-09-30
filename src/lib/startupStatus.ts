import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const startupStatusSchema = z.discriminatedUnion("phase", [
  z.object({ phase: z.literal("starting") }).strict(),
  z.object({ phase: z.literal("ready") }).strict(),
  z
    .object({
      phase: z.literal("recovery"),
      reason: z.enum([
        "settings_invalid",
        "storage_unavailable",
        "migration_recovery_required",
      ]),
      settings_editable: z.boolean(),
    })
    .strict(),
]);

export type StartupStatus = z.infer<typeof startupStatusSchema>;

export async function readStartupStatus(): Promise<StartupStatus> {
  return startupStatusSchema.parse(
    await invoke<unknown>("control_settings_status"),
  );
}

export async function recoverStartup(
  action: "restart" | "open_data_folder",
): Promise<void> {
  await invoke("control_settings_recovery", { action });
}
