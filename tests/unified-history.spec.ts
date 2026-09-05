import { expect, test, type Page } from "@playwright/test";
import type { UnifiedHistoryItem } from "../src/bindings";

interface HistoryHarness {
  calls: { command: string; args?: Record<string, unknown> }[];
  actions: { action: string; item: UnifiedHistoryItem; patch?: unknown }[];
  items: UnifiedHistoryItem[];
  output: string;
  rejectMutation: boolean;
  rejectListener: boolean;
  pending: Map<string, (value: unknown) => void>;
  deferQueries: boolean;
  emit: () => void;
  listeners: Map<number, number>;
  unmount: () => void;
}
declare global {
  interface Window {
    __HISTORY_TEST__: HistoryHarness;
  }
}

// Browser mocks verify UI behavior only; they do not prove native insertion.
async function mountHistory(page: Page) {
  await page.route("**/__unified-history-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root" style="height:95vh;padding:24px"></div></body></html>',
    }),
  );
  await page.goto("/__unified-history-test");
  await page.evaluate(async () => {
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<number, number>();
    let next = 0;
    const fixture: UnifiedHistoryItem = {
      item_id: "voice-1",
      store_id: "voice-store",
      record_id: "1",
      revision: 1,
      source_kind: "voice",
      content_type: "text",
      text: "Voice fixture",
      title: null,
      starred: false,
      pinned: false,
      created_at_ms: 1000000,
      asset_ref: null,
      source_app: null,
    };
    const harness = {
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      actions: [] as {
        action: string;
        item: typeof fixture;
        patch?: unknown;
      }[],
      items: [
        fixture,
        {
          ...fixture,
          item_id: "file-2",
          record_id: "2",
          source_kind: "clipboard",
          content_type: "files",
          text: '["/synthetic/report.pdf"]',
        },
      ],
      output: "confirmed",
      rejectMutation: false,
      rejectListener: false,
      pending: new Map<string, (value: unknown) => void>(),
      deferQueries: false,
      emit: () => {
        for (const [id, handler] of listeners)
          callbacks.get(handler)?.({
            event: "unified-history-update",
            id,
            payload: { generation: 2 },
          });
      },
      listeners,
    };
    Object.assign(window, {
      __HISTORY_TEST__: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: {
        unregisterListener: (_event: string, id: number) =>
          listeners.delete(id),
      },
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (value: unknown) => void) => {
          const id = ++next;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args?: Record<string, unknown>) => {
          harness.calls.push({ command, args });
          if (command === "plugin:event|listen") {
            if (harness.rejectListener) throw "listener unavailable";
            const id = ++next;
            listeners.set(id, Number(args?.handler));
            return id;
          }
          if (command === "plugin:event|unlisten") {
            listeners.delete(Number(args?.eventId));
            return;
          }
          if (command === "refresh_unified_history") return 2;
          if (command === "get_unified_history_revisions")
            return [
              { revision: 1, text: "Original revision", asset_ref: null },
            ];
          if (command === "get_unified_history") {
            const query = args?.query as {
              search: string | null;
              source_kind: string | null;
              content_type: string | null;
              starred_only: boolean;
              offset: number;
              limit: number;
            };
            if (harness.deferQueries)
              return new Promise((resolve) =>
                harness.pending.set(query.search || "", resolve),
              );
            return harness.items
              .filter(
                (item) =>
                  (!query.search || item.text?.includes(query.search)) &&
                  (!query.source_kind ||
                    item.source_kind === query.source_kind) &&
                  (!query.content_type ||
                    item.content_type === query.content_type) &&
                  (!query.starred_only || item.starred),
              )
              .slice(query.offset, query.offset + query.limit);
          }
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
    // Vite's refresh preamble is needed when loading a TSX module in this isolated harness.
    const { default: RefreshRuntime } = await import("/@react-refresh");
    RefreshRuntime.injectIntoGlobalHook(window);
    Object.assign(window, {
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    const { UnifiedHistory } = await import(
      "/src/components/history/UnifiedHistory.tsx"
    );
    const root = ReactDOM.createRoot(document.getElementById("root")!);
    Object.assign(harness, { unmount: () => root.unmount() });
    root.render(
      React.createElement(UnifiedHistory, {
        resolveAsset: async () => null,
        onCopy: async (item: typeof fixture) => {
          harness.actions.push({ action: "copy", item });
          return { status: harness.output };
        },
        onInsert: async (item: typeof fixture) => {
          harness.actions.push({ action: "insert", item });
          if (harness.output === "throw") throw new Error("response lost");
          return { status: harness.output };
        },
        onUpdate: async (item: typeof fixture, patch: unknown) => {
          harness.actions.push({ action: "update", item, patch });
          if (harness.rejectMutation) throw new Error("conflict");
          harness.items = harness.items.map((entry) =>
            entry.item_id === item.item_id
              ? { ...entry, ...(patch as object), revision: entry.revision + 1 }
              : entry,
          );
        },
      }),
    );
  });
  await expect(page.getByText("Voice fixture", { exact: true })).toBeVisible();
}

test("single click previews; explicit copy preserves the file item and does not insert", async ({
  page,
}, testInfo) => {
  await mountHistory(page);
  await page.getByText("report.pdf", { exact: true }).click();
  await expect(
    page.getByRole("complementary", { name: "Preview" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  await expect(page.getByText("Copied.", { exact: true })).toBeVisible();
  const screenshot = testInfo.outputPath("unified-history-preview.png");
  await page.screenshot({ path: screenshot });
  await testInfo.attach(
    "Browser interaction preview (not native insertion evidence)",
    { path: screenshot, contentType: "image/png" },
  );
  expect(
    await page.evaluate(() =>
      window.__HISTORY_TEST__.actions.map((entry) => [
        entry.action,
        entry.item.content_type,
      ]),
    ),
  ).toEqual([["copy", "files"]]);
});

test("composition and search letters are not intercepted; Enter inserts and Escape closes preview", async ({
  page,
}) => {
  await mountHistory(page);
  await page.getByText("Voice fixture", { exact: true }).click();
  const search = page.getByRole("searchbox");
  await search.dispatchEvent("keydown", { key: "Enter", isComposing: true });
  await search.dispatchEvent("keydown", { key: "j" });
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
  await search.press("Enter");
  await expect(page.getByText("Inserted.", { exact: true })).toBeVisible();
  await search.press("Escape");
  await expect(
    page.getByRole("complementary", { name: "Preview" }),
  ).toHaveCount(0);
});

for (const status of [
  "pending_target",
  "uncertain",
  "rejected",
  "failed",
  "dispatched",
]) {
  test(`output ${status} is not presented as success`, async ({ page }) => {
    await mountHistory(page);
    await page.evaluate((value) => {
      window.__HISTORY_TEST__.output = value;
    }, status);
    await page.getByText("Voice fixture", { exact: true }).dblclick();
    await expect(page.getByText("Inserted.", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("status")).toBeVisible();
    if (status === "uncertain" || status === "dispatched") {
      await expect(
        page.getByRole("button", { name: "Insert", exact: true }),
      ).toBeDisabled();
      await page.getByRole("searchbox").press("Enter");
      expect(
        await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
      ).toBe(1);
    }
  });
}

test("a lost insertion response requires explicit acknowledgement before another action", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "throw";
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  await page.getByRole("searchbox").press("Enter");
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page
    .getByRole("button", {
      name: "I checked the result; allow another insertion",
      exact: true,
    })
    .click();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "confirmed";
  });
  await page.getByRole("button", { name: "Insert", exact: true }).click();
  await expect(page.getByText("Inserted.", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(2);
});

test("stale search replies cannot replace newer results", async ({ page }) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.deferQueries = true;
  });
  await page.getByRole("searchbox").fill("old");
  await page.getByRole("searchbox").fill("new");
  await expect
    .poll(() => page.evaluate(() => window.__HISTORY_TEST__.pending.has("new")))
    .toBe(true);
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.pending.get("new")([{ ...h.items[0], text: "New result" }]);
  });
  await expect(page.getByText("New result", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.pending.get("old")([{ ...h.items[0], text: "Stale result" }]);
  });
  await expect(page.getByText("Stale result", { exact: true })).toHaveCount(0);
  await expect(page.getByText("New result", { exact: true })).toBeVisible();
});

test("live changes preserve edit revision and failed mutations do not report success", async ({
  page,
}) => {
  await mountHistory(page);
  await page.getByText("Voice fixture", { exact: true }).click();
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Text", exact: true })
    .fill("My correction");
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.items[0] = {
      ...h.items[0],
      revision: 2,
      text: "Other window correction",
    };
    h.rejectMutation = true;
    h.emit();
  });
  await expect(
    page.getByText("Other window correction", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText(/Could not save/)).toBeVisible();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions[0].item.revision),
  ).toBe(1);
  await expect(
    page.getByText("Saved and synchronized.", { exact: true }),
  ).toHaveCount(0);
});

