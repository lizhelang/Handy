import { expect, test, type Page } from "@playwright/test";
import type { PrivacyOperation } from "../src/lib/privacyOperations";

declare global {
  interface Window {
    __KNOWLEDGE_TEST__: {
      calls: { action: string; payload: Record<string, unknown> }[];
      fail: string;
      clipboard: string;
      privacy: PrivacyOperation[];
      holdPrivacy: boolean;
    };
  }
}

// These tests exercise UI and IPC contracts, not native dialogs or indexing.
async function mountKnowledge(page: Page, app = false) {
  await page.route("**/__knowledge-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__knowledge-test");
  await page.evaluate(async (app) => {
    const harness = {
      calls: [] as { action: string; payload: Record<string, unknown> }[],
      fail: "",
      clipboard: "",
      privacy: [] as PrivacyOperation[],
      holdPrivacy: false,
    };
    let personalization = {
      enabled: true,
      epoch: 1,
      terms_partial: true,
      counts: { terms: 1, events: 2, imports: 0 },
      learned_terms: [{ text: "示例词", score: 3 }],
      backfill_summary: null as null | {
        imported: number;
        skipped: number;
        processed: number;
        source: string;
        has_more: boolean;
        next_cursor: string | null;
        warnings: string[];
      },
    };
    const source = {
      id: "managed",
      name: "Knowledge folder",
      kind: "managed",
      path: "/synthetic/knowledge",
      enabled: true,
      external_access: false,
      status: "ready",
      item_count: 1,
    };
    let sources = [
      source,
      {
        ...source,
        id: "voice",
        name: "Voice records",
        kind: "voice",
        path: "",
        item_count: null,
      },
    ];
    sources.push({
      ...source,
      id: "history:saved_snippet",
      name: "Saved typing",
      kind: "saved_snippet",
      enabled: true,
      external_access: false,
      capture_enabled: false,
      status: "ready",
      item_count: 1,
    });
    const item = {
      id: "item-1",
      revision: "sha256-fixture",
      source_id: "managed",
      title: "Project notes",
      text: "A searchable evidence snippet",
      locator: "/synthetic/knowledge/project.md",
      kind: "file",
    };
    let typedItems = [
      {
        ...item,
        id: "history:typed-1",
        kind: "saved_snippet",
        source_id: "history:saved_snippet",
        title: "Typed evidence",
        text: "Committed typing",
      },
    ];
    Object.assign(window, {
      __KNOWLEDGE_TEST__: harness,
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "macos",
        os_type: "macos",
        family: "unix",
        version: "15.0",
        arch: "aarch64",
        eol: "\n",
        exe_extension: "",
      },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (command: string, args?: Record<string, unknown>) => {
          if (command === "plugin:dialog|open")
            return (args?.options as { directory: boolean })?.directory
              ? "/synthetic/project"
              : ["/synthetic/import.md"];
          if (command === "plugin:clipboard-manager|write_text") {
            harness.clipboard = args?.text as string;
            return;
          }
          if (command !== "knowledge_request") {
            harness.calls.push({ action: command, payload: args ?? {} });
            if (command === "get_app_settings")
              return { onboarding_completed: false };
            if (command === "plugin:app|name") return "Inputia Test";
            if (
              command.includes("check_accessibility_permission") ||
              command.includes("check_microphone_permission")
            )
              return false;
            if (command === "plugin:event|listen") return 1;
            return null;
          }
          const action = args?.action as string;
          const payload = args?.payload as Record<string, unknown>;
          harness.calls.push({ action, payload });
          if (harness.fail === action) throw "synthetic failure";
          if (action === "privacy_status")
            return {
              epoch: harness.privacy.length ? 2 : 1,
              operations: structuredClone(harness.privacy),
            };
          if (action === "privacy_begin") {
            let operation = harness.privacy.find(
              (operation) => operation.operation_id === payload.operation_id,
            );
            if (!operation) {
              operation = {
                operation_id: payload.operation_id as string,
                scope: (
                  payload.scope as { kind: "forget_term" | "clear_learned" }
                ).kind,
                expected_epoch: payload.expected_epoch as number,
                epoch: 2,
                state: harness.holdPrivacy ? "accepted" : "completed",
                domain_receipts: {
                  integration: true,
                  personalization: !harness.holdPrivacy,
                  readers: !harness.holdPrivacy,
                },
                failure: null,
              };
              harness.privacy.push(operation);
            }
            if (!harness.holdPrivacy) {
              personalization.learned_terms = [];
              personalization.counts = { terms: 0, events: 0, imports: 0 };
            }
            return operation;
          }
          if (action.startsWith("personalization_")) {
            if (action === "personalization_enabled")
              personalization.enabled = payload.enabled as boolean;
            if (action === "personalization_backfill")
              personalization.backfill_summary = {
                imported: 2,
                skipped: 1,
                processed: 3,
                source: payload.source as string,
                has_more: !payload.cursor,
                next_cursor: payload.cursor ? null : "next-page-fixture",
                warnings: ["Synthetic source warning"],
              };
            return { ...personalization };
          }
          if (action === "status")
            return { managed_path: source.path, sources };
          if (action === "search")
            return {
              items: [item, ...typedItems].filter(
                (entry) =>
                  (!payload.query ||
                    entry.text.includes(payload.query as string)) &&
                  (!payload.source_id || entry.source_id === payload.source_id),
              ),
              warnings: [],
              has_more: true,
            };
          if (action === "read")
            return {
              ...(typedItems.find((entry) => entry.id === payload.id) ?? item),
              text: "Full evidence text for the selected revision.",
              truncated: true,
            };
          if (action === "typed_capture")
            sources = sources.map((s) =>
              s.kind === "saved_snippet"
                ? { ...s, capture_enabled: payload.enabled as boolean }
                : s,
            );
          if (action === "delete_typed")
            typedItems = typedItems.filter((entry) => entry.id !== payload.id);
          if (action === "clear_typed") typedItems = [];
          if (action === "sync")
            return { warnings: ["Skipped unreadable file"] };
          if (action === "import_files")
            return { partial: true, errors: ["Import permission denied"] };
          if (action === "add_directory")
            sources.push({
              ...source,
              id: "linked",
              kind: "directory",
              name: "Project folder",
              path: payload.path as string,
            });
          if (action === "remove_source")
            sources = sources.filter((s) => s.id !== payload.id);
          if (action === "update_source")
            sources = sources.map((s) =>
              s.id === payload.id
                ? {
                    ...s,
                    enabled: payload.enabled as boolean,
                    external_access: payload.external_access as boolean,
                  }
                : s,
            );
          if (action === "export_connection")
            return {
              prompt:
                "Read /synthetic/skill/SKILL.md and install this skill in your own skill library. Query only allowed sources.",
              skill_path: "/synthetic/skill/SKILL.md",
              cli_path: "/synthetic/inputia-knowledge",
            };
          return null;
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
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    if (app) {
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      useSettingsStore.setState({ isLoading: false });
      const { default: App } = await import("/src/App.tsx");
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(App),
      );
      return;
    }
    const { KnowledgePage } = await import(
      "/src/components/knowledge/KnowledgePage.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(KnowledgePage),
    );
  }, app);
  if (app) return;
  await expect(page.getByText("Project notes", { exact: true })).toBeVisible();
}

test("search and read preserve the selected revision", async ({
  page,
}, testInfo) => {
  await mountKnowledge(page);
  await page.getByRole("searchbox").fill("no match");
  await expect(
    page.getByText(
      "No matching knowledge. Add a source or try another search.",
    ),
  ).toBeVisible();
  await page.getByRole("searchbox").fill("evidence");
  await page.getByRole("button", { name: /Project notes/ }).click();
  await expect(
    page.getByRole("complementary", { name: "Document preview" }),
  ).toContainText("Full evidence text");
  expect(
    await page.evaluate(
      () =>
        window.__KNOWLEDGE_TEST__.calls.find((c) => c.action === "read")
          ?.payload,
    ),
  ).toEqual({ id: "item-1", revision: "sha256-fixture" });
  await page.screenshot({
    path: testInfo.outputPath("knowledge-library.png"),
    fullPage: true,
  });
});

test("folder linking, import, note creation and unlink use the command contracts", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page.getByRole("button", { name: "Link folder", exact: true }).click();
  const linked = page.getByRole("listitem", { name: "Project folder" });
  await expect(linked).toBeVisible();
  await page
    .getByRole("button", { name: "Change knowledge folder", exact: true })
    .click();
  await page.getByRole("button", { name: "Add files", exact: true }).click();
  await page.getByRole("button", { name: "New note", exact: true }).click();
  await page.getByRole("textbox", { name: "Note title" }).fill("Notes");
  await page
    .getByRole("textbox", { name: "Markdown content" })
    .fill("# Evidence\nA note");
  await page.getByRole("button", { name: "Save note" }).click();
  await linked.getByRole("button", { name: "Unlink", exact: true }).click();
  await expect(page.getByText(/Original files will remain/)).toBeVisible();
  await page.getByRole("button", { name: "Confirm unlink" }).click();
  await expect(linked).toHaveCount(0);
  const calls = await page.evaluate(() => window.__KNOWLEDGE_TEST__.calls);
  expect(calls).toContainEqual({
    action: "add_directory",
    payload: { path: "/synthetic/project" },
  });
  expect(calls).toContainEqual({
    action: "set_managed_path",
    payload: { path: "/synthetic/project" },
  });
  expect(calls).toContainEqual({
    action: "import_files",
    payload: { paths: ["/synthetic/import.md"] },
  });
  expect(calls).toContainEqual({
    action: "save_note",
    payload: { title: "Notes", text: "# Evidence\nA note" },
  });
});

