import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __PASTE_SETTINGS_TEST__: {
      calls: { command: string; args?: Record<string, unknown> }[];
    };
  }
}

async function mountPasteSettings(
  page: Page,
  osType: string,
  reliablePaste: boolean,
) {
  await page.route("**/__paste-settings-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__paste-settings-test");
  await page.evaluate(
    async ({ osType, reliablePaste }) => {
      const calls: { command: string; args?: Record<string, unknown> }[] = [];
      Object.assign(window, {
        __PASTE_SETTINGS_TEST__: { calls },
        __TAURI_OS_PLUGIN_INTERNALS__: {
          os_type: osType,
          platform: osType,
          family: "unix",
          version: "26.0",
          arch: "aarch64",
          eol: "\n",
          exe_extension: "",
        },
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args?: Record<string, unknown>) => {
            calls.push({ command, args });
            if (command === "get_app_settings") return { app_language: "en" };
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
      const { default: React } = await import(
        "/node_modules/.vite/deps/react.js"
      );
      const { default: ReactDOM } = await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      );
      const { default: i18n } = await import("/src/i18n/index.ts");
      await i18n.changeLanguage("en");
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      useSettingsStore.setState({
        isLoading: false,
        settings: {
          reliable_paste: reliablePaste,
          paste_delay_ms: 60,
          paste_delay_after_ms: 120,
        },
      });
      const { PasteDelay } = await import(
        "/src/components/settings/debug/PasteDelay.tsx"
      );
      const { ReliablePasteToggle } = await import(
        "/src/components/settings/debug/ReliablePaste.tsx"
      );
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(
          React.Fragment,
          null,
          React.createElement(
            "div",
            { id: "before" },
            React.createElement(PasteDelay),
          ),
          React.createElement(
            "div",
            { id: "after" },
            React.createElement(PasteDelay, {
              settingKey: "paste_delay_after_ms",
              labelKey: "settings.debug.pasteDelayAfter.title",
              descriptionKey: "settings.debug.pasteDelayAfter.description",
            }),
          ),
          React.createElement(
            "div",
            { id: "reliable" },
            React.createElement(ReliablePasteToggle),
          ),
        ),
      );
    },
    { osType, reliablePaste },
  );
  await expect(page.locator("#before").getByRole("slider")).toBeVisible();
}

for (const [osType, reliablePaste] of [
  ["macos", false],
  ["macos", true],
  ["windows", false],
  ["windows", true],
  ["linux", true],
] as const) {
  test(`${osType} reliable_paste=${reliablePaste} 只按实际粘贴方式隐藏后延迟`, async ({
    page,
  }) => {
    await mountPasteSettings(page, osType, reliablePaste);
    const after = page.locator("#after").getByRole("slider");
    if (osType === "macos" && reliablePaste) {
      await expect(after).toHaveCount(0);
    } else {
      await expect(after).toBeVisible();
      await expect(after).toHaveValue("120");
    }
    const reliable = page.locator("#reliable").getByRole("checkbox");
    if (osType === "linux") {
      await expect(reliable).toHaveCount(0);
    } else {
      await expect(reliable).toBeEnabled();
      await expect(reliable).toBeChecked({ checked: reliablePaste });
    }
  });
}

test("macOS可保存后延迟并切换可靠粘贴，切回标准方式仍保留用户延迟", async ({
  page,
}) => {
  await mountPasteSettings(page, "macos", false);
  const after = page.locator("#after").getByRole("slider");
  await after.focus();
  await after.press("ArrowRight");
  await expect(after).toHaveValue("130");
  await page.locator("#reliable label").click();
  await expect(page.locator("#reliable").getByRole("checkbox")).toBeChecked();
  await expect(after).toHaveCount(0);
  await page.locator("#reliable label").click();
  await expect(
    page.locator("#reliable").getByRole("checkbox"),
  ).not.toBeChecked();
  await expect(after).toHaveValue("130");
  const changes = await page.evaluate(() =>
    window.__PASTE_SETTINGS_TEST__.calls.filter((call) =>
      call.command.startsWith("change_"),
    ),
  );
  expect(changes).toEqual([
    { command: "change_paste_delay_after_ms_setting", args: { ms: 130 } },
    { command: "change_reliable_paste_setting", args: { enabled: true } },
    { command: "change_reliable_paste_setting", args: { enabled: false } },
  ]);
});
