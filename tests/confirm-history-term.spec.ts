import { expect, test } from "@playwright/test";

test("term confirmation is opt-in and retries the same operation", async ({
  page,
}) => {
  await page.route("**/__confirm-term", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__confirm-term");
  await page.evaluate(async () => {
    const calls: unknown[] = [];
    Object.assign(window, {
      termCalls: calls,
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: unknown) => {
          if (command !== "confirm_unified_history_term") return {};
          calls.push(args);
          if (calls.length === 1) throw new Error("uncertain");
          return "replay";
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
    await i18n.changeLanguage("zh");
    const { ConfirmHistoryTerm } = await import(
      "/src/components/history/ConfirmHistoryTerm.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(ConfirmHistoryTerm, {
        itemId: "synthetic-item",
        revision: 3,
      }),
    );
  });
  await page.getByText("确认一个术语", { exact: true }).click();
  const save = page.getByRole("button", { name: "确认并保存" });
  await expect(page.getByRole("textbox")).toHaveValue("");
  await expect(save).toBeDisabled();
  await page.getByRole("textbox").fill("Inputia");
  await expect(save).toBeDisabled();
  await page.getByRole("checkbox").check();
  await save.click();
  await expect(page.getByRole("alert")).toBeVisible();
  await save.click();
  await expect(page.getByRole("status")).toContainText("这次确认操作已处理");
  const calls = await page.evaluate(() => Reflect.get(window, "termCalls"));
  expect(calls).toHaveLength(2);
  expect(calls[0]).toEqual(calls[1]);
  expect(calls[0]).toMatchObject({
    itemId: "synthetic-item",
    expectedRevision: 3,
    term: "Inputia",
    confirmed: true,
  });
  await expect(save).toBeDisabled();
  await page.getByRole("textbox").fill("Inputia Pro");
  await expect(page.getByRole("checkbox")).not.toBeChecked();
  await expect(save).toBeDisabled();
  await page.screenshot({
    path: "test-results/confirm-history-term.png",
    fullPage: true,
  });
});