test("external permission and connection prompt copying are explicit", async ({
  page,
}) => {
  await mountKnowledge(page);
  const source = page.getByRole("listitem", { name: "Knowledge folder" });
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).not.toBeChecked();
  await source.getByRole("checkbox", { name: "Allow external AI" }).check();
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).toBeChecked();
  await page
    .getByRole("button", { name: "Generate connection prompt" })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Connection prompt" }),
  ).toHaveValue(/SKILL.md/);
  await page.getByRole("button", { name: "Copy prompt" }).click();
  await expect(page.getByText("Prompt copied.", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(() => window.__KNOWLEDGE_TEST__.clipboard),
  ).toContain("your own skill library");
});

test("failed mutations retain source permissions and show a recoverable error", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "update_source";
  });
  const source = page.getByRole("listitem", { name: "Knowledge folder" });
  await source.getByRole("checkbox", { name: "Allow external AI" }).click();
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).not.toBeChecked();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await source.getByRole("checkbox", { name: "Allow external AI" }).check();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).toBeChecked();
});

test("knowledge library bypass does not complete or initialize input onboarding", async ({
  page,
}) => {
  await mountKnowledge(page, true);
  await page
    .getByRole("button", { name: "Use knowledge library only" })
    .click();
  await expect(
    page.getByRole("heading", { name: "Knowledge library", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Back to input setup" }),
  ).toBeVisible();
  const calls = await page.evaluate(() =>
    window.__KNOWLEDGE_TEST__.calls.map((call) => call.action),
  );
  expect(calls).not.toContain("initialize_enigo");
  expect(calls).not.toContain("initialize_shortcuts");
  expect(calls).not.toContain("set_onboarding_completed");
  expect(calls.some((call) => call.includes("download_model"))).toBe(false);
  await page.getByRole("button", { name: "Back to input setup" }).click();
  await expect(
    page.getByRole("button", { name: "Use knowledge library only" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Knowledge library", exact: true }),
  ).toHaveCount(0);
});

test("partial outcomes and bounded result warnings remain visible", async ({
  page,
}) => {
  await mountKnowledge(page);
  await expect(page.getByText("Count unavailable")).toHaveCount(1);
  await expect(
    page
      .getByRole("listitem", { name: "Saved typing" })
      .getByRole("checkbox", { name: "Include in search" }),
  ).toBeEnabled();
  await expect(
    page.getByText(
      "More results are available. Refine your keywords or select a source.",
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: /Project notes/ }).click();
  await expect(
    page.getByText(
      "This preview contains the beginning of a long record. The external query tool can read subsequent pages.",
    ),
  ).toBeVisible();
  await page.getByRole("button", { name: "Refresh index" }).click();
  await expect(page.getByText("Skipped unreadable file")).toBeVisible();
  await page.getByRole("button", { name: "Add files" }).click();
  await expect(page.getByText("Import permission denied")).toBeVisible();
  await expect(
    page.getByText("Some files could not be imported. See the details below."),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "search";
  });
  await page.getByRole("searchbox").fill("error");
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await page.getByRole("searchbox").fill("evidence");
  await expect(page.getByText("Project notes", { exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("typing capture is opt-in and independent from search and external AI", async ({
  page,
}) => {
  await mountKnowledge(page);
  const source = page.getByRole("listitem", { name: "Saved typing" });
  const capture = source.getByRole("checkbox", {
    name: "Automatically capture typing snippets",
  });
  await expect(capture).not.toBeChecked();
  await expect(
    source.getByRole("checkbox", { name: "Include in search" }),
  ).toBeChecked();
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).not.toBeChecked();
  await capture.check();
  await expect(capture).toBeChecked();
  await expect(
    source.getByRole("checkbox", { name: "Allow external AI" }),
  ).not.toBeChecked();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "typed_capture";
  });
  await capture.click();
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await expect(capture).toBeChecked();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await capture.uncheck();
  await expect(capture).not.toBeChecked();
  await expect(
    page.getByRole("button", { name: /Typed evidence/ }),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter(
        (c) => c.action === "update_source",
      ),
    ),
  ).toEqual([]);
});

test("typed deletion confirms, preserves state on failure, and clears the selected preview", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page.getByRole("button", { name: /Typed evidence/ }).click();
  await expect(page.getByRole("complementary")).toContainText("Typed evidence");
  await page
    .getByRole("button", { name: "Delete typing record", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter(
        (c) => c.action === "delete_typed",
      ),
    ),
  ).toEqual([]);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Typed evidence/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Delete typing record", exact: true })
    .click();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "delete_typed";
  });
  await page
    .getByRole("button", { name: "Confirm deletion of typing records" })
    .click();
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await expect(page.getByRole("complementary")).toContainText("Typed evidence");
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await page
    .getByRole("button", { name: "Confirm deletion of typing records" })
    .click();
  await expect(
    page.getByRole("button", { name: /Typed evidence/ }),
  ).toHaveCount(0);
  await expect(page.getByRole("complementary")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /Project notes/ }),
  ).toBeVisible();
});

