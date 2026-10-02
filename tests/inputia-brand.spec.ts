import { expect, test } from "@playwright/test";

// 使用真实品牌组件与CSS；这里只验证视觉布局，不替代原生语音/插入验收。
for (const theme of ["light", "dark"] as const) {
  test(`Inputia wordmark stays legible in a narrow ${theme} sidebar`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 360, height: 180 });
    await page.route("**/__inputia-brand-test", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
      }),
    );
    await page.goto("/__inputia-brand-test");
    await page.evaluate(async (theme) => {
      document.documentElement.dataset.theme = theme;
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
      const { default: Wordmark } = await import(
        "/src/components/icons/InputiaWordmark.tsx"
      );
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(
          "div",
          { className: "flex w-40 flex-col items-center px-2" },
          React.createElement(Wordmark, { className: "m-4" }),
        ),
      );
    }, theme);
    const image = page.locator("[data-inputia-mark]");
    await expect(image).toBeVisible();
    await expect(page.getByText("Inputia", { exact: true })).toBeVisible();
    await expect(page.getByText("Inputia", { exact: true })).toHaveCSS(
      "color",
      theme === "dark" ? "rgb(251, 251, 251)" : "rgb(15, 15, 15)",
    );
    const geometry = await image.boundingBox();
    expect(geometry!.width).toBeGreaterThanOrEqual(30);
    expect(Math.abs(geometry!.width - geometry!.height)).toBeLessThan(1);
    const accent = await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue("--color-background-ui")
        .trim()
        .toLowerCase(),
    );
    expect(accent).toBe("#2f6f73");
    await page.waitForLoadState("networkidle");
    await expect(image.locator("circle")).toHaveCount(1);
    await expect(image.locator("path")).toHaveCount(1);
    await expect(page.getByText(/handy/i)).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath(`inputia-${theme}.png`),
    });
  });
}
