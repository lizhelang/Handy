import { expect, test, type Page } from "@playwright/test";

// 挂载真实 App 与历史页；仅替代原生数据接口，不替代原生输入验收。
async function mountAppHistory(page: Page, fontSize: number) {
  await page.route("**/__history-layout-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__history-layout-test");
  await page.evaluate(async (fontSize) => {
    document.documentElement.style.fontSize = `${fontSize}px`;
    const settings = {
      onboarding_completed: true,
      show_whats_new_on_update: false,
      update_checks_enabled: false,
      bindings: {},
      selected_model: "",
      post_process_enabled: false,
      debug_mode: false,
    };
    let callbackId = 0;
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "linux",
        os_type: "linux",
        family: "unix",
        version: "6",
        arch: "x86_64",
        eol: "\n",
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => null },
      __TAURI_INTERNALS__: {
        transformCallback: () => ++callbackId,
        unregisterCallback: () => null,
        invoke: async (cmd: string) => {
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return settings;
          if (cmd === "plugin:app|version") return "1.0.0";
          if (cmd === "plugin:event|listen") return ++callbackId;
          if (cmd === "get_unified_history")
            return [
              {
                item_id: "long-item",
                store_id: "fixture",
                record_id: "1",
                revision: 1,
                source_kind: "clipboard",
                content_type: "text",
                text: "文件路径" + "verylongunbrokenpath".repeat(25),
                title: null,
                starred: false,
                pinned: false,
                created_at_ms: 1000000,
                asset_ref: null,
                source_app: "com.example." + "longname".repeat(30),
              },
            ];
          if (
            cmd === "get_available_models" ||
            cmd === "get_available_microphones" ||
            cmd === "get_available_output_devices" ||
            cmd === "initialize_shortcuts"
          )
            return [];
          if (cmd === "get_unified_history_revisions") return [];
          if (cmd === "get_model_load_status")
            return { loaded: false, model_id: null };
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
    const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
    useSettingsStore.setState({
      settings,
      isLoading: false,
      initialize: async () => undefined,
      refreshAudioDevices: async () => undefined,
      refreshOutputDevices: async () => undefined,
    });
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { default: App } = await import("/src/App.tsx");
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(App),
    );
  }, fontSize);
  await page.getByText("历史记录", { exact: true }).click();
  await expect(page.getByRole("searchbox")).toBeVisible();
}

for (const size of [
  { width: 1000, height: 700, font: 15 },
  { width: 720, height: 600, font: 15 },
  { width: 720, height: 600, font: 20 },
  { width: 720, height: 600, font: 30 },
  { width: 600, height: 600, font: 20 },
]) {
  test(`history fits beside sidebar at ${size.width}px and ${size.font}px text`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize(size);
    await mountAppHistory(page, size.font);
    const history = page.getByRole("region", { name: "历史记录", exact: true });
    await expect(history).toBeVisible();
    const measure = async () =>
      page.evaluate(() => {
        const section = document.querySelector(
          'section[aria-label="历史记录"]',
        )!;
        const sidebar = document.querySelector("[data-inputia-mark]")!
          .parentElement!.parentElement!;
        const bounds = section.getBoundingClientRect();
        const side = sidebar.getBoundingClientRect();
        return {
          left: bounds.left,
          sidebarRight: side.right,
          right: bounds.right,
          viewport: innerWidth,
          scrollWidth: section.scrollWidth,
          clientWidth: section.clientWidth,
        };
      });
    let bounds = await measure();
    expect(bounds.left).toBeGreaterThanOrEqual(bounds.sidebarRight);
    expect(bounds.right).toBeLessThanOrEqual(bounds.viewport);
    expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
    await history.getByRole("list").locator("li").first().click();
    await expect(page.getByRole("complementary")).toBeVisible();
    bounds = await measure();
    expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
    for (const control of [
      history.locator("header button").first(),
      page.getByRole("searchbox"),
      page.getByRole("combobox").first(),
    ]) {
      const box = await control.boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(bounds.left);
      expect(box!.x + box!.width).toBeLessThanOrEqual(bounds.right + 1);
    }
    await page.screenshot({ path: testInfo.outputPath("history-layout.png") });
  });
}