test("clear typed records requires confirmation and does not alter capture permissions", async ({
  page,
}) => {
  await mountKnowledge(page);
  const capture = page.getByRole("checkbox", {
    name: "Automatically capture typing snippets",
  });
  await capture.check();
  await page
    .getByRole("button", { name: "Clear typing records", exact: true })
    .click();
  await expect(
    page.getByText(/Text in the original apps stays unchanged/),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter((c) => c.action === "clear_typed"),
    ),
  ).toEqual([]);
  await page
    .getByRole("button", { name: "Confirm deletion of typing records" })
    .click();
  await expect(
    page.getByRole("button", { name: /Typed evidence/ }),
  ).toHaveCount(0);
  await expect(capture).toBeChecked();
  await expect(
    page.getByRole("button", { name: /Project notes/ }),
  ).toBeVisible();
});

test("personal learning, text capture and external AI switches remain independent", async ({
  page,
}) => {
  await mountKnowledge(page);
  const learning = page.getByRole("checkbox", {
    name: "Enable Inputia personalization",
  });
  const capture = page.getByRole("checkbox", {
    name: "Automatically capture typing snippets",
  });
  const external = page
    .getByRole("listitem", { name: "Saved typing" })
    .getByRole("checkbox", { name: "Allow external AI" });
  await expect(learning).toBeChecked();
  await expect(
    page.getByText(
      "Term and learning-event counts and the term list include only sources verified in this pass, not all imported records. The total import count is shown separately.",
    ),
  ).toBeVisible();
  await expect(
    page.getByText(/This switch does not control Rime/),
  ).toBeVisible();
  await expect(capture).not.toBeChecked();
  await expect(external).not.toBeChecked();
  await learning.uncheck();
  await expect(learning).not.toBeChecked();
  await capture.check();
  await expect(learning).not.toBeChecked();
  await expect(external).not.toBeChecked();
  await expect(page.getByRole("list", { name: "Learned terms" })).toContainText(
    "示例词",
  );
});

