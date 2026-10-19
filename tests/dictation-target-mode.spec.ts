import { expect, test, type Page } from "@playwright/test";

declare global {
  interface Window {
    __DICTATION_TARGET_TEST__: {
      calls: { command: string; args?: Record<string, unknown> }[];
      stored: Record<string, unknown>;
      rejectSave: boolean;
      refresh: () => Promise<void>;
    };
  }
}

async function mountDictationSettings(
  page: Page,
  osType = "macos",
  mode?: "current_cursor" | "recording_application",
) {
  await page.route("**/__dictation-settings-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__dictation-settings-test");
  await page.evaluate(
    async ({ osType, mode }) => {
      const state = {
        calls: [] as { command: string; args?: Record<string, unknown> }[],
        stored: {
          app_language: "zh",
          paste_method: "ctrl_v",
          auto_submit: false,
          auto_submit_key: "cmd_enter",
          custom_words: ["Inputia", "保留词语"],
          ...(mode ? { dictation_target_mode: mode } : {}),
        } as Record<string, unknown>,
        rejectSave: false,
        refresh: async () => {},
      };
      Object.assign(window, {
        __DICTATION_TARGET_TEST__: state,
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
            state.calls.push({ command, args });
            if (command === "get_app_settings") return { ...state.stored };
            if (command === "change_dictation_target_mode_setting") {
              if (state.rejectSave) throw "save failed";
              state.stored.dictation_target_mode = args?.mode;
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
      const { default: React } = await import(
        "/node_modules/.vite/deps/react.js"
      );
      const { default: ReactDOM } = await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      );
      const { default: i18n } = await import("/src/i18n/index.ts");
      await i18n.changeLanguage("zh");
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      useSettingsStore.setState({
        isLoading: false,
        settings: { ...state.stored },
      });
      state.refresh = () => useSettingsStore.getState().refreshSettings();
      const { DictationTargetModeSetting } = await import(
        "/src/components/settings/DictationTargetMode.tsx"
      );
      const { PasteMethodSetting } = await import(
        "/src/components/settings/PasteMethod.tsx"
      );
      const { AutoSubmit } = await import(
        "/src/components/settings/AutoSubmit.tsx"
      );
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(
          React.Fragment,
          null,
          React.createElement(
            "div",
            { id: "paste" },
            React.createElement(PasteMethodSetting),
          ),
          React.createElement(
            "div",
            { id: "target" },
            React.createElement(DictationTargetModeSetting),
          ),
          React.createElement(
            "div",
            { id: "submit" },
            React.createElement(AutoSubmit),
          ),
        ),
      );
    },
    { osType, mode },
  );
  await expect(
    page.locator("#paste").getByRole("button", { name: /Cmd|Ctrl/ }),
  ).toBeVisible();
}

test("旧配置缺少插入位置时显示当前光标，不自动改写配置", async ({ page }) => {
  await mountDictationSettings(page);
  await expect(
    page
      .locator("#target")
      .getByRole("button", { name: "当前光标（默认）", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("选择文字准备好后的插入位置。更改从下次录音生效。"),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      window.__DICTATION_TARGET_TEST__.calls.filter((call) =>
        call.command.startsWith("change_"),
      ),
    ),
  ).toEqual([]);
});

test("已保存的开始录音应用模式正确回显", async ({ page }) => {
  await mountDictationSettings(page, "macos", "recording_application");
  await expect(
    page
      .locator("#target")
      .getByRole("button", { name: "开始录音的应用", exact: true }),
  ).toBeVisible();
});

test("两种模式说明清晰，切换保存后可重新读取且保留其它设置", async ({
  page,
}) => {
  await mountDictationSettings(page, "macos", "current_cursor");
  const target = page.locator("#target");
  await target
    .getByRole("button", { name: "当前光标（默认）", exact: true })
    .click();
  await expect(
    target.getByText("文字准备好后，插入当时的光标位置。讲话时可以切换应用。"),
  ).toBeVisible();
  await expect(
    target.getByText(
      "插入前需回到开始录音的应用。讲话时可以切换应用，Inputia 不会自动切回。",
    ),
  ).toBeVisible();
  await target.getByRole("button", { name: /^开始录音的应用/ }).click();
  await expect(
    target.getByRole("button", { name: "开始录音的应用", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => window.__DICTATION_TARGET_TEST__.refresh());
  await expect(
    target.getByRole("button", { name: "开始录音的应用", exact: true }),
  ).toBeVisible();
  await target
    .getByRole("button", { name: "开始录音的应用", exact: true })
    .click();
  await target.getByRole("button", { name: /^当前光标（默认）/ }).click();
  await expect(
    target.getByRole("button", { name: "当前光标（默认）", exact: true }),
  ).toBeEnabled();
  await page.evaluate(() => window.__DICTATION_TARGET_TEST__.refresh());
  await expect(
    target.getByRole("button", { name: "当前光标（默认）", exact: true }),
  ).toBeVisible();
  const state = await page.evaluate(() => ({
    changes: window.__DICTATION_TARGET_TEST__.calls.filter((call) =>
      call.command.startsWith("change_"),
    ),
    stored: window.__DICTATION_TARGET_TEST__.stored,
  }));
  expect(state.changes).toEqual([
    {
      command: "change_dictation_target_mode_setting",
      args: { mode: "recording_application" },
    },
    {
      command: "change_dictation_target_mode_setting",
      args: { mode: "current_cursor" },
    },
  ]);
  expect(state.stored).toEqual({
    app_language: "zh",
    paste_method: "ctrl_v",
    auto_submit: false,
    auto_submit_key: "cmd_enter",
    custom_words: ["Inputia", "保留词语"],
    dictation_target_mode: "current_cursor",
  });
});

test("保存失败恢复已保存的模式且不重发修改", async ({ page }) => {
  await mountDictationSettings(page, "macos", "recording_application");
  await page.evaluate(() => {
    window.__DICTATION_TARGET_TEST__.rejectSave = true;
  });
  const target = page.locator("#target");
  await target
    .getByRole("button", { name: "开始录音的应用", exact: true })
    .click();
  await target.getByRole("button", { name: /^当前光标（默认）/ }).click();
  await expect(
    target.getByRole("button", { name: "开始录音的应用", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      () => window.__DICTATION_TARGET_TEST__.stored.dictation_target_mode,
    ),
  ).toBe("recording_application");
  expect(
    await page.evaluate(() =>
      window.__DICTATION_TARGET_TEST__.calls.filter((call) =>
        call.command.startsWith("change_"),
      ),
    ),
  ).toHaveLength(1);
});

for (const osType of ["windows", "linux"]) {
  test(`${osType} 不显示仅用于 macOS 的语音插入位置`, async ({ page }) => {
    await mountDictationSettings(page, osType, "current_cursor");
    await expect(page.locator("#target")).toBeEmpty();
  });
}
