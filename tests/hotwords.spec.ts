import { expect, test } from "@playwright/test";

// 使用真实设置存储和命令路由，浏览器 mock 不替代原生候选/语音验收。
test("hotwords is the second sidebar entry and edits existing custom_words", async ({
  page,
}) => {
  await page.setViewportSize({ width: 720, height: 600 });
  await page.route("**/__hotwords-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__hotwords-test");
  await page.evaluate(async () => {
    const calls: Array<{ cmd: string; args: unknown }> = [];
    Object.assign(window, {
      __HOTWORDS_CALLS__: calls,
      __TAURI_INTERNALS__: {
        transformCallback: () => 1,
        unregisterCallback: () => null,
        invoke: async (cmd: string, args: unknown) => {
          calls.push({ cmd, args });
          return null;
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => null },
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
    const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
    useSettingsStore.setState({
      settings: {
        custom_words: ["原有术语"],
        post_process_enabled: false,
        debug_mode: false,
      },
      isLoading: false,
      initialize: async () => undefined,
    });
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { Sidebar, SECTIONS_CONFIG } = await import(
      "/src/components/Sidebar.tsx"
    );
    function Harness() {
      const [section, setSection] = React.useState("general");
      const Active = SECTIONS_CONFIG.hotwords.component;
      return React.createElement(
        "div",
        { className: "flex h-screen" },
        React.createElement(Sidebar, {
          activeSection: section,
          onSectionChange: setSection,
        }),
        React.createElement(
          "main",
          { className: "min-w-0 flex-1 p-4" },
          section === "hotwords" ? React.createElement(Active) : null,
        ),
      );
    }
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(Harness),
    );
  });
  const entry = page.getByText("热词", { exact: true });
  await expect(entry).toBeVisible();
  const labels = await entry.locator("../../..").locator("p").allTextContents();
  expect(labels.slice(0, 2)).toEqual(["通用", "热词"]);
  await entry.click();
  const region = page.getByRole("region", { name: "热词" });
  await expect(
    region.getByRole("button", { name: "删除 原有术语" }),
  ).toBeVisible();
  await expect(
    region.getByText(/英文开头的热词输入恰好前三个字母/),
  ).toBeVisible();
  await expect(
    region.getByText(/前两个字的完整全拼或自然码双拼编码/),
  ).toBeVisible();
  await expect(region.getByText(/继续输入会撤下前缀补全/)).toBeVisible();
  await expect(
    region.getByText(
      /语音识别优先参考热词，实际结果仍取决于上下文、发音及所用模型/,
    ),
  ).toBeVisible();
  await region.getByRole("textbox", { name: "输入热词" }).fill(" 新术语 ");
  await region
    .getByRole("textbox", { name: "输入热词" })
    .dispatchEvent("keydown", { key: "Enter", isComposing: true });
  await expect(region.getByRole("button", { name: "删除 新术语" })).toHaveCount(
    0,
  );
  await region.getByRole("textbox", { name: "输入热词" }).press("Enter");
  await expect(
    region.getByRole("button", { name: "删除 新术语" }),
  ).toBeVisible();
  await expect(
    region.getByRole("button", { name: "删除 原有术语" }),
  ).toBeVisible();
  await region.getByRole("button", { name: "删除 新术语" }).click();
  const calls = await page.evaluate(() =>
    (
      window as unknown as {
        __HOTWORDS_CALLS__: Array<{ cmd: string; args: { words: string[] } }>;
      }
    ).__HOTWORDS_CALLS__.filter((call) => call.cmd === "update_custom_words"),
  );
  expect(calls.map((call) => call.args.words)).toEqual([
    ["原有术语", "新术语"],
    ["原有术语"],
  ]);
  const box = await region.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(720);
});
