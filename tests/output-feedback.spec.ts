import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __OUTPUT_FEEDBACK_TEST__: {
      send: (
        state: string,
        operationId?: string,
        historySaved?: boolean,
      ) => void;
      opened: boolean;
      ready: boolean;
      queryCalls: number;
      acknowledgements: number;
      releaseBootstrap: () => void;
      remount: () => void;
    };
  }
}

async function mountFeedback(
  page: Page,
  historical = false,
  holdBootstrap = false,
) {
  await page.route("**/__output-feedback-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__output-feedback-test");
  await page.evaluate(
    async ({ historical, holdBootstrap }) => {
      let sequence = 0;
      const callbacks = new Map<number, (value: unknown) => void>();
      const listeners = new Map<number, number>();
      const persisted = historical
        ? [
            { operation_id: "old-1", item_id: "voice-1", state: "rejected" },
            { operation_id: "old-2", item_id: "voice-2", state: "uncertain" },
          ]
        : [];
      const harness = {
        opened: false,
        ready: false,
        queryCalls: 0,
        acknowledgements: 0,
        releaseBootstrap: () => {},
        remount: () => {},
        send: (
          state: string,
          operationId = "new-output",
          historySaved?: boolean,
        ) => {
          const at = persisted.findIndex(
            (item) => item.operation_id === operationId,
          );
          if (at >= 0) persisted.splice(at, 1);
          if (
            historySaved !== false &&
            ["pending_target", "uncertain", "rejected", "dispatched"].includes(
              state,
            )
          ) {
            persisted.push({
              operation_id: operationId,
              item_id: `voice-${operationId}`,
              state: state === "dispatched" ? "uncertain" : state,
            });
          }
          for (const [id, handler] of listeners) {
            callbacks.get(handler)?.({
              event: "voice-output-result",
              id,
              payload: {
                operation_id: operationId,
                state,
                history_saved: historySaved,
              },
            });
          }
        },
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
              const id = ++sequence;
              listeners.set(id, Number(args?.handler));
              harness.ready = true;
              return id;
            }
            if (command === "plugin:event|unlisten") {
              listeners.delete(Number(args?.eventId));
              return;
            }
            if (command === "get_app_settings") return { app_language: "en" };
            if (command === "list_unresolved_unified_outputs") {
              harness.queryCalls += 1;
              const page = { items: [...persisted], next_cursor: null };
              if (holdBootstrap && harness.queryCalls === 1) {
                await new Promise<void>((resolve) => {
                  harness.releaseBootstrap = resolve;
                });
              }
              return page;
            }
            if (command === "acknowledge_unified_output_notice") {
              harness.acknowledgements += 1;
              return;
            }
            throw new Error(`unexpected command: ${command}`);
          },
        },
        $RefreshReg$: () => {},
        $RefreshSig$: () => (type: unknown) => type,
        __vite_plugin_react_preamble_installed__: true,
      });
      const { default: React } = await import(
        "/node_modules/.vite/deps/react.js"
      );
      const { default: ReactDOM } = await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      );
      const { default: i18n } = await import("/src/i18n/index.ts");
      await i18n.changeLanguage("en");
      const { OutputFeedback } = await import(
        "/src/components/history/OutputFeedback.tsx"
      );
      const { UnresolvedOutputNotices } = await import(
        "/src/components/history/UnresolvedOutputNotices.tsx"
      );
      // 与实际组件共用同一个通知模块，避免测试另建通知单例。
      const moduleSource = await fetch(
        "/src/components/history/OutputFeedback.tsx",
      ).then((response) => response.text());
      const sonnerPath = moduleSource.match(
        /from\s+"([^"]+sonner[^\"]*)"/,
      )?.[1];
      if (!sonnerPath) throw new Error("notification module not found");
      const { Toaster } = await import(sonnerPath);
      const root = ReactDOM.createRoot(document.getElementById("root")!);
      let generation = 0;
      const render = () =>
        root.render(
          React.createElement(
            React.Fragment,
            null,
            React.createElement(Toaster),
            React.createElement(OutputFeedback, {
              key: generation,
              onOpenHistory: () => {
                harness.opened = true;
                render();
              },
            }),
            harness.opened
              ? React.createElement(UnresolvedOutputNotices)
              : null,
          ),
        );
      harness.remount = () => {
        generation += 1;
        render();
      };
      render();
    },
    { historical, holdBootstrap },
  );
  await expect
    .poll(() => page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.ready))
    .toBe(true);
  await expect
    .poll(() => page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.queryCalls))
    .toBe(1);
}

const notifications = (page: Page) => page.locator("[data-sonner-toast]");
const closeNotification = (page: Page) =>
  page.locator("[data-close-button]").click();

test("启动只显示一条可关闭摘要，重挂载不重弹旧结果", async ({ page }) => {
  await mountFeedback(page, true);
  await expect(page.getByText(/Earlier outputs need attention/)).toBeVisible();
  await expect(notifications(page)).toHaveCount(1);
  await closeNotification(page);
  await expect(notifications(page)).toHaveCount(0);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.remount());
  await expect(notifications(page)).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.queryCalls),
  ).toBe(1);
});

test("短时间多次故障更新同一提示，关闭后重复回执不再弹出", async ({ page }) => {
  await mountFeedback(page, true);
  await expect(page.getByText(/Earlier outputs need attention/)).toBeVisible();
  await page.evaluate(() => {
    for (let index = 0; index < 20; index += 1) {
      window.__OUTPUT_FEEDBACK_TEST__.send("pending_target", `output-${index}`);
    }
    window.__OUTPUT_FEEDBACK_TEST__.send("rejected", "latest-output");
  });
  await expect(notifications(page)).toHaveCount(1);
  await expect(notifications(page)).toContainText("The output was rejected");
  await closeNotification(page);
  await expect(notifications(page)).toHaveCount(0);
  await page.evaluate(() => {
    window.__OUTPUT_FEEDBACK_TEST__.remount();
    window.__OUTPUT_FEEDBACK_TEST__.send("rejected", "latest-output");
  });
  await expect(notifications(page)).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.acknowledgements),
  ).toBe(0);
});