test("backfill sends only explicitly confirmed source and limit and renders exact outcome", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page
    .getByRole("combobox", { name: "Learning source" })
    .selectOption("clipboard");
  await page
    .getByRole("combobox", { name: "Record limit" })
    .selectOption("200");
  await page
    .getByRole("button", { name: "Learn from selected history" })
    .click();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter(
        (c) => c.action === "personalization_backfill",
      ),
    ),
  ).toEqual([]);
  await expect(
    page.getByRole("group", { name: "Confirm learning change" }),
  ).toContainText("Clipboard records");
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  expect(
    await page.evaluate(
      () =>
        window.__KNOWLEDGE_TEST__.calls.find(
          (c) => c.action === "personalization_backfill",
        )?.payload,
    ),
  ).toEqual({ source: "clipboard", limit: 200 });
  await expect(
    page.getByText("Imported this batch: 2 source records"),
  ).toBeVisible();
  await page
    .getByRole("combobox", { name: "Record limit" })
    .selectOption("500");
  await page.getByRole("button", { name: "Learn the next batch" }).click();
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  expect(
    await page.evaluate(
      () =>
        window.__KNOWLEDGE_TEST__.calls
          .filter((c) => c.action === "personalization_backfill")
          .at(-1)?.payload,
    ),
  ).toEqual({ source: "clipboard", limit: 500, cursor: "next-page-fixture" });
  await expect(
    page.getByRole("button", { name: "Learn the next batch" }),
  ).toHaveCount(0);
});

