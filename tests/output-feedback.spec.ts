import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __OUTPUT_FEEDBACK_TEST__: {
      send: (state: string) => void;
      opened: boolean;
      ready: boolean;
    };
  }
}

async function mountFeedback(page: Page, historical: boolean) {
  await page.route("**/__output-feedback-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__output-feedback-test");
  await page.evaluate(async (hasHistory) => {
    let sequence = 0;
    let handler = 0;
    const callbacks = new Map<number, (value: unknown) => void>();
    const harness = {
      opened: false,
      ready: false,
      send: (state: string) =>
        callbacks.get(handler)?.({
          event: "voice-output-result",
          id: handler,
          payload: { operation_id: "new-output", state },
        }),
    };
    Object.assign(window, {
      __OUTPUT_FEEDBACK_TEST__: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (value: unknown) => void) => {
          callbacks.set(++sequence, callback);
          return sequence;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args?: Record<string, unknown>) => {
          if (command === "plugin:event|listen") {
            handler = Number(args?.handler);
            harness.ready = true;
            return ++sequence;
          }
          if (command === "plugin:event|unlisten") return;
          if (command === "get_app_settings") return { app_language: "en" };
          if (command === "list_unresolved_unified_outputs")
            return {
              items: hasHistory
                ? [
                    {
                      operation_id: "old-1",
                      item_id: "voice-1",
                      state: "rejected",
                    },
                    {
                      operation_id: "old-2",
                      item_id: "voice-2",
                      state: "rejected",
                    },
                  ]
                : [],
              next_cursor: null,
            };
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
    const { OutputFeedback } = await import(
      "/src/components/history/OutputFeedback.tsx"
    );
    // 与真实组件使用同一个 Vite 模块身份，避免另建通知单例。
    const moduleSource = await fetch(
      "/src/components/history/OutputFeedback.tsx",
    ).then((response) => response.text());
    const sonnerPath = moduleSource.match(/from\s+"([^"]+sonner[^\"]*)"/)?.[1];
    if (!sonnerPath) throw new Error("notification module not found");
    const { Toaster } = await import(sonnerPath);
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(
        React.Fragment,
        null,
        React.createElement(Toaster),
        React.createElement(OutputFeedback, {
          onOpenHistory: () => {
            harness.opened = true;
          },
        }),
      ),
    );
  }, historical);
  await expect
    .poll(() => page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.ready))
    .toBe(true);
}

test("启动只汇总旧结果，不把旧拒绝当成新的持续失败", async ({ page }) => {
  await mountFeedback(page, true);
  await expect(page.getByText(/Earlier outputs need attention/)).toBeVisible();
  await expect(page.getByText(/The output was rejected/)).toHaveCount(0);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(1);
  await page.getByRole("button", { name: "History" }).click();
  await expect
    .poll(() => page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.opened))
    .toBe(true);
});

test("新拒绝仍单独提醒，重复回执只更新同一提醒", async ({ page }) => {
  await mountFeedback(page, true);
  await expect(page.getByText(/Earlier outputs need attention/)).toBeVisible();
  await page.evaluate(() => {
    window.__OUTPUT_FEEDBACK_TEST__.send("rejected");
    window.__OUTPUT_FEEDBACK_TEST__.send("rejected");
  });
  await expect(page.getByText(/The output was rejected/)).toHaveCount(1);
});

test("没有旧结果时不提示，成功回执不冒充失败", async ({ page }) => {
  await mountFeedback(page, false);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("confirmed"));
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
});
