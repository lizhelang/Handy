import { expect, test, type Page } from "@playwright/test";
import type {
  InputSettingsApplication,
  InputSettingsOperation,
  InputSettingsSnapshot,
  ExternalInputSettings,
} from "../src/lib/inputSettings";

declare global {
  interface Window {
    __INPUT_SETTINGS_TEST__: {
      calls: { action: string; request?: InputSettingsOperation["request"] }[];
      snapshot: InputSettingsSnapshot;
      external: ExternalInputSettings;
      application: InputSettingsApplication;
      mode:
        | "saved"
        | "conflict"
        | "outcome_expired"
        | "commit_uncertain"
        | "throw"
        | "malformed";
      readError: string;
      importError: string;
      applicationError: boolean;
      remount: () => void;
      holdNextSave: boolean;
      completeHeldSave?: () => void;
    };
  }
}

// 只验证真实 React 面板与模拟 IPC 的合同，不代表原生引擎或系统 UI 验收。
async function mount(page: Page) {
  await page.route("**/__input-settings-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__input-settings-test");
  await page.evaluate(async () => {
    const snapshot: InputSettingsSnapshot = {
      store_id: "80000000-0000-4000-8000-000000000001",
      revision: "9007199254740993",
      values_digest: "a".repeat(64),
      values: {
        schema_id: "luna_pinyin_simp",
        candidate_page_size: 7,
        candidate_font_size: 14,
        menu_icon_variant: "pearl_16",
        shift_toggle_enabled: true,
        input_mode_toggle_shortcut: "shift",
        chinese_script: "simplified",
        script_toggle_shortcut: "control_shift_s",
        punctuation_preference: "follow_input_mode",
        character_width_preference: "half_width",
        spelling_correction_enabled: true,
        memory_enabled: true,
        privacy_learning_enabled: true,
        sensitive_bundle_ids: ["com.example.private"],
        rime_user_data_dir: "/synthetic/rime",
        memory_db_path: "/synthetic/memory.sqlite",
        future_extension: { enabled: true, value: "preserve me" },
      },
    };
    const harness: Window["__INPUT_SETTINGS_TEST__"] = {
      calls: [],
      snapshot,
      external: {
        store_id: snapshot.store_id,
        revision: snapshot.revision,
        observed_file_digest: "c".repeat(64),
        values: {
          ...structuredClone(snapshot.values),
          candidate_font_size: 21,
        },
      },
      application: {
        scope: "observed_engine_sessions",
        lease_ms: 2500,
        current_store_id: snapshot.store_id,
        current_revision: snapshot.revision,
        current_values_digest: snapshot.values_digest,
        sessions: [],
      },
      mode: "saved",
      readError: "",
      importError: "",
      applicationError: false,
      remount: () => {},
      holdNextSave: false,
    };
    window.__INPUT_SETTINGS_TEST__ = harness;
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (
          command: string,
          args: {
            request: {
              action: string;
              request?: InputSettingsOperation["request"];
            };
          },
        ) => {
          if (command !== "input_settings_request") return null;
          const request = args.request;
          harness.calls.push(structuredClone(request));
          if (request.action === "read")
            return harness.readError
              ? { ok: false, code: harness.readError }
              : { ok: true, snapshot: structuredClone(harness.snapshot) };
          if (request.action === "inspect_external")
            return { ok: true, external: structuredClone(harness.external) };
          if (request.action === "application_status") {
            if (harness.applicationError)
              throw new Error("synthetic unavailable");
            return {
              ok: true,
              application: structuredClone(harness.application),
            };
          }
          if (request.action === "import_external" && harness.importError)
            return { ok: false, code: harness.importError };
          if (harness.mode === "throw")
            throw new Error("synthetic disconnected after commit");
          if (harness.mode === "commit_uncertain")
            return { ok: false, code: "commit_uncertain" };
          if (harness.mode === "malformed")
            return { ok: true, result: { status: "saved" } };
          if (harness.mode !== "saved")
            return {
              ok: true,
              result: {
                status: harness.mode,
                current: structuredClone(harness.snapshot),
              },
            };
          if (request.request && "patch" in request.request)
            Object.assign(harness.snapshot.values, request.request.patch);
          if (request.action === "import_external")
            harness.snapshot.values = structuredClone(harness.external.values);
          harness.snapshot.revision = (
            BigInt(harness.snapshot.revision) + 1n
          ).toString();
          harness.snapshot.values_digest = "b".repeat(64);
          const reply = {
            ok: true,
            result: {
              status: "saved",
              commit_revision: harness.snapshot.revision,
              replayed: false,
              current: structuredClone(harness.snapshot),
            },
          };
          if (harness.holdNextSave) {
            harness.holdNextSave = false;
            return await new Promise((resolve) => {
              harness.completeHeldSave = () => resolve(reply);
            });
          }
          return reply;
        },
      },
    });
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { default: RefreshRuntime } = await import("/@react-refresh");
    RefreshRuntime.injectIntoGlobalHook(window);
    Object.assign(window, {
      $RefreshReg$: () => {},
      $RefreshSig$: () => (value: unknown) => value,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    const { InputSettingsPanel } = await import(
      "/src/components/settings/general/InputSettingsPanel.tsx"
    );
    let root = ReactDOM.createRoot(document.getElementById("root")!);
    harness.remount = () => {
      root.unmount();
      root = ReactDOM.createRoot(document.getElementById("root")!);
      root.render(React.createElement(InputSettingsPanel));
    };
    root.render(React.createElement(InputSettingsPanel));
  });
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toHaveValue("14");
}
const mutations = (page: Page) =>
  page.evaluate(() =>
    window.__INPUT_SETTINGS_TEST__.calls.filter((call) =>
      ["apply", "import_external"].includes(call.action),
    ),
  );