test("迟到的启动查询和其他结果不能覆盖实时未知状态，历史仍可找到结果", async ({
  page,
}) => {
  await mountFeedback(page, true, true);
  await page.evaluate(() => {
    window.__OUTPUT_FEEDBACK_TEST__.send("uncertain", "unknown-output");
    window.__OUTPUT_FEEDBACK_TEST__.send("pending_target", "another-output");
    window.__OUTPUT_FEEDBACK_TEST__.send("confirmed", "unrelated-success");
    window.__OUTPUT_FEEDBACK_TEST__.releaseBootstrap();
  });
  await expect(notifications(page)).toHaveCount(1);
  await expect(notifications(page)).toContainText(
    "Insertion could not be confirmed",
  );
  await expect(page.getByText(/Earlier outputs need attention/)).toHaveCount(0);
  await page.getByRole("button", { name: "History", exact: true }).click();
  await expect(notifications(page)).toHaveCount(0);
  const section = page.getByRole("region", {
    name: "Outputs needing attention",
  });
  await expect(section.locator("summary")).toContainText(
    "Outputs needing attention",
  );
  await expect(
    section.getByText(/Insertion could not be confirmed/).first(),
  ).toBeHidden();
  await section.locator("summary").click();
  await expect(
    section.getByText(/Insertion could not be confirmed/),
  ).toHaveCount(2);
  await expect(
    section.getByText(/Insertion could not be confirmed/).first(),
  ).toBeVisible();
  await expect(
    section.getByText(/Waiting for a valid input target/),
  ).toBeVisible();
  expect(
    await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.acknowledgements),
  ).toBe(0);
});

test("标准粘贴已派发不冒充失败，真正未知仅由同一操作的确认收起", async ({
  page,
}) => {
  await mountFeedback(page);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("dispatched"));
  await expect(notifications(page)).toHaveCount(0);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("uncertain"));
  await expect(notifications(page)).toContainText(
    "Insertion could not be confirmed",
  );
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("dispatched"));
  await expect(notifications(page)).toContainText(
    "Insertion could not be confirmed",
  );
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send("confirmed", "other"),
  );
  await expect(notifications(page)).toHaveCount(1);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("confirmed"));
  await expect(notifications(page)).toHaveCount(0);
});

test("实时提示限时消失，不自动标已读或重试输出", async ({ page }) => {
  await page.clock.install();
  await mountFeedback(page);
  await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.send("uncertain"));
  await expect(notifications(page)).toHaveCount(1);
  await page.clock.fastForward(11000);
  await expect(notifications(page)).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__OUTPUT_FEEDBACK_TEST__.acknowledgements),
  ).toBe(0);
});

test("没有旧结果时不提示，成功或未知格式回执不冒充失败", async ({ page }) => {
  await mountFeedback(page);
  await page.evaluate(() => {
    window.__OUTPUT_FEEDBACK_TEST__.send("confirmed");
    window.__OUTPUT_FEEDBACK_TEST__.send("invalid-state");
  });
  await expect(notifications(page)).toHaveCount(0);
});

test("历史保存失败时不承诺已保存，不给出无效历史跳转", async ({ page }) => {
  await mountFeedback(page);
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send(
      "pending_target",
      "unsaved-output",
      false,
    ),
  );
  await expect(notifications(page)).toContainText(
    "Text could not be pasted into the active application",
  );
  await expect(notifications(page)).not.toContainText("kept in history");
  await expect(
    page.getByRole("button", { name: "History", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send("uncertain", "unsaved-output", false),
  );
  await expect(notifications(page)).toContainText(
    "Insertion could not be confirmed",
  );
  await expect(notifications(page)).toHaveCount(1);
});

test("已派发但历史保存失败时只说明保存问题，不能淹没真实未知", async ({
  page,
}) => {
  await mountFeedback(page);
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send(
      "dispatched",
      "unsaved-dispatch",
      false,
    ),
  );
  await expect(notifications(page)).toHaveCount(1);
  await expect(notifications(page)).toContainText(
    "This transcription could not be saved to history",
  );
  await expect(notifications(page)).not.toContainText("could not be pasted");
  await expect(notifications(page)).not.toContainText(
    "Insertion could not be confirmed",
  );
  await expect(
    page.getByRole("button", { name: "History", exact: true }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send("uncertain", "unknown-output", true),
  );
  await expect(notifications(page)).toHaveCount(1);
  await expect(notifications(page)).toContainText(
    "Insertion could not be confirmed",
  );
  await expect(notifications(page)).toContainText(
    "This transcription could not be saved to history",
  );
});

test("保存失败与一般输出失败各自说明，不猜测是否发过粘贴", async ({ page }) => {
  await mountFeedback(page);
  await page.evaluate(() =>
    window.__OUTPUT_FEEDBACK_TEST__.send("failed", "failed-output", false),
  );
  await expect(notifications(page)).toContainText(
    "The output failed. Check the target before trying again.",
  );
  await expect(notifications(page)).toContainText(
    "This transcription could not be saved to history",
  );
  await expect(notifications(page)).not.toContainText("could not be pasted");
  await expect(notifications(page)).not.toContainText("kept in history");
});
