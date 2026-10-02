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
              total_items: 1,
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

  await expect(page.getByText("History", { exact: true })).toBeVisible();
  await expect(page.getByText("Clipboard", { exact: true })).toBeVisible();
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