test("13 preferences preserve extensions and exact revisions; only dirty fields and shortcut alias are sent", async ({
  page,
}) => {
  await mount(page);
  await expect(page.locator('[id^="input-setting-"]')).toHaveCount(13);
  await expect(
    page.getByLabel("Input scheme", { exact: true }).locator("option"),
  ).toHaveCount(9);
  await page
    .getByLabel("Input scheme", { exact: true })
    .selectOption("double_pinyin");
  await expect(
    page.getByLabel("Spelling correction", { exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByLabel("Spelling correction", { exact: true }),
  ).toBeChecked();
  await page
    .getByLabel("Chinese / English shortcut", { exact: true })
    .selectOption("control_space");
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(
    page.getByText("Saved as revision 9007199254740994."),
  ).toBeVisible();
  const calls = await mutations(page);
  expect(calls).toHaveLength(1);
  expect(calls[0].request).toMatchObject({
    expected_revision: "9007199254740993",
    patch: {
      schema_id: "double_pinyin",
      candidate_font_size: 18,
      input_mode_toggle_shortcut: "control_space",
      shift_toggle_enabled: false,
    },
  });
  expect(
    Object.keys((calls[0].request as { patch: object }).patch).sort(),
  ).toEqual([
    "candidate_font_size",
    "input_mode_toggle_shortcut",
    "schema_id",
    "shift_toggle_enabled",
  ]);
  expect(calls[0].request?.operation_id).toContain(
    "v1:80000000-0000-4000-8000-000000000001:9007199254740993:",
  );
  await expect(
    page.getByText("No recent input session confirmation is available."),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => window.__INPUT_SETTINGS_TEST__.snapshot.values.future_extension,
    ),
  ).toEqual({ enabled: true, value: "preserve me" });
});

test("conflict shows both values and needs explicit editing of current revision", async ({
  page,
}) => {
  await mount(page);
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page.evaluate(() => {
    const h = window.__INPUT_SETTINGS_TEST__;
    h.mode = "conflict";
    h.snapshot.values.candidate_font_size = 20;
    h.snapshot.revision = "9007199254740994";
    h.snapshot.values_digest = "d".repeat(64);
  });
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  const row = page.getByRole("row").filter({
    has: page.getByRole("rowheader", { name: "Candidate font size" }),
  });
  await expect(row).toContainText("18");
  await expect(row).toContainText("20");
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toBeDisabled();
  const original = (await mutations(page))[0].request?.operation_id;
  await page
    .getByRole("button", { name: "Discard this draft and edit current values" })
    .click();
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toHaveValue("20");
  expect(await mutations(page)).toHaveLength(1);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "saved";
  });
  await page.getByLabel("Candidate font size", { exact: true }).fill("19");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(
    page.getByText("Saved as revision 9007199254740995."),
  ).toBeVisible();
  const latest = (await mutations(page))[1].request;
  expect(latest?.operation_id).not.toBe(original);
  expect(latest?.expected_revision).toBe("9007199254740994");
});

