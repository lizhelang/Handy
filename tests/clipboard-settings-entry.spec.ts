import { expect, test, type Page } from "@playwright/test";

type InvokeRecord = { cmd: string; args?: Record<string, unknown> };

declare global {
  interface Window {
    __CLIPBOARD_SETTINGS_TEST__: {
      records: InvokeRecord[];
      resetRecords: () => void;
    };
  }
}

const installReactHarness = async (page: Page, path: string) => {
  await page.route(`**/${path}`, (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root" style="height:95vh;padding:24px"></div></body></html>',
    }),
  );
  await page.goto(`/${path}`);
  await page.evaluate(async () => {
    const { default: RefreshRuntime } = await import("/@react-refresh");
    RefreshRuntime.injectIntoGlobalHook(window);
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { os_type: "macos", platform: "macos" },
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
  });
};

const installClipboardMocks = async (page: Page) => {
  await page.evaluate(() => {
    const records: InvokeRecord[] = [];
    Object.assign(window, {
      __CLIPBOARD_SETTINGS_TEST__: {
        records,
        resetRecords: () => {
          records.length = 0;
        },
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: {
        unregisterListener: () => null,
      },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => null,
        invoke: async (cmd: string, args?: Record<string, unknown>) => {
          records.push({ cmd, args });
          if (cmd === "plugin:event|listen") return 1;
          if (cmd === "plugin:event|unlisten") return null;
          if (cmd === "get_clipboard_stats") {
            return {
              total_items: 4,
              favorites_count: 0,
              pinned_count: 0,
              total_size_bytes: 24,
            };
          }
          if (cmd === "get_clipboard_settings") {
            return {
              max_records: 500,
              hotkey: "Alt+Shift+Space",
              confirm_mode: "copy",
            };
          }
          if (cmd === "get_clipboard_items") {
            return {
              items: [
                {
                  id: 1,
                  content_type: "text",
                  content_preview: "Existing clipboard item",
                  content_hash: "hash-1",
                  full_text: "Existing clipboard item",
                  source_app: "Notes",
                  is_favorite: false,
                  is_pinned: false,
                  created_at: "2026-09-02T03:20:58Z",
                  size_bytes: 24,
                },
                {
                  id: 2,
                  content_type: "image",
                  content_preview: "Image 2x2",
                  content_hash: "hash-2",
                  source_app: "Preview",
                  is_favorite: false,
                  is_pinned: false,
                  created_at: "2026-09-02T03:20:57Z",
                  size_bytes: 32,
                },
                {
                  id: 3,
                  content_type: "file",
                  content_preview: "Files",
                  content_hash: "hash-3",
                  full_text: '["/synthetic/retained-document.pdf"]',
                  source_app: "Finder",
                  is_favorite: false,
                  is_pinned: false,
                  created_at: "2026-09-02T03:20:56Z",
                  size_bytes: 40,
                },
                {
                  id: 4,
                  content_type: "richtext",
                  content_preview: "Retained rich text",
                  content_hash: "hash-4",
                  full_text: "<p>Retained rich text</p>",
                  source_app: "Notes",
                  is_favorite: false,
                  is_pinned: false,
                  created_at: "2026-09-02T03:20:55Z",
                  size_bytes: 30,
                },
              ],
              has_more: false,
            };
          }
          if (cmd === "show_clipboard_overlay") return null;
          if (cmd === "change_clipboard_enabled_setting") return null;
          throw new Error(`Unhandled command: ${cmd}`);
        },
      },
    });
  });
};

test("sidebar keeps separate history and clipboard entries when capture is off", async ({
  page,
}) => {
  await installReactHarness(page, "__clipboard-sidebar-test");
  await page.evaluate(async () => {
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("zh");
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
    useSettingsStore.setState({
      settings: {
        clipboard_enabled: false,
        post_process_enabled: false,
        debug_mode: false,
      },
      isLoading: false,
    });
    const { Sidebar } = await import("/src/components/Sidebar.tsx");
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(Sidebar, {
        activeSection: "general",
        onSectionChange: () => null,
      }),
    );
  });

  await expect(page.getByText("录音", { exact: true })).toBeVisible();
  await expect(page.getByText("剪切", { exact: true })).toBeVisible();
  await expect(page.getByText("历史记录", { exact: true })).toHaveCount(0);
  await expect(page.getByText("剪贴历史", { exact: true })).toHaveCount(0);
});

test("clipboard page opens the real overlay only from the explicit button", async ({
  page,
}) => {
  await installReactHarness(page, "__clipboard-settings-test");
  await installClipboardMocks(page);
  await page.evaluate(async () => {
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
    useSettingsStore.setState({
      settings: {
        clipboard_enabled: false,
        post_process_enabled: false,
        debug_mode: false,
      },
      isLoading: false,
      refreshSettings: async () => undefined,
    });
    const { ClipboardSettings } = await import(
      "/src/components/clipboard/ClipboardSettings.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(ClipboardSettings),
    );
  });

  await expect(page.getByText("Existing clipboard item")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Clipboard", exact: true }),
  ).toBeVisible();
  await page.getByTitle("List", { exact: true }).click();
  await expect(page.getByText("Images 2x2", { exact: true })).toBeVisible();
  await expect(
    page.getByText("retained-document.pdf", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Retained rich text", { exact: true }),
  ).toBeVisible();
  const loadCommands = await page.evaluate(() =>
    window.__CLIPBOARD_SETTINGS_TEST__.records.map((record) => record.cmd),
  );
  expect(loadCommands).toContain("get_clipboard_items");
  expect(loadCommands).not.toContain("get_unified_history");
  expect(
    loadCommands.some((command) => /delete|clear|migrat/.test(command)),
  ).toBe(false);
  expect(
    await page.evaluate(() =>
      window.__CLIPBOARD_SETTINGS_TEST__.records.map((record) => record.cmd),
    ),
  ).not.toContain("change_clipboard_enabled_setting");
  expect(
    await page.evaluate(() =>
      window.__CLIPBOARD_SETTINGS_TEST__.records.map((record) => record.cmd),
    ),
  ).not.toContain("show_clipboard_overlay");

  await page.evaluate(() => window.__CLIPBOARD_SETTINGS_TEST__.resetRecords());
  await page.getByRole("button", { name: "Open clipboard overlay" }).click();

  expect(
    await page.evaluate(() =>
      window.__CLIPBOARD_SETTINGS_TEST__.records.map((record) => record.cmd),
    ),
  ).toEqual(["show_clipboard_overlay"]);
});