test("source/type/favorites filters and revision view use their real command contracts", async ({
  page,
}) => {
  await mountHistory(page);
  await page
    .getByRole("combobox", { name: "Source", exact: true })
    .selectOption("clipboard");
  await expect(page.getByText("Voice fixture", { exact: true })).toHaveCount(0);
  await page
    .getByRole("combobox", { name: "Content type" })
    .selectOption("files");
  await page.getByText("report.pdf", { exact: true }).click();
  await page.getByText("Revisions", { exact: true }).click();
  await expect(
    page.getByText("Original revision", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Add to favorites" }).click();
  await expect(
    page.getByText("Saved and synchronized.", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Favorites", exact: true }).click();
  await expect(page.getByText("report.pdf", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    window.__HISTORY_TEST__.unmount();
  });
  await expect
    .poll(() => page.evaluate(() => window.__HISTORY_TEST__.listeners.size))
    .toBe(0);
});

test("pagination retains unique results and requests the next offset", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.items = Array.from({ length: 45 }, (_, index) => ({
      ...h.items[0],
      item_id: `item-${index}`,
      text: `Entry ${index}`,
    }));
    h.emit();
  });
  await expect(
    page.getByRole("list", { name: "History entries" }).locator("li"),
  ).toHaveCount(40);
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(
    page.getByRole("list", { name: "History entries" }).locator("li"),
  ).toHaveCount(45);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
});

test("late listener registration is disposed and failed registration can be retried", async ({
  page,
}) => {
  await mountHistory(page);
  const result = await page.evaluate(async () => {
    const { createUnifiedHistoryStore } = await import(
      "/src/stores/unifiedHistoryStore.ts"
    );
    const h = window.__HISTORY_TEST__;
    const baseline = h.listeners.size;
    const store = createUnifiedHistoryStore();
    const dispose = store.getState().subscribe();
    dispose();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const afterDispose = h.listeners.size;
    h.rejectListener = true;
    const secondDispose = store.getState().subscribe();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const failed = store.getState().subscriptionError;
    h.rejectListener = false;
    await store.getState().refresh();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const recovered =
      !store.getState().subscriptionError && h.listeners.size === baseline + 1;
    secondDispose();
    return { baseline, afterDispose, failed, recovered };
  });
  expect(result.afterDispose).toBe(result.baseline);
  expect(result.failed).toBe(true);
  expect(result.recovered).toBe(true);
});
