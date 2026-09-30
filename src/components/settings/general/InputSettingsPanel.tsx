import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import {
  applyInputSettings,
  createExternalInputSettingsImport,
  createInputSettingsPatch,
  forgetInputSettingsOperation,
  discardUnreadableInputSettingsOperation,
  inputSettingsApplication,
  inspectExternalInputSettings,
  InputSettingsError,
  readInputSettings,
  recoverInputSettingsOperation,
  rememberInputSettingsOperation,
  type ExternalInputSettings,
  type InputSettingField,
  type InputSettingsApplication,
  type InputSettingsOperation,
  type InputSettingsResult,
  type InputSettingsSnapshot,
  type InputSettingsValues,
} from "@/lib/inputSettings";

const schemas = [
  "luna_pinyin_simp",
  "double_pinyin",
  "double_pinyin_flypy",
  "double_pinyin_sogou",
  "guobiao_bispell",
  "double_pinyin_mspy",
  "double_pinyin_abc",
  "double_pinyin_pyjj",
  "double_pinyin_st",
];
const spellingSchemas = new Set([
  "luna_pinyin",
  "luna_pinyin_simp",
  "luna_pinyin_tw",
  "luna_pinyin_fluency",
  "luna_quanpin",
]);
const fields: InputSettingField[] = [
  "schema_id",
  "candidate_page_size",
  "candidate_font_size",
  "menu_icon_variant",
  "input_mode_toggle_shortcut",
  "chinese_script",
  "script_toggle_shortcut",
  "punctuation_preference",
  "character_width_preference",
  "spelling_correction_enabled",
  "memory_enabled",
  "privacy_learning_enabled",
  "sensitive_bundle_ids",
];
const button =
  "rounded-lg border border-mid-gray/30 px-3 py-2 text-sm hover:bg-mid-gray/10 disabled:opacity-40 disabled:cursor-not-allowed";
const control =
  "rounded-lg border border-mid-gray/30 bg-transparent px-2 py-1.5 text-sm disabled:opacity-40";
const same = (a: unknown, b: unknown) =>
  JSON.stringify(a) === JSON.stringify(b);

