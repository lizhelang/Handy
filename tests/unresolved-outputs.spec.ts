import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __OUTPUT_NOTICE_TEST__: {
      calls: { command: string; args?: Record<string, unknown> }[];
      rejectAck: boolean;
      malformed: boolean;
      remount: () => void;
    };
  }
}

// 仅验证 UI 与命令合同；不证明原生输入或真正的数据库持久化。
async function mountNotices(page: Page) {
  await page.route("**/__output-notice-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__output-notice-test");
  await page.evaluate(async () => {
    let sequence = 0;
    const callbacks = new Map<number, (value: unknown) => void>();
    const persisted = [
      { operation_id: "op-1", item_id: "voice-1", state: "uncertain" },
      { operation_id: "op-2", item_id: "voice-2", state: "pending_target" },
    ];
    const harness = {
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      rejectAck: false,
      malformed: false,
      remount: () => {},
    };
    Object.assign(window, {
      __OUTPUT_NOTICE_TEST__: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (value: unknown) => void) => {
          callbacks.set(++sequence, callback);
          return sequence;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args?: Record<string, unknown>) => {
          harness.calls.push({ command, args });
          if (command === "plugin:event|listen") return ++sequence;
          if (command === "plugin:event|unlisten") return;
          if (command === "get_app_settings") return { app_language: "en" };
          if (command === "list_unresolved_unified_outputs") {
            if (harness.malformed)
              return { items: [{ state: "confirmed" }], next_cursor: null };
            const start = args?.cursor
              ? persisted.findIndex(
                  (item) => item.operation_id === args.cursor,
                ) + 1
              : 0;
            const items = persisted.slice(start, start + 1);
            return {
              items,
              next_cursor:
                start + 1 < persisted.length ? items[0].operation_id : null,
            };
          }
          if (command === "acknowledge_unified_output_notice") {
            if (harness.rejectAck) throw new Error("state changed");
            const at = persisted.findIndex(
              (item) =>
                item.operation_id === args?.operationId &&
                item.state === args?.expectedState,
            );
            if (at < 0) throw new Error("state changed");
            persisted.splice(at, 1);
            return true;
          }
          throw new Error("unexpected command");
        },
      },
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    // @ts-expect-error Vite 浏览器测试动态模块。
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    // @ts-expect-error Vite 浏览器测试动态模块。
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    // @ts-expect-error Vite 浏览器测试动态模块。
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    // @ts-expect-error Vite 浏览器测试动态模块。
    const { UnresolvedOutputNotices } = await import(
      "/src/components/history/UnresolvedOutputNotices.tsx"
    );
    let root = ReactDOM.createRoot(document.getElementById("root")!);
    const render = () =>
      root.render(React.createElement(UnresolvedOutputNotices));
    harness.remount = () => {
      root.unmount();
      root = ReactDOM.createRoot(document.getElementById("root")!);
      render();
    };
    render();
  });
}

test("重建界面仍查询持久结果，分页后确认只标已读", async ({ page }) => {
  await mountNotices(page);
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
  await page.evaluate(() => window.__OUTPUT_NOTICE_TEST__.remount());
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(
    page.getByText(/Waiting for a valid input target/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Dismiss reminder" }).first().click();
  await expect(page.getByText(/Insertion could not be confirmed/)).toHaveCount(
    0,
  );
  const calls = await page.evaluate(() => window.__OUTPUT_NOTICE_TEST__.calls);
  expect(
    calls.find((call) => call.command === "acknowledge_unified_output_notice")
      ?.args,
  ).toEqual({ operationId: "op-1", expectedState: "uncertain" });
  expect(
    calls.filter(
      (call) =>
        [
          "plugin:event|listen",
          "plugin:event|unlisten",
          "get_app_settings",
          "list_unresolved_unified_outputs",
          "acknowledge_unified_output_notice",
        ].includes(call.command) === false,
    ),
  ).toEqual([]);
});

test("确认失败保留提醒，可重试且不派发文本", async ({ page }) => {
  await mountNotices(page);
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__OUTPUT_NOTICE_TEST__.rejectAck = true;
  });
  await page.getByRole("button", { name: "Dismiss reminder" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__OUTPUT_NOTICE_TEST__.rejectAck = false;
  });
  await page.getByRole("button", { name: "Dismiss reminder" }).click();
  await expect(page.getByText(/Insertion could not be confirmed/)).toHaveCount(
    0,
  );
});

test("损坏响应显示重试错误，不能当作没有待处理结果", async ({ page }) => {
  await mountNotices(page);
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
  await page.evaluate(() => {
    window.__OUTPUT_NOTICE_TEST__.malformed = true;
    window.__OUTPUT_NOTICE_TEST__.remount();
  });
  await expect(page.getByRole("alert")).toHaveText("Could not load history.");
  await page.evaluate(() => {
    window.__OUTPUT_NOTICE_TEST__.malformed = false;
  });
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(
    page.getByText(/Insertion could not be confirmed/),
  ).toBeVisible();
});
