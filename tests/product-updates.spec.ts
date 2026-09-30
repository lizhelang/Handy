import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __UPDATE_TEST__: {
      calls: string[];
      status: string;
      reject: boolean;
      defer: boolean;
      resolve: () => void;
      setEnabled: (value: boolean) => void;
      emit: () => void;
    };
  }
}
async function mount(page: Page) {
  await page.route("**/__update-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__update-test");
  await page.evaluate(async () => {
    let sequence = 0;
    let running = false;
    const callbacks = new Map<number, (v: unknown) => void>();
    let listener: number | undefined;
    const harness = {
      calls: [] as string[],
      status: "unavailable",
      reject: false,
      defer: false,
      resolve: () => {},
      setEnabled: (_: boolean) => {},
      emit: () => {},
    };
    Object.assign(window, {
      __UPDATE_TEST__: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (v: unknown) => void) => {
          callbacks.set(++sequence, callback);
          return sequence;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args?: Record<string, unknown>) => {
          harness.calls.push(command);
          if (command === "get_app_settings") return { app_language: "en" };
          if (command === "plugin:event|listen") {
            listener = args?.handler as number;
            return ++sequence;
          }
          if (command === "plugin:event|unlisten") return;
          if (command !== "check_product_update")
            throw new Error("unexpected update path");
          const reply = (status: string) => ({
            status,
            reason: status === "unavailable" ? "source_unconfigured" : null,
            version: "1.1.1",
            release_id: "inputia-fixture",
            installable: false,
          });
          if (running) return reply("checking");
          running = true;
          try {
            if (harness.defer)
              await new Promise<void>((resolve) => {
                harness.resolve = resolve;
              });
            if (harness.reject) throw new Error("offline");
            return reply(harness.status);
          } finally {
            running = false;
          }
        },
      },
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    // @ts-expect-error Vite 浏览器模块
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    // @ts-expect-error Vite 浏览器模块
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    // @ts-expect-error Vite 浏览器模块
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    // @ts-expect-error Vite 浏览器模块
    const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
    useSettingsStore.setState({
      settings: { update_checks_enabled: false },
      isLoading: false,
      updateChecksLocked: false,
    });
    harness.setEnabled = (enabled) =>
      useSettingsStore.setState({
        settings: { update_checks_enabled: enabled },
      });
    harness.emit = () => {
      if (listener)
        callbacks.get(listener)?.({
          event: "check-for-updates",
          payload: null,
        });
    };
    // @ts-expect-error Vite 浏览器模块
    const { default: UpdateChecker } = await import(
      "/src/components/update-checker/UpdateChecker.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")).render(
      React.createElement(
        React.StrictMode,
        null,
        React.createElement(UpdateChecker),
      ),
    );
  });
  await expect(page.getByRole("button")).toBeDisabled();
}
const count = (page: Page) =>
  page.evaluate(
    () =>
      window.__UPDATE_TEST__.calls.filter((v) => v === "check_product_update")
        .length,
  );

test("未配置和离线都不显示已经最新，可通过同一入口重试", async ({ page }) => {
  await mount(page);
  expect(await count(page)).toBe(0);
  await page.evaluate(() => window.__UPDATE_TEST__.setEnabled(true));
  await expect(page.getByRole("status")).toContainText(
    "no Inputia update source",
  );
  await page.evaluate(() => {
    window.__UPDATE_TEST__.status = "current";
  });
  await page.getByRole("button").click();
  await expect(page.getByRole("status")).toContainText("up to date", {
    ignoreCase: true,
  });
  await page.evaluate(() => {
    window.__UPDATE_TEST__.reject = true;
  });
  await page.getByRole("button").click();
  await expect(page.getByRole("status")).toContainText("Unable to check");
});

test("禁用丢弃旧结果，重开跟进正在结束的请求，不永久停在busy", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__UPDATE_TEST__.defer = true;
    window.__UPDATE_TEST__.status = "current";
    window.__UPDATE_TEST__.setEnabled(true);
  });
  await expect.poll(() => count(page)).toBe(1);
  await page.evaluate(() => window.__UPDATE_TEST__.setEnabled(false));
  await expect(page.getByRole("button")).toBeDisabled();
  await page.evaluate(() => window.__UPDATE_TEST__.setEnabled(true));
  await expect(page.getByRole("status")).toContainText("already running");
  await page.evaluate(() => {
    window.__UPDATE_TEST__.resolve();
  });
  await expect(page.getByRole("status")).not.toContainText("up to date", {
    ignoreCase: true,
  });
  await page.evaluate(() => {
    window.__UPDATE_TEST__.defer = false;
    window.__UPDATE_TEST__.status = "available";
  });
  await expect(page.getByRole("status")).toContainText("1.1.1", {
    timeout: 6000,
  });
});

test("菜单与按钮合并在同一个进行中查询，发现版本不会调用单应用安装", async ({
  page,
}) => {
  await mount(page);
  await page.evaluate(() => {
    window.__UPDATE_TEST__.defer = true;
    window.__UPDATE_TEST__.status = "available";
    window.__UPDATE_TEST__.setEnabled(true);
  });
  await expect.poll(() => count(page)).toBe(1);
  await page.evaluate(() => {
    window.__UPDATE_TEST__.emit();
    window.__UPDATE_TEST__.emit();
  });
  expect(await count(page)).toBe(1);
  await page.evaluate(() => window.__UPDATE_TEST__.resolve());
  await expect(page.getByRole("status")).toContainText("not yet supported");
  const calls = await page.evaluate(() => window.__UPDATE_TEST__.calls);
  expect(
    calls.some((command) => /updater|download|install|relaunch/.test(command)),
  ).toBe(false);
});

test("忙碌重查达到上限会明确失败，手动点击可开启新一轮", async ({ page }) => {
  await page.clock.install();
  await mount(page);
  await page.evaluate(() => {
    window.__UPDATE_TEST__.status = "checking";
    window.__UPDATE_TEST__.setEnabled(true);
  });
  await expect(page.getByRole("status")).toContainText("already running");
  await page.clock.runFor(122_000);
  await expect(page.getByRole("status")).toContainText("Unable to check");
  await page.evaluate(() => {
    window.__UPDATE_TEST__.status = "available";
  });
  await page.getByRole("button").click();
  await expect(page.getByRole("status")).toContainText("1.1.1");
});