export function InputSettingsPanel() {
  const { t } = useTranslation();
  const mounted = useRef(true);
  const requestGeneration = useRef(0);
  const isCurrent = (ticket: number) =>
    mounted.current && requestGeneration.current === ticket;
  const [base, setBase] = useState<InputSettingsSnapshot>();
  const [draft, setDraft] = useState<Partial<InputSettingsValues>>({});
  const [sensitiveText, setSensitiveText] = useState("");
  const [pending, setPending] = useState<InputSettingsOperation>();
  const [conflict, setConflict] =
    useState<
      Extract<InputSettingsResult, { status: "conflict" | "outcome_expired" }>
    >();
  const [external, setExternal] = useState<ExternalInputSettings>();
  const [application, setApplication] = useState<InputSettingsApplication>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState<{ revision: string; replayed: boolean }>();
  const [recordBroken, setRecordBroken] = useState(false);
  const values = useMemo(
    () => (base ? { ...base.values, ...draft } : undefined),
    [base, draft],
  );
  const dirty = Object.keys(draft).length > 0;
  const locked = busy || !!pending || !!conflict || !!external || recordBroken;
  const report = (cause: unknown) =>
    setError(
      cause instanceof InputSettingsError ? cause.code : "response_unknown",
    );
  const acceptSnapshot = useCallback((value: InputSettingsSnapshot) => {
    setBase(value);
    setDraft({});
    setSensitiveText(value.values.sensitive_bundle_ids.join("\n"));
  }, []);
  useEffect(() => {
    mounted.current = true;
    const ticket = ++requestGeneration.current;
    try {
      const recovered = recoverInputSettingsOperation();
      setPending(recovered?.operation);
      setExternal(recovered?.preview);
      if (recovered?.operation.action === "apply")
        setDraft(recovered.operation.request.patch);
    } catch (cause) {
      setRecordBroken(true);
      report(cause);
    }
    void readInputSettings()
      .then((value) => {
        if (!isCurrent(ticket)) return;
        setBase(value);
        const recovered = recoverInputSettingsOperation();
        setSensitiveText(
          (recovered?.operation.action === "apply"
            ? recovered.operation.request.patch.sensitive_bundle_ids
            : undefined
          )?.join("\n") ?? value.values.sensitive_bundle_ids.join("\n"),
        );
      })
      .catch((cause) => {
        if (isCurrent(ticket)) report(cause);
      });
    return () => {
      mounted.current = false;
      requestGeneration.current++;
    };
  }, []);
  useEffect(() => {
    setApplication(undefined);
    if (!base) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let expiry: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const started = performance.now();
      try {
        const status = await inputSettingsApplication();
        if (!active) return;
        const remaining = status.lease_ms - (performance.now() - started);
        if (remaining > 0) {
          setApplication(status);
          clearTimeout(expiry);
          expiry = setTimeout(() => {
            if (active) setApplication(undefined);
          }, remaining);
        } else setApplication(undefined);
      } catch {
        if (active) setApplication(undefined);
      }
      if (active) timer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
      clearTimeout(expiry);
    };
  }, [base?.store_id]);
  function change<K extends InputSettingField>(
    key: K,
    value: InputSettingsValues[K],
  ) {
    if (!base) return;
    setSaved(undefined);
    setDraft((previous) => {
      const next = { ...previous, [key]: value };
      if (same(value, base.values[key])) delete next[key];
      if (key === "input_mode_toggle_shortcut") {
        next.shift_toggle_enabled = value === "shift";
        if (next.shift_toggle_enabled === base.values.shift_toggle_enabled)
          delete next.shift_toggle_enabled;
      }
      return next;
    });
  }
  async function submit(operation: InputSettingsOperation) {
    const ticket = ++requestGeneration.current;
    setBusy(true);
    setError("");
    setSaved(undefined);
    try {
      rememberInputSettingsOperation(
        operation,
        operation.action === "import_external" ? external : undefined,
      );
      setPending(operation);
      const result = await applyInputSettings(operation);
      if (!isCurrent(ticket)) return;
      if (result.status === "saved") {
        forgetInputSettingsOperation(operation);
        setPending(undefined);
        setConflict(undefined);
        setExternal(undefined);
        acceptSnapshot(result.current);
        setSaved({
          revision: result.commit_revision,
          replayed: result.replayed,
        });
      } else {
        setConflict(result);
      }
    } catch (cause) {
      if (isCurrent(ticket)) report(cause);
    } finally {
      if (isCurrent(ticket)) setBusy(false);
    }
  }
  async function save() {
    if (!base || !dirty) return;
    try {
      await submit(createInputSettingsPatch(base, draft));
    } catch {
      setError("invalid_request");
    }
  }
  async function refresh() {
    const ticket = ++requestGeneration.current;
    setBusy(true);
    setError("");
    try {
      const current = await readInputSettings();
      if (!isCurrent(ticket)) return;
      if (
        dirty &&
        base &&
        (current.store_id !== base.store_id ||
          current.revision !== base.revision ||
          current.values_digest !== base.values_digest)
      )
        setConflict({ status: "conflict", current });
      else if (!dirty && !pending) acceptSnapshot(current);
      else if (!base) setBase(current);
    } catch (cause) {
      if (isCurrent(ticket)) report(cause);
    } finally {
      if (isCurrent(ticket)) setBusy(false);
    }
  }
  async function previewExternal() {
    if (pending) return;
    const ticket = ++requestGeneration.current;
    setBusy(true);
    setError("");
    try {
      const preview = await inspectExternalInputSettings();
      if (isCurrent(ticket)) setExternal(preview);
    } catch (cause) {
      if (isCurrent(ticket)) report(cause);
    } finally {
      if (isCurrent(ticket)) setBusy(false);
    }
  }
  function useCurrent() {
    if (!conflict) return;
    try {
      forgetInputSettingsOperation(pending);
      setPending(undefined);
      setSaved(undefined);
      acceptSnapshot(conflict.current);
      setConflict(undefined);
      setExternal(undefined);
      setError("");
    } catch (cause) {
      report(cause);
    }
  }
  // 用户明确停止重试只丢弃本地请求，不撤销可能已发生的提交，也不改服务端配置。
  async function abandonRetry() {
    const ticket = ++requestGeneration.current;
    setBusy(true);
    try {
      if (recordBroken) discardUnreadableInputSettingsOperation();
      else forgetInputSettingsOperation(pending);
      setPending(undefined);
      setDraft({});
      setConflict(undefined);
      setExternal(undefined);
      setRecordBroken(false);
      setSaved(undefined);
      setError("");
      setBase(undefined);
      const current = await readInputSettings();
      if (isCurrent(ticket)) acceptSnapshot(current);
    } catch (cause) {
      if (isCurrent(ticket)) report(cause);
    } finally {
      if (isCurrent(ticket)) setBusy(false);
    }
  }
  const showValue = (field: InputSettingField, value: unknown): string => {
    if (typeof value === "boolean")
      return t(value ? "inputSettings.on" : "inputSettings.off");
    if (Array.isArray(value)) return value.join("\n");
    if (field === "schema_id")
      return schemas.includes(String(value))
        ? t(`inputSettings.schemas.${String(value)}`)
        : String(value);
    if (typeof value === "string" && field !== "sensitive_bundle_ids")
      return t(`inputSettings.options.${value}`, { defaultValue: value });
    return String(value ?? "");
  };
  const matchingSessions =
    application?.sessions.filter(
      (session) =>
        base &&
        session.store_id === base.store_id &&
        session.revision === base.revision &&
        session.values_digest === base.values_digest,
    ) ?? [];
  const attempted =
    pending?.action === "import_external" ? external?.values : draft;
  const settingFields = (names: string[]) => {
    const known = names.filter((name): name is InputSettingField =>
      fields.includes(name as InputSettingField),
    );
    const labels = known.map((name) => t(`inputSettings.fields.${name}`));
    if (known.length !== names.length)
      labels.push(t("inputSettings.runtimeFields"));
    return labels.join(t("inputSettings.separator"));
  };
  const select = (
    field: InputSettingField,
    options: string[],
    onChange: (value: string) => void,
  ) => (
    <select
      id={`input-setting-${field}`}
      className={control}
      value={String(values?.[field] ?? "")}
      disabled={locked}
      onChange={(event) => onChange(event.target.value)}
    >
      {options.map((option) => (
        <option key={option} value={option}>
          {showValue(field, option)}
        </option>
      ))}
    </select>
  );
  const row = (
    field: InputSettingField,
    node: React.ReactNode,
    hint?: string,
  ) => (
    <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
      <div>
        <label
          className="text-sm font-medium"
          htmlFor={`input-setting-${field}`}
        >
          {t(`inputSettings.fields.${field}`)}
        </label>
        {hint && (
          <p className="mt-1 max-w-md text-xs text-mid-gray">{t(hint)}</p>
        )}
      </div>
      {node}
    </div>
  );
  const toggle = (
    field:
      | "spelling_correction_enabled"
      | "memory_enabled"
      | "privacy_learning_enabled",
    hint?: string,
  ) =>
    row(
      field,
      <input
        id={`input-setting-${field}`}
        type="checkbox"
        checked={Boolean(values?.[field])}
        disabled={
          locked ||
          (field === "spelling_correction_enabled" &&
            !spellingSchemas.has(values?.schema_id ?? ""))
        }
        onChange={(event) => change(field, event.target.checked)}
      />,
      hint,
    );
  const errorKey =
    error &&
    t(`inputSettings.errors.${error}`, {
      defaultValue: t("inputSettings.errors.response_unknown"),
    });
  return (
    <section aria-label={t("inputSettings.title")}>
      <SettingsGroup
        title={t("inputSettings.title")}
        description={t("inputSettings.description")}
      >
        {values && (
          <>
            {row(
              "schema_id",
              select(
                "schema_id",
                schemas.includes(values.schema_id)
                  ? schemas
                  : [values.schema_id, ...schemas],
                (value) => change("schema_id", value),
              ),
            )}
            {row(
              "input_mode_toggle_shortcut",
              select(
                "input_mode_toggle_shortcut",
                ["shift", "control_space", "none"],
                (value) =>
                  change(
                    "input_mode_toggle_shortcut",
                    value as InputSettingsValues["input_mode_toggle_shortcut"],
                  ),
              ),
            )}
            {row(
              "chinese_script",
              select("chinese_script", ["simplified", "traditional"], (value) =>
                change(
                  "chinese_script",
                  value as InputSettingsValues["chinese_script"],
                ),
              ),
            )}
            {row(
              "script_toggle_shortcut",
              select(
                "script_toggle_shortcut",
                ["control_shift_s", "none"],
                (value) =>
                  change(
                    "script_toggle_shortcut",
                    value as InputSettingsValues["script_toggle_shortcut"],
                  ),
              ),
            )}
            {(["candidate_page_size", "candidate_font_size"] as const).map(
              (field) => (
                <div key={field}>
                  {row(
                    field,
                    <input
                      id={`input-setting-${field}`}
                      className={`${control} w-20`}
                      type="number"
                      min={field === "candidate_page_size" ? 1 : 12}
                      max={field === "candidate_page_size" ? 9 : 22}
                      step={1}
                      value={values[field]}
                      disabled={locked}
                      onChange={(event) =>
                        change(field, Number(event.target.value))
                      }
                    />,
                  )}
                </div>
              ),
            )}
            {row(
              "menu_icon_variant",
              select(
                "menu_icon_variant",
                ["pearl_16", "pearl_14", "pearl_12", "pearl_18"],
                (value) =>
                  change(
                    "menu_icon_variant",
                    value as InputSettingsValues["menu_icon_variant"],
                  ),
              ),
              "inputSettings.iconRestart",
            )}
            {row(
              "punctuation_preference",
              select(
                "punctuation_preference",
                ["english_in_chinese", "follow_input_mode"],
                (value) =>
                  change(
                    "punctuation_preference",
                    value as InputSettingsValues["punctuation_preference"],
                  ),
              ),
            )}
            {row(
              "character_width_preference",
              select(
                "character_width_preference",
                ["half_width", "full_width"],
                (value) =>
                  change(
                    "character_width_preference",
                    value as InputSettingsValues["character_width_preference"],
                  ),
              ),
            )}
            {toggle(
              "spelling_correction_enabled",
              "inputSettings.spellingHint",
            )}
            {toggle("memory_enabled")}
            {toggle("privacy_learning_enabled", "inputSettings.learningHint")}
            {row(
              "sensitive_bundle_ids",
              <textarea
                id="input-setting-sensitive_bundle_ids"
                className={`${control} min-h-24 w-full font-mono`}
                value={sensitiveText}
                disabled={locked}
                onChange={(event) => {
                  setSensitiveText(event.target.value);
                  change(
                    "sensitive_bundle_ids",
                    event.target.value
                      .split("\n")
                      .map((value) => value.trim())
                      .filter(Boolean),
                  );
                }}
              />,
              "inputSettings.sensitiveHint",
            )}
          </>
        )}
        <div className="space-y-3 p-4">
          {error && (
            <p role="alert" className="text-sm text-red-500">
              {errorKey}
            </p>
          )}
          {!base && !error && (
            <p className="text-sm">{t("inputSettings.loading")}</p>
          )}
          {recordBroken && (
            <button
              className={button}
              disabled={busy}
              onClick={() => void abandonRetry()}
            >
              {t("inputSettings.discardBroken")}
            </button>
          )}
          {pending && !conflict && (
            <div
              role="status"
              className="space-y-2 rounded border border-amber-500/40 p-3 text-sm"
            >
              <p>{t("inputSettings.pending")}</p>
              <div className="flex flex-wrap gap-2">
                <button
                  className={button}
                  disabled={
                    busy ||
                    (!!base &&
                      pending.request.expected_store_id !== base.store_id)
                  }
                  onClick={() => void submit(pending)}
                >
                  {t("inputSettings.retrySame")}
                </button>
                <button
                  className={button}
                  disabled={busy}
                  onClick={() => void abandonRetry()}
                >
                  {t("inputSettings.stopRetry")}
                </button>
              </div>
            </div>
          )}
          {conflict && (
            <div
              role="alert"
              className="space-y-3 rounded border border-amber-500/40 p-3 text-sm"
            >
              <p>
                {t(
                  conflict.status === "conflict"
                    ? "inputSettings.conflict"
                    : "inputSettings.expired",
                )}
              </p>
              <div className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead>
                    <tr>
                      <th>{t("inputSettings.field")}</th>
                      <th>{t("inputSettings.mine")}</th>
                      <th>{t("inputSettings.current")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {fields
                      .filter((field) => attempted && field in attempted)
                      .map((field) => (
                        <tr key={field}>
                          <th className="py-2 font-normal">
                            {t(`inputSettings.fields.${field}`)}
                          </th>
                          <td className="whitespace-pre-wrap px-2">
                            {showValue(field, attempted?.[field])}
                          </td>
                          <td className="whitespace-pre-wrap px-2">
                            {showValue(field, conflict.current.values[field])}
                          </td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
              <button className={button} disabled={busy} onClick={useCurrent}>
                {t("inputSettings.editCurrent")}
              </button>
            </div>
          )}
          {external && (
            <div
              role="region"
              aria-label={t("inputSettings.externalPreview")}
              className="space-y-3 rounded border border-mid-gray/30 p-3 text-sm"
            >
              <p>{t("inputSettings.externalHint")}</p>
              <dl className="grid gap-2 sm:grid-cols-2">
                {fields.map((field) => (
                  <div key={field}>
                    <dt className="font-medium">
                      {t(`inputSettings.fields.${field}`)}
                    </dt>
                    <dd className="whitespace-pre-wrap">
                      {showValue(field, external.values[field])}
                    </dd>
                  </div>
                ))}
              </dl>
              <details>
                <summary className="cursor-pointer">
                  {t("inputSettings.extraValues")}
                </summary>
                <p className="my-2 text-xs text-mid-gray">
                  {t("inputSettings.extraValuesHint")}
                </p>
                <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded bg-mid-gray/10 p-2 text-xs">
                  {JSON.stringify(
                    Object.fromEntries(
                      Object.entries(external.values).filter(
                        ([key]) => !fields.includes(key as InputSettingField),
                      ),
                    ),
                    null,
                    2,
                  )}
                </pre>
              </details>
              <div className="flex flex-wrap gap-2">
                <button
                  className={button}
                  disabled={busy || !!pending || recordBroken}
                  onClick={() =>
                    void submit(createExternalInputSettingsImport(external))
                  }
                >
                  {t("inputSettings.confirmExternal")}
                </button>
                <button
                  className={button}
                  disabled={busy || !!pending}
                  onClick={() => setExternal(undefined)}
                >
                  {t("inputSettings.cancelPreview")}
                </button>
              </div>
            </div>
          )}
          {saved && (
            <p role="status" className="text-sm">
              {t("inputSettings.saved", { revision: saved.revision })}
              {saved.replayed && <> {t("inputSettings.replayed")}</>}
              {base?.revision !== saved.revision && (
                <>
                  {" "}
                  {t("inputSettings.newerCurrent", {
                    revision: base?.revision,
                  })}
                </>
              )}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <button
              className={button}
              disabled={!base || !dirty || locked}
              onClick={() => void save()}
            >
              {t("inputSettings.save")}
            </button>
            <button
              className={button}
              disabled={busy}
              onClick={() => void refresh()}
            >
              {t("inputSettings.refresh")}
            </button>
            {(error === "external_edit" || error === "external_changed") && (
              <button
                className={button}
                disabled={busy || !!pending}
                onClick={() => void previewExternal()}
              >
                {t("inputSettings.previewExternal")}
              </button>
            )}
          </div>
          <p className="text-xs text-mid-gray">
            {t("inputSettings.historyHint")}
          </p>
        </div>
        <div
          role="region"
          aria-label={t("inputSettings.applicationTitle")}
          className="space-y-2 p-4 text-sm"
        >
          <h3 className="font-medium">{t("inputSettings.applicationTitle")}</h3>
          <p className="text-xs text-mid-gray">
            {t("inputSettings.applicationHint")}
          </p>
          {!application || application.sessions.length === 0 ? (
            <p>{t("inputSettings.noSessions")}</p>
          ) : (
            <>
              <p>
                {t("inputSettings.sessionCount", {
                  matching: matchingSessions.length,
                  total: application.sessions.length,
                })}
              </p>
              {application.sessions.map((session, index) => (
                <div
                  key={session.instance_id}
                  className="rounded border border-mid-gray/20 p-2"
                >
                  <p>
                    {t("inputSettings.session", {
                      number: index + 1,
                      revision: session.revision,
                    })}{" "}
                    {t(
                      matchingSessions.includes(session)
                        ? "inputSettings.matches"
                        : "inputSettings.otherVersion",
                    )}
                  </p>
                  {session.applied_fields.length > 0 && (
                    <p>
                      {t("inputSettings.appliedFields", {
                        fields:
                          settingFields(session.applied_fields) ||
                          t("inputSettings.runtimeFields"),
                      })}
                    </p>
                  )}
                  {session.unavailable_fields.length > 0 && (
                    <p>
                      {t("inputSettings.unavailableFields", {
                        fields:
                          settingFields(session.unavailable_fields) ||
                          t("inputSettings.runtimeFields"),
                      })}
                    </p>
                  )}
                </div>
              ))}
            </>
          )}
        </div>
      </SettingsGroup>
    </section>
  );
}
