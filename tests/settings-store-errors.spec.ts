import { expect, test } from "@playwright/test";

// 真实 Zustand store + 生成的 Specta 包装；IPC 仅合成回复，不访问用户配置。
for (const scenario of [
  "different-fields",
  "newer-same-field",
  "failed-overlap",
  "saved-then-failed",
  "saved-during-rollback-read",
  "late-success-after-latest-failure",
  "older-success-after-newer-success",
  "refresh-during-save",
  "late-refresh-after-save",
  "transport-error",
  "provider-error",
  "binding-business-error",
] as const) {
  test(`settings save failure: ${scenario}`, async ({ page }) => {
    const logs: string[] = [];
    page.on("console", (message) => logs.push(message.text()));
    await page.route("**/__settings-error-check", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "<html><body></body></html>",
      }),
    );
    await page.goto("/__settings-error-check");
    const result = await page.evaluate(async (scenario) => {
      const secret = "TEST_PRIVATE_CONFIG_MUST_NOT_LEAK";
      const saved = {
        audio_feedback: true,
        debug_mode: false,
        selected_language: "en",
        post_process_api_keys: { fixture: "old-key" },
        bindings: {
          transcribe: {
            id: "transcribe",
            name: "Transcribe",
            description: "fixture",
            default_binding: "Ctrl+Space",
            current_binding: "Ctrl+Space",
          },
        },
      };
      const requests: Array<{
        command: string;
        resolve: (value: unknown) => void;
        reject: (value: unknown) => void;
      }> = [];
      let readsFail = false;
      let deferredRead: ((value: unknown) => void) | undefined;
      let holdReads = false;
      let reads = 0;
      Object.assign(window, {
        __TAURI_INTERNALS__: {
          transformCallback: () => 1,
          unregisterCallback: () => undefined,
          invoke: (command: string) => {
            if (command === "get_app_settings") {
              reads += 1;
              if (holdReads)
                return new Promise((resolve) => {
                  deferredRead = resolve;
                });
              return readsFail
                ? Promise.reject(secret)
                : Promise.resolve(saved);
            }
            return new Promise((resolve, reject) => {
              requests.push({ command, resolve, reject });
            });
          },
        },
      });
      // 使用 Vite 给真实 store 注入的同一 URL（含版本查询串），避免创建第二份单例。
      const storeModule = await (
        await fetch("/src/stores/settingsStore.ts")
      ).text();
      const dependency = (name: string) => {
        const match = storeModule.match(
          new RegExp(`from ["']([^"']*/${name}\\.js[^"']*)["']`),
        );
        if (!match) throw new Error("fixture dependency missing");
        return match[1];
      };
      const { default: i18n } = await import(dependency("i18next"));
      await i18n.init({
        lng: "en",
        resources: {
          en: {
            translation: {
              unifiedHistory: {
                feedback: {
                  updateFailed: "Could not save. Refresh and try again.",
                },
              },
            },
          },
        },
      });
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      const { toast } = await import(dependency("sonner"));
      useSettingsStore.setState({
        settings: saved,
        isLoading: false,
        isUpdating: {},
      });
      const store = useSettingsStore.getState();
      let middleBusy = false;
      let caught = "";
      let succeeded: boolean | undefined;
      let failed: boolean | undefined;
      if (scenario === "different-fields") {
        const first = store.updateSetting("audio_feedback", false);
        const second = store.updateSetting("debug_mode", true);
        requests[1].resolve(null);
        succeeded = await second;
        requests[0].reject(secret); // string rejection -> Specta status:error，不是抛异常。
        failed = await first;
      } else if (scenario === "newer-same-field") {
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[1].resolve(null);
        await second;
        middleBusy = useSettingsStore.getState().isUpdating.selected_language;
        requests[0].reject(secret);
        await first;
      } else if (scenario === "failed-overlap") {
        readsFail = true;
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[0].reject(secret);
        await first;
        requests[1].reject(secret);
        await second;
      } else if (
        scenario === "saved-then-failed" ||
        scenario === "saved-during-rollback-read"
      ) {
        readsFail = true;
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[0].resolve(null);
        await first;
        requests[1].reject(secret);
        await second;
      } else if (scenario === "late-success-after-latest-failure") {
        readsFail = true;
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[1].reject(secret);
        await second;
        requests[0].resolve(null);
        await first;
      } else if (scenario === "older-success-after-newer-success") {
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[1].resolve(null);
        await second;
        requests[0].resolve(null);
        await first;
      } else if (scenario === "saved-during-rollback-read") {
        holdReads = true;
        const first = store.updateSetting("selected_language", "fr");
        const second = store.updateSetting("selected_language", "de");
        requests[1].reject(secret);
        while (!deferredRead)
          await new Promise((resolve) => setTimeout(resolve, 0));
        requests[0].resolve(null);
        await first;
        deferredRead(saved);
        await second;
      } else if (
        scenario === "refresh-during-save" ||
        scenario === "late-refresh-after-save"
      ) {
        holdReads = true;
        const first = store.updateSetting("debug_mode", true);
        const second = store.updatePostProcessBaseUrl(
          "fixture",
          "https://fixture.invalid",
        );
        requests[1].reject(secret);
        // 等到失败分支真实启动整表刷新。
        while (!deferredRead)
          await new Promise((resolve) => setTimeout(resolve, 0));
        if (scenario === "late-refresh-after-save") {
          requests[0].resolve(null);
          await first;
        }
        deferredRead(saved);
        await second;
        if (scenario === "refresh-during-save") {
          requests[0].resolve(null);
          await first;
        }
      } else if (scenario === "transport-error") {
        const first = store.updateSetting("audio_feedback", false);
        requests[0].reject(new Error(secret)); // Error rejection走包装的throw分支。
        await first;
      } else if (scenario === "provider-error") {
        const first = store.updatePostProcessApiKey("fixture", "new-key");
        requests[0].reject(secret);
        await first;
      } else {
        const first = store.updateBinding("transcribe", "Ctrl+A");
        requests[0].resolve({ success: false, binding: null, error: secret });
        try {
          await first;
        } catch (error) {
          caught = error instanceof Error ? error.message : "unexpected";
        }
      }
      return {
        settings: useSettingsStore.getState().settings,
        updating: useSettingsStore.getState().isUpdating,
        middleBusy,
        succeeded,
        failed,
        caught,
        reads,
        commands: requests.map((request) => request.command),
        notices: toast
          .getHistory()
          .map((entry) => ("title" in entry ? String(entry.title) : "")),
      };
    }, scenario);
    expect(JSON.stringify(result.notices)).not.toContain("TEST_PRIVATE_CONFIG");
    expect(logs.join("\n")).not.toContain("TEST_PRIVATE_CONFIG");
    if (scenario !== "older-success-after-newer-success")
      expect(result.notices).toContain(
        "Could not save. Refresh and try again.",
      );
    expect(
      Object.values(result.updating).every((value) => value === false),
    ).toBe(true);
    if (scenario === "different-fields") {
      expect(result.succeeded).toBe(true);
      expect(result.failed).toBe(false);
      expect(result.settings.audio_feedback).toBe(true);
      expect(result.settings.debug_mode).toBe(true);
      expect(result.commands).toHaveLength(2);
      expect(result.reads).toBe(1);
    } else if (scenario === "newer-same-field") {
      expect(result.settings.selected_language).toBe("de");
      expect(result.middleBusy).toBe(true);
      expect(result.reads).toBe(0);
    } else if (scenario === "older-success-after-newer-success") {
      expect(result.settings.selected_language).toBe("de");
    } else if (scenario === "late-success-after-latest-failure") {
      expect(result.settings.selected_language).toBe("fr");
    } else if (scenario === "failed-overlap") {
      expect(result.settings.selected_language).toBe("en");
    } else if (
      scenario === "saved-then-failed" ||
      scenario === "saved-during-rollback-read"
    ) {
      expect(result.settings.selected_language).toBe("fr");
    } else if (
      scenario === "refresh-during-save" ||
      scenario === "late-refresh-after-save"
    ) {
      expect(result.settings.debug_mode).toBe(true);
      expect(result.commands).toHaveLength(2);
    } else if (scenario === "transport-error") {
      expect(result.settings.audio_feedback).toBe(true);
      expect(result.commands).toEqual(["change_audio_feedback_setting"]);
    } else if (scenario === "provider-error") {
      expect(result.settings.post_process_api_keys.fixture).toBe("old-key");
      expect(result.commands).toEqual(["change_post_process_api_key_setting"]);
    } else {
      expect(result.settings.bindings.transcribe.current_binding).toBe(
        "Ctrl+Space",
      );
      expect(result.caught).toBe("Could not save. Refresh and try again.");
    }
  });
}