test("unknown outcome survives remount and retries exactly the original operation", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "throw";
  });
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Retry the same operation" }),
  ).toBeEnabled();
  const original = (await mutations(page))[0];
  await page.evaluate(() => window.__INPUT_SETTINGS_TEST__.remount());
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toHaveValue("18");
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toBeDisabled();
  expect(await mutations(page)).toHaveLength(1);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "saved";
  });
  await page.getByRole("button", { name: "Retry the same operation" }).click();
  await expect(
    page.getByText("Saved as revision 9007199254740994."),
  ).toBeVisible();
  expect((await mutations(page))[1]).toEqual(original);
  expect(
    await page.evaluate(() =>
      localStorage.getItem("inputia.settings.pending.v1"),
    ),
  ).toBeNull();
});

test("external file preview is read only and changed bytes require a new explicit confirmation", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.readError = "external_edit";
  });
  await page
    .getByRole("button", { name: "Reload saved settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Preview external changes" }).click();
  const preview = page.getByRole("region", {
    name: "External settings preview",
  });
  await expect(preview).toContainText("21");
  await preview.getByText("Other values in the file (read only)").click();
  await expect(preview).toContainText("/synthetic/rime");
  await expect(preview).toContainText("future_extension");
  expect(await mutations(page)).toHaveLength(0);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.importError = "external_changed";
  });
  await page
    .getByRole("button", { name: "Confirm this external file" })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "changed after the preview",
  );
  const first = (await mutations(page))[0];
  expect(Object.keys(first.request!).sort()).toEqual([
    "expected_revision",
    "expected_store_id",
    "observed_file_digest",
    "operation_id",
  ]);
  expect(first.request).toMatchObject({ observed_file_digest: "c".repeat(64) });
  await page.getByRole("button", { name: "Stop retrying and reload" }).click();
  await page.evaluate(() => {
    const h = window.__INPUT_SETTINGS_TEST__;
    h.external.observed_file_digest = "d".repeat(64);
    h.importError = "";
  });
  await page.getByRole("button", { name: "Preview external changes" }).click();
  expect(await mutations(page)).toHaveLength(1);
  await page
    .getByRole("button", { name: "Confirm this external file" })
    .click();
  await expect(
    page.getByText("Saved as revision 9007199254740994."),
  ).toBeVisible();
  const second = (await mutations(page))[1];
  expect(second.request?.operation_id).not.toEqual(first.request?.operation_id);
  expect(second.request).toMatchObject({
    observed_file_digest: "d".repeat(64),
  });
});

test("recent confirmations distinguish each session, version and unavailable fields, then expire on failure", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    const h = window.__INPUT_SETTINGS_TEST__;
    const entry = {
      instance_id: "80000000-0000-4000-8000-000000000002",
      store_id: h.snapshot.store_id,
      revision: h.snapshot.revision,
      values_digest: h.snapshot.values_digest,
      applied_fields: ["candidate_font_size", "rime_user_data_dir"],
      unavailable_fields: [],
    };
    h.application.sessions = [
      entry,
      {
        ...entry,
        instance_id: "80000000-0000-4000-8000-000000000003",
        values_digest: "e".repeat(64),
        applied_fields: [],
        unavailable_fields: ["schema_id"],
      },
    ];
  });
  const region = page.getByRole("region", {
    name: "Recent input session confirmations",
  });
  await expect(region).toContainText("1 of 2 observed sessions match");
  await expect(region).toContainText("Matches displayed version");
  await expect(region).toContainText("Different version");
  await expect(region).toContainText(
    "Unavailable in this session: Input scheme",
  );
  await expect(region).toContainText(
    "Confirmed fields: Candidate font size, Other runtime fields",
  );
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.applicationError = true;
  });
  await expect(region).toContainText(
    "No recent input session confirmation is available.",
  );
  await expect(region).not.toContainText("Matches displayed version");
});

test("invalid values never send and malformed save response retains its original request", async ({
  page,
}) => {
  await mount(page);
  await page.getByLabel("Candidate font size", { exact: true }).fill("23");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("allowed ranges");
  expect(await mutations(page)).toHaveLength(0);
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "malformed";
  });
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("could not be verified");
  await expect(
    page.getByRole("button", { name: "Retry the same operation" }),
  ).toBeEnabled();
  expect(await mutations(page)).toHaveLength(1);
});

