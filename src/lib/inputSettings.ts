import { invoke } from "@tauri-apps/api/core";
import { z } from "zod";

const revision = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,19})$/)
  .refine((value) => BigInt(value) <= 18446744073709551615n);
const digest = z.string().regex(/^[a-f0-9]{64}$/);
const editable = z.object({
  schema_id: z.string().min(1).max(128),
  candidate_page_size: z.number().int().min(1).max(9),
  candidate_font_size: z.number().int().min(12).max(22),
  menu_icon_variant: z.enum(["pearl_16", "pearl_14", "pearl_12", "pearl_18"]),
  shift_toggle_enabled: z.boolean(),
  input_mode_toggle_shortcut: z.enum(["shift", "control_space", "none"]),
  chinese_script: z.enum(["simplified", "traditional"]),
  script_toggle_shortcut: z.enum(["control_shift_s", "none"]),
  punctuation_preference: z.enum(["english_in_chinese", "follow_input_mode"]),
  character_width_preference: z.enum(["half_width", "full_width"]),
  spelling_correction_enabled: z.boolean(),
  memory_enabled: z.boolean(),
  privacy_learning_enabled: z.boolean(),
  sensitive_bundle_ids: z
    .array(
      z
        .string()
        .min(1)
        .max(256)
        .refine((value) => new TextEncoder().encode(value).length <= 256)
        .refine((value) => !/[\u0000-\u001f\u007f]/.test(value)),
    )
    .min(1)
    .max(512),
});
export type InputSettingsValues = z.infer<typeof editable>;
export type InputSettingField = keyof InputSettingsValues;
const snapshot = z
  .object({
    store_id: z.string().uuid(),
    revision,
    values_digest: digest,
    values: editable.passthrough(),
  })
  .strict();
export type InputSettingsSnapshot = z.infer<typeof snapshot>;
const patch = editable
  .partial()
  .strict()
  .refine((value) => Object.keys(value).length > 0);
const patchRequest = z
  .object({
    operation_id: z.string().max(150),
    expected_store_id: z.string().uuid(),
    expected_revision: revision,
    patch,
  })
  .strict();
const importRequest = z
  .object({
    operation_id: z.string().max(150),
    expected_store_id: z.string().uuid(),
    expected_revision: revision,
    observed_file_digest: digest,
  })
  .strict();
const pendingOperation = z.discriminatedUnion("action", [
  z.object({ action: z.literal("apply"), request: patchRequest }).strict(),
  z
    .object({ action: z.literal("import_external"), request: importRequest })
    .strict(),
]);
export type InputSettingsOperation = z.infer<typeof pendingOperation>;
const result = z.discriminatedUnion("status", [
  z
    .object({
      status: z.literal("saved"),
      commit_revision: revision,
      replayed: z.boolean(),
      current: snapshot,
    })
    .strict(),
  z.object({ status: z.literal("conflict"), current: snapshot }).strict(),
  z
    .object({ status: z.literal("outcome_expired"), current: snapshot })
    .strict(),
]);
export type InputSettingsResult = z.infer<typeof result>;
const external = z
  .object({
    store_id: z.string().uuid(),
    revision,
    observed_file_digest: digest,
    values: editable.passthrough(),
  })
  .strict();
export type ExternalInputSettings = z.infer<typeof external>;
const pendingRecord = z
  .object({
    operation: pendingOperation,
    preview: external.optional(),
  })
  .strict()
  .refine(
    ({ operation, preview }) =>
      !preview ||
      (operation.action === "import_external" &&
        operation.request.expected_store_id === preview.store_id &&
        operation.request.expected_revision === preview.revision &&
        operation.request.observed_file_digest ===
          preview.observed_file_digest),
  );
const application = z
  .object({
    scope: z.literal("observed_engine_sessions"),
    lease_ms: z.number().int().nonnegative().max(2500),
    current_store_id: z.string().uuid(),
    current_revision: revision,
    current_values_digest: digest,
    sessions: z
      .array(
        z
          .object({
            remaining_ms: z.number().int().positive().max(2500).optional(),
            instance_id: z.string().uuid(),
            store_id: z.string().uuid(),
            revision,
            values_digest: digest,
            applied_fields: z.array(z.string()).max(32),
            unavailable_fields: z.array(z.string()).max(32),
          })
          .strict(),
      )
      .max(1024),
  })
  .strict();