for (const component of [
  "AutoSubmit",
  "AccelerationSelector",
  "LanguageSelector",
] as const) {
  test(`dependent settings action stops after failed save: ${component}`, async ({
    page,
  }) => {
    await page.route("**/__settings-sequence-check", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "<html><body><div id='root'></div></body></html>",
      }),
    );
    await page.goto("/__settings-sequence-check");
    await page.evaluate(async (component) => {
      const refresh = await import("/@react-refresh");
      refresh.default.injectIntoGlobalHook(window);
      Object.assign(window, {
        $RefreshReg$: () => {},
        $RefreshSig$: () => (type: unknown) => type,
        __vite_plugin_react_preamble_installed__: true,
      });
      const saved = {
        auto_submit: false,
        auto_submit_key: "enter",
        transcribe_accelerator: "auto",
        transcribe_gpu_device: null,
        ort_accelerator: "auto",
        selected_language: "en",
      };
      const mutations: string[] = [];
      Object.assign(window, {
        __settingsTestMutations: mutations,
        __TAURI_OS_PLUGIN_INTERNALS__: { os_type: "macos" },
        __TAURI_INTERNALS__: {
          transformCallback: () => 1,
          unregisterCallback: () => {},
          invoke: (command: string) => {
            if (command === "get_app_settings") return Promise.resolve(saved);
            if (command === "get_available_accelerators")
              return Promise.resolve({
                transcribe: ["auto", "cpu"],
                ort: ["auto"],
                gpu_devices: [],
              });
            mutations.push(command);
            return Promise.reject("TEST_PRIVATE_CONFIG_MUST_NOT_LEAK");
          },
        },
      });
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      useSettingsStore.setState({
        settings: saved,
        isLoading: false,
        isUpdating: {},
      });
      const moduleSource = await (
        await fetch(`/src/components/settings/${component}.tsx`)
      ).text();
      const reactURL = moduleSource.match(
        /from ["']([^"']*\/react\.js[^"']*)["']/,
      )?.[1];
      if (!reactURL) throw new Error("missing React fixture dependency");
      const { default: React } = await import(reactURL);
      const { default: ReactDOM } = await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      );
      const exported = await import(
        `/src/components/settings/${component}.tsx`
      );
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(exported[component], { descriptionMode: "inline" }),
      );
    }, component);
    if (component === "AutoSubmit") {
      await page
        .getByRole("button", {
          name: "settings.advanced.autoSubmit.options.off",
          exact: true,
        })
        .click();
      await page
        .getByRole("button", {
          name: "settings.advanced.autoSubmit.options.ctrlEnter",
          exact: true,
        })
        .click();
    } else if (component === "AccelerationSelector") {
      await page
        .getByRole("button", {
          name: "settings.advanced.acceleration.gpuDevice.auto",
          exact: true,
        })
        .click();
      await page.getByRole("button", { name: "CPU", exact: true }).click();
    } else {
      await page.getByRole("button", { name: "English", exact: true }).click();
      await page.getByRole("textbox").fill("French");
      await page.getByRole("button", { name: "French", exact: true }).click();
      await expect(page.getByRole("textbox")).toHaveValue("French");
    }
    await expect
      .poll(() =>
        page.evaluate(async () => {
          const { useSettingsStore } = await import(
            "/src/stores/settingsStore.ts"
          );
          return Object.values(useSettingsStore.getState().isUpdating).every(
            (value) => !value,
          );
        }),
      )
      .toBe(true);
    const mutations = await page.evaluate(() =>
      Reflect.get(window, "__settingsTestMutations"),
    );
    expect(mutations).toEqual([
      component === "AutoSubmit"
        ? "change_auto_submit_key_setting"
        : component === "AccelerationSelector"
          ? "change_transcribe_gpu_device"
          : "change_selected_language_setting",
    ]);
  });
}