test("forget and clear require confirmation, retain data on failure and refresh on success", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page
    .getByRole("button", { name: "Forget 示例词", exact: true })
    .click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter(
        (c) => c.action === "privacy_begin",
      ),
    ),
  ).toEqual([]);
  await page
    .getByRole("button", { name: "Forget 示例词", exact: true })
    .click();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "privacy_begin";
  });
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await expect(page.getByRole("list", { name: "Learned terms" })).toContainText(
    "示例词",
  );
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  await expect(page.getByText("No learned terms yet.")).toBeVisible();
  await mountKnowledge(page);
  await page
    .getByRole("button", { name: "Clear personalization", exact: true })
    .click();
  expect(
    await page.evaluate(() =>
      window.__KNOWLEDGE_TEST__.calls.filter(
        (c) => c.action === "privacy_begin",
      ),
    ),
  ).toEqual([]);
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "privacy_begin";
  });
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  await expect(page.getByRole("alert")).toContainText("synthetic failure");
  await expect(page.getByRole("list", { name: "Learned terms" })).toContainText(
    "示例词",
  );
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  await expect(
    page.getByRole("group", { name: "Confirm learning change" }),
  ).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("privacy accepted and partial recovery remain visible and timeout retry keeps operation identity", async ({
  page,
}) => {
  await mountKnowledge(page);
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.holdPrivacy = true;
    window.__KNOWLEDGE_TEST__.fail = "privacy_begin";
  });
  await page
    .getByRole("button", { name: "Forget 示例词", exact: true })
    .click();
  await page.getByRole("button", { name: "Confirm learning change" }).click();
  await expect(
    page.getByRole("button", { name: "Retry the same forgetting operation" }),
  ).toBeVisible();
  const first = await page.evaluate(
    () =>
      window.__KNOWLEDGE_TEST__.calls.find(
        (call) => call.action === "privacy_begin",
      )?.payload,
  );
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.fail = "";
  });
  await page
    .getByRole("button", { name: "Retry the same forgetting operation" })
    .click();
  const calls = await page.evaluate(() =>
    window.__KNOWLEDGE_TEST__.calls
      .filter((call) => call.action === "privacy_begin")
      .map((call) => call.payload),
  );
  expect(calls.at(-1)).toEqual(first);
  await expect(
    page.getByText("Forgetting accepted — revocation is durable"),
  ).toBeVisible();
  await expect(page.getByText(/Original history, attachments/)).toBeVisible();
  await page.evaluate(() => {
    const operation = window.__KNOWLEDGE_TEST__.privacy[0];
    operation.state = "partial_failure";
    operation.failure = "personalization_unavailable";
  });
  await expect(
    page.getByText("Partially complete — recovery will retry automatically"),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__KNOWLEDGE_TEST__.privacy[0].failure =
      "privacy_domain_receipt_missing";
  });
  await expect(
    page.getByText(/Completion evidence is missing or inconsistent/),
  ).toBeVisible();

  await page.evaluate(() => {
    const operation = window.__KNOWLEDGE_TEST__.privacy[0];
    operation.state = "completed";
    operation.failure = null;
    operation.domain_receipts.personalization = true;
    operation.domain_receipts.readers = true;
  });
  await expect(
    page.getByText("Learning evidence forgotten; active readers settled"),
  ).toBeVisible();
});