export type InputSettingsApplication = z.infer<typeof application>;
export class InputSettingsError extends Error {
  constructor(public code: string) {
    super(code);
    this.name = "InputSettingsError";
  }
}
async function call<T>(
  request: unknown,
  key: string,
  schema: z.ZodType<T>,
): Promise<T> {
  const raw: unknown = await invoke("input_settings_request", { request });
  const error = z
    .object({ ok: z.literal(false), code: z.string() })
    .strict()
    .safeParse(raw);
  if (error.success) throw new InputSettingsError(error.data.code);
  const envelope = z
    .object({ ok: z.literal(true), [key]: schema })
    .strict()
    .parse(raw);
  return schema.parse(envelope[key]);
}
export const readInputSettings = () =>
  call({ action: "read" }, "snapshot", snapshot);
export const inspectExternalInputSettings = () =>
  call({ action: "inspect_external" }, "external", external);
export const inputSettingsApplication = () =>
  call({ action: "application_status" }, "application", application);
export const applyInputSettings = (operation: InputSettingsOperation) =>
  call(pendingOperation.parse(operation), "result", result);
export function createInputSettingsPatch(
  base: InputSettingsSnapshot,
  changes: Partial<InputSettingsValues>,
): InputSettingsOperation {
  return pendingOperation.parse({
    action: "apply",
    request: {
      operation_id: `v1:${base.store_id}:${base.revision}:${crypto.randomUUID()}`,
      expected_store_id: base.store_id,
      expected_revision: base.revision,
      patch: changes,
    },
  });
}
export function createExternalInputSettingsImport(
  preview: ExternalInputSettings,
): InputSettingsOperation {
  return pendingOperation.parse({
    action: "import_external",
    request: {
      operation_id: `v1:${preview.store_id}:${preview.revision}:${crypto.randomUUID()}`,
      expected_store_id: preview.store_id,
      expected_revision: preview.revision,
      observed_file_digest: preview.observed_file_digest,
    },
  });
}
// 先保存操作身份再发送。组件关闭或传输结果未知时，也不能生成新ID重复同一修改。
const pendingKey = "inputia.settings.pending.v1";
export function rememberInputSettingsOperation(
  operation: InputSettingsOperation,
  preview?: ExternalInputSettings,
) {
  const existing = recoverInputSettingsOperation();
  if (existing && !sameOperation(existing.operation, operation)) {
    throw new InputSettingsError("pending_operation_changed");
  }
  try {
    localStorage.setItem(
      pendingKey,
      JSON.stringify(
        pendingRecord.parse({
          operation,
          preview: existing?.preview ?? preview,
        }),
      ),
    );
  } catch {
    throw new InputSettingsError("pending_storage_unavailable");
  }
}
const sameOperation = (a: InputSettingsOperation, b: InputSettingsOperation) =>
  JSON.stringify(pendingOperation.parse(a)) ===
  JSON.stringify(pendingOperation.parse(b));
export function recoverInputSettingsOperation():
  | z.infer<typeof pendingRecord>
  | undefined {
  let value: string | null;
  try {
    value = localStorage.getItem(pendingKey);
  } catch {
    throw new InputSettingsError("pending_storage_unavailable");
  }
  if (!value) return undefined;
  try {
    return pendingRecord.parse(JSON.parse(value));
  } catch {
    throw new InputSettingsError("pending_record_invalid");
  }
}
// 清理只作用于当前请求，卸载组件的迟到响应不能清除后来操作的身份。
export function forgetInputSettingsOperation(
  expected?: InputSettingsOperation,
) {
  const current = recoverInputSettingsOperation();
  if (!current) return;
  if (!expected || !sameOperation(current.operation, expected)) {
    throw new InputSettingsError("pending_operation_changed");
  }
  try {
    localStorage.removeItem(pendingKey);
  } catch {
    throw new InputSettingsError("pending_storage_unavailable");
  }
}
export function discardUnreadableInputSettingsOperation() {
  try {
    const current = recoverInputSettingsOperation();
    if (!current) return;
  } catch (cause) {
    if (
      cause instanceof InputSettingsError &&
      cause.code === "pending_record_invalid"
    ) {
      try {
        localStorage.removeItem(pendingKey);
        return;
      } catch {
        throw new InputSettingsError("pending_storage_unavailable");
      }
    }
    throw cause;
  }
  throw new InputSettingsError("pending_operation_changed");
}
