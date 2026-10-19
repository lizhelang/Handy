import { expect, test, type Page } from "@playwright/test";

type Harness = {
  calls: string[];
  dragOk: boolean;
};

declare global {
  interface Window {
    __DRAG_ASSIST__: Harness;
  }
}

async function mountAssist(page: Page, dragOk = true) {
  await page.route("**/__drag-assist", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__drag-assist");
  await page.evaluate(async (dragOk) => {
    const harness: Harness = { calls: [], dragOk };
    Object.assign(window, {
      __DRAG_ASSIST__: harness,
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args?: { action?: string }) => {
          harness.calls.push(`${command}:${args?.action ?? ""}`);
          if (command === "open_inputia_permission_help") return null;
          if (command === "begin_inputia_app_bundle_drag") {
            if (!harness.dragOk) throw new Error("drag_rejected");
            return null;
          }
          return null;
        },
      },
    });
    const { default: RefreshRuntime } = await import("/@react-refresh");
    RefreshRuntime.injectIntoGlobalHook(window);
    Object.assign(window, {
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("zh");
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { AccessibilityDragAssist } = await import(
      "/src/components/onboarding/AccessibilityDragAssist.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(AccessibilityDragAssist, {
        open: true,
        onClose: () => undefined,
      }),
    );
  }, dragOk);
  await expect(
    page.getByRole("dialog", { name: "把 Inputia 加到辅助功能" }),
  ).toBeVisible();
}

test("助手弹窗可打开设置并显示应用", async ({ page }) => {
  await mountAssist(page);
  await page.getByRole("button", { name: "打开设置并显示应用" }).click();
  await expect(page.getByRole("status")).toContainText("已打开设置");
  expect(await page.evaluate(() => window.__DRAG_ASSIST__.calls)).toContain(
    "open_inputia_permission_help:assist",
  );
});

test("直接拖出失败时回退到在 Finder 显示应用", async ({ page }) => {
  await mountAssist(page, false);
  await page
    .getByRole("button", { name: "拖动 Inputia 到系统设置" })
    .dispatchEvent("pointerdown", { button: 0 });
  await expect(page.getByRole("status")).toContainText("已在 Finder 中选中");
  const calls = await page.evaluate(() => window.__DRAG_ASSIST__.calls);
  expect(calls).toContain("begin_inputia_app_bundle_drag:");
  expect(calls).toContain("open_inputia_permission_help:app");
});