test("a confirmation with only 100 ms remaining expires without waiting for the next poll", async ({
  page,
}) => {
  await page.clock.install();
  await mount(page);
  await page.evaluate(() => {
    const h = window.__INPUT_SETTINGS_TEST__;
    h.application.lease_ms = 100;
    h.application.sessions = [
      {
        instance_id: "80000000-0000-4000-8000-000000000002",
        remaining_ms: 100,
        store_id: h.snapshot.store_id,
        revision: h.snapshot.revision,
        values_digest: h.snapshot.values_digest,
        applied_fields: ["candidate_font_size"],
        unavailable_fields: [],
      },
    ];
  });
  await page.clock.fastForward(1000);
  const region = page.getByRole("region", {
    name: "Recent input session confirmations",
  });
  await expect(region).toContainText("1 of 1 observed sessions match");
  await page.clock.fastForward(101);
  await expect(region).toContainText(
    "No recent input session confirmation is available.",
  );
});

test("unknown existing scheme is preserved and a damaged retry record blocks writes until explicitly discarded", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.snapshot.values.schema_id = "future_schema";
    localStorage.setItem("inputia.settings.pending.v1", "{broken");
    window.__INPUT_SETTINGS_TEST__.remount();
  });
  await expect(page.getByRole("alert")).toContainText(
    "local retry record is unreadable",
  );
  await expect(page.getByLabel("Input scheme", { exact: true })).toHaveValue(
    "future_schema",
  );
  await expect(
    page.getByRole("button", { name: "Save input preferences", exact: true }),
  ).toBeDisabled();
  expect(await mutations(page)).toHaveLength(0);
  await page
    .getByRole("button", {
      name: "Discard unreadable local retry record and reload",
    })
    .click();
  await expect(page.getByLabel("Input scheme", { exact: true })).toBeEnabled();
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(
    page.getByText("Saved as revision 9007199254740994."),
  ).toBeVisible();
  expect((await mutations(page))[0].request).toMatchObject({
    patch: { candidate_font_size: 18 },
  });
  expect(
    Object.keys(
      ((await mutations(page))[0].request as { patch: object }).patch,
    ),
  ).toEqual(["candidate_font_size"]);
  await expect(page.getByLabel("Input scheme", { exact: true })).toHaveValue(
    "future_schema",
  );
});

test("an unmounted operation's late reply cannot clear a newer pending operation", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.holdNextSave = true;
  });
  await page.getByLabel("Candidate font size", { exact: true }).fill("18");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect.poll(async () => (await mutations(page)).length).toBe(1);
  const first = (await mutations(page))[0];
  await page.evaluate(() => window.__INPUT_SETTINGS_TEST__.remount());
  await page.getByRole("button", { name: "Retry the same operation" }).click();
  await expect(
    page.getByText("Saved as revision 9007199254740995."),
  ).toBeVisible();
  expect((await mutations(page))[1]).toEqual(first);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "commit_uncertain";
  });
  await page.getByLabel("Candidate font size", { exact: true }).fill("19");
  await page
    .getByRole("button", { name: "Save input preferences", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "save outcome is unknown",
  );
  const second = (await mutations(page))[2];
  expect(second.request?.operation_id).not.toBe(first.request?.operation_id);
  await page.evaluate(() =>
    window.__INPUT_SETTINGS_TEST__.completeHeldSave?.(),
  );
  const record = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("inputia.settings.pending.v1")!),
  );
  expect(record.operation).toEqual(second);
  await expect(
    page.getByLabel("Candidate font size", { exact: true }),
  ).toHaveValue("19");
});

test("an unresolved external import retains its original preview and request", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.readError = "external_edit";
  });
  await page
    .getByRole("button", { name: "Reload saved settings", exact: true })
    .click();
  await page.getByRole("button", { name: "Preview external changes" }).click();
  await page.evaluate(() => {
    window.__INPUT_SETTINGS_TEST__.mode = "throw";
  });
  await page
    .getByRole("button", { name: "Confirm this external file" })
    .click();
  await expect(
    page.getByRole("button", { name: "Retry the same operation" }),
  ).toBeEnabled();
  const first = (await mutations(page))[0];
  await page.evaluate(() => {
    const h = window.__INPUT_SETTINGS_TEST__;
    h.external.observed_file_digest = "d".repeat(64);
    h.external.values.candidate_font_size = 22;
  });
  await page
    .getByRole("button", { name: "Reload saved settings", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Preview external changes" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("region", { name: "External settings preview" }),
  ).toContainText("21");
  await page.getByRole("button", { name: "Retry the same operation" }).click();
  await expect.poll(async () => (await mutations(page)).length).toBe(2);
  expect((await mutations(page))[1]).toEqual(first);
  const record = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("inputia.settings.pending.v1")!),
  );
  expect(record.preview.observed_file_digest).toBe("c".repeat(64));
});
