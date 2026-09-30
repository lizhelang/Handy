import { expect, test, type Page } from "@playwright/test";

type FixtureMode =
  | "normal"
  | "begin-error"
  | "late-begin"
  | "end-error"
  | "late-end"
  | "end-once"
  | "expired";
async function mount(page: Page, native: boolean, mode: FixtureMode) {
  await page.route("**/__capture-check", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<html><body><div id='root'></div><button id='outside'>outside</button></body></html>",
    }),
  );
  await page.goto("/__capture-check");
  await page.evaluate(
    async ({ native, mode }) => {
      const refresh = await import("/@react-refresh");
      refresh.default.injectIntoGlobalHook(window);
      const commands: { command: string; args: Record<string, unknown> }[] = [];
      const callbacks = new Map<number, (event: unknown) => void>();
      const listeners = new Map<string, number>();
      let callbackId = 0;
      let beginCount = 0;
      let endCount = 0;
      let resolveBegin: ((token: string) => void) | undefined;
      let resolveEnd: (() => void) | undefined;
      const saved = {
        bindings: {
          transcribe: {
            id: "transcribe",
            name: "Transcribe",
            description: "fixture",
            default_binding: "F8",
            current_binding: "F8",
          },
        },
        keyboard_implementation: native ? "handy_keys" : "tauri",
      };
      Object.assign(window, {
        $RefreshReg$: () => {},
        $RefreshSig$: () => (type: unknown) => type,
        __vite_plugin_react_preamble_installed__: true,
        __TAURI_OS_PLUGIN_INTERNALS__: { os_type: "macos" },
        __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
        __TAURI_INTERNALS__: {
          transformCallback: (callback: (event: unknown) => void) => {
            callbacks.set(++callbackId, callback);
            return callbackId;
          },
          unregisterCallback: () => {},
          invoke: (command: string, args: Record<string, unknown> = {}) => {
            if (command === "plugin:event|listen") {
              listeners.set(String(args.event), Number(args.handler));
              return Promise.resolve(1);
            }
            if (command === "plugin:event|unlisten") return Promise.resolve();
            if (command === "get_app_settings") return Promise.resolve(saved);
            commands.push({ command, args });
            if (
              command === "suspend_all_bindings" ||
              command === "start_handy_keys_recording"
            ) {
              beginCount += 1;
              if (mode === "begin-error")
                return Promise.reject("shortcut_settings_busy");
              if (mode === "late-begin" && beginCount === 1)
                return new Promise<string>((resolve) => {
                  resolveBegin = resolve;
                });
              return Promise.resolve(`capture-${beginCount}`);
            }
            if (
              command === "resume_all_bindings" ||
              command === "stop_handy_keys_recording"
            ) {
              endCount += 1;
              if (mode === "expired") return Promise.resolve(false);
              if (mode === "end-once" && endCount === 1)
                return Promise.reject("shortcut_capture_cleanup_pending");
              if (mode === "end-error")
                return Promise.reject("shortcut_capture_unconfirmed");
              if (mode === "late-end")
                return new Promise<boolean>((resolve) => {
                  resolveEnd = () => resolve(true);
                });
              return Promise.resolve(true);
            }
            if (command === "change_binding") {
              saved.bindings.transcribe.current_binding = String(
                args.newBinding,
              );
              return Promise.resolve({
                success: true,
                binding: saved.bindings.transcribe,
                error: null,
              });
            }
            return Promise.resolve(null);
          },
        },
      });
      const { useSettingsStore } = await import("/src/stores/settingsStore.ts");
      useSettingsStore.setState({
        settings: saved,
        isLoading: false,
        isUpdating: {},
      });
      const component = native
        ? "HandyKeysShortcutInput"
        : "GlobalShortcutInput";
      const path = `/src/components/settings/${component}.tsx`;
      const moduleSource = await (await fetch(path)).text();
      const reactURL = moduleSource.match(
        /from ["']([^"']*\/react\.js[^"']*)["']/,
      )?.[1];
      if (!reactURL) throw new Error("missing React fixture dependency");
      const { default: React } = await import(reactURL);
      const { default: ReactDOM } = await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      );
      const exported = await import(path);
      const root = ReactDOM.createRoot(document.getElementById("root")!);
      const render = (key: string) =>
        root.render(
          React.createElement(exported[component], {
            key,
            shortcutId: "transcribe",
            descriptionMode: "inline",
          }),
        );
      Object.assign(window, {
        captureFixture: {
          commands,
          render,
          unmount: () => root.render(null),
          resolveBegin: () => resolveBegin?.("capture-1"),
          resolveEnd: () => resolveEnd?.(),
          listenerReady: () => listeners.has("handy-keys-event"),
          key: (token: string, down: boolean) => {
            const callback = callbacks.get(
              listeners.get("handy-keys-event") ?? -1,
            );
            callback?.({
              event: "handy-keys-event",
              id: 1,
              payload: {
                capture_token: token,
                modifiers: [],
                key: "f9",
                is_key_down: down,
                hotkey_string: "F9",
              },
            });
          },
        },
      });
      render("first");
    },
    { native, mode },
  );
  await expect(page.getByText("F8", { exact: true })).toBeVisible();
}
async function commands(page: Page) {
  return page.evaluate(
    () =>
      Reflect.get(window, "captureFixture").commands as {
        command: string;
        args: Record<string, unknown>;
      }[],
  );
}
async function select(page: Page, native: boolean, token = "capture-1") {
  if (native) {
    await expect
      .poll(() =>
        page.evaluate(() =>
          Reflect.get(window, "captureFixture").listenerReady(),
        ),
      )
      .toBe(true);
    await page.evaluate(
      (token) => Reflect.get(window, "captureFixture").key(token, true),
      token,
    );
    await expect(page.getByText("F9", { exact: true })).toBeVisible();
    await page.evaluate(
      (token) => Reflect.get(window, "captureFixture").key(token, false),
      token,
    );
  } else {
    await page.keyboard.down("F9");
    await expect(page.getByText("F9", { exact: true })).toBeVisible();
    await page.keyboard.up("F9");
  }
}
for (const native of [false, true]) {
  test(`${native ? "HandyKeys" : "Tauri"}: begin denied never enters capture`, async ({
    page,
  }) => {
    await mount(page, native, "begin-error");
    await page.getByText("F8", { exact: true }).click();
    await expect.poll(async () => (await commands(page)).length).toBe(1);
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toHaveCount(0);
    expect((await commands(page)).map((item) => item.command)).toEqual([
      native ? "start_handy_keys_recording" : "suspend_all_bindings",
    ]);
  });
  test(`${native ? "HandyKeys" : "Tauri"}: late begin only ends its own token`, async ({
    page,
  }) => {
    await mount(page, native, "late-begin");
    await page.getByText("F8", { exact: true }).click();
    await expect.poll(async () => (await commands(page)).length).toBe(1);
    await page.evaluate(() =>
      Reflect.get(window, "captureFixture").render("second"),
    );
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await page.evaluate(() =>
      Reflect.get(window, "captureFixture").resolveBegin(),
    );
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-1",
          ).length,
      )
      .toBe(1);
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    expect(
      (await commands(page)).some((item) => item.args.token === "capture-2"),
    ).toBe(false);
    await page.evaluate(() => Reflect.get(window, "captureFixture").unmount());
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-2",
          ).length,
      )
      .toBe(1);
  });
  test(`${native ? "HandyKeys" : "Tauri"}: failed end cannot save the binding`, async ({
    page,
  }) => {
    await mount(page, native, "end-error");
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await select(page, native);
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-1",
          ).length,
      )
      .toBe(1);
    expect(
      (await commands(page)).some((item) => item.command === "change_binding"),
    ).toBe(false);
    await expect(page.getByText("F9", { exact: true })).toBeVisible();
  });
  test(`${native ? "HandyKeys" : "Tauri"}: idle deadline exits and late begin cannot reopen capture`, async ({
    page,
  }) => {
    await page.clock.install();
    await mount(page, native, "late-begin");
    await page.getByText("F8", { exact: true }).click();
    await expect.poll(async () => (await commands(page)).length).toBe(1);
    await page.clock.fastForward(60_001);
    await page.evaluate(() =>
      Reflect.get(window, "captureFixture").resolveBegin(),
    );
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-1",
          ).length,
      )
      .toBe(1);
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toHaveCount(0);
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await page.clock.fastForward(60_001);
    await expect(page.getByText("F8", { exact: true })).toBeVisible();
    expect(
      (await commands(page)).some((item) => item.command === "change_binding"),
    ).toBe(false);
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-2",
          ).length,
      )
      .toBe(1);
  });
  test(`${native ? "HandyKeys" : "Tauri"}: expired capture exits without saving`, async ({
    page,
  }) => {
    await mount(page, native, "expired");
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await select(page, native);
    await expect(page.getByText("F8", { exact: true })).toBeVisible();
    expect(
      (await commands(page)).some((item) => item.command === "change_binding"),
    ).toBe(false);
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
  });
  test(`${native ? "HandyKeys" : "Tauri"}: unknown end keeps original token for retry`, async ({
    page,
  }) => {
    await mount(page, native, "end-once");
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await select(page, native);
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-1",
          ).length,
      )
      .toBe(1);
    await page.locator("#outside").click();
    await expect(page.getByText("F8", { exact: true })).toBeVisible();
    expect(
      (await commands(page)).filter((item) => item.args.token === "capture-1")
        .length,
    ).toBe(2);
    expect(
      (await commands(page)).some((item) => item.command === "change_binding"),
    ).toBe(false);
  });
  test(`${native ? "HandyKeys" : "Tauri"}: cancel wins while end is pending`, async ({
    page,
  }) => {
    await mount(page, native, "late-end");
    await page.getByText("F8", { exact: true }).click();
    await expect(
      page.getByText("settings.general.shortcut.pressKeys"),
    ).toBeVisible();
    await select(page, native);
    await expect
      .poll(
        async () =>
          (await commands(page)).filter(
            (item) => item.args.token === "capture-1",
          ).length,
      )
      .toBe(1);
    await page.locator("#outside").click();
    await page.evaluate(() =>
      Reflect.get(window, "captureFixture").resolveEnd(),
    );
    await expect(page.getByText("F8", { exact: true })).toBeVisible();
    expect(
      (await commands(page)).some((item) => item.command === "change_binding"),
    ).toBe(false);
  });
}
test("HandyKeys: stale capture events ignored; matching event ends before save", async ({
  page,
}) => {
  await mount(page, true, "normal");
  await page.getByText("F8", { exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Reflect.get(window, "captureFixture").listenerReady(),
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    const fixture = Reflect.get(window, "captureFixture");
    fixture.key("old", true);
    fixture.key("old", false);
  });
  await expect(
    page.getByText("settings.general.shortcut.pressKeys"),
  ).toBeVisible();
  expect((await commands(page)).length).toBe(1);
  await select(page, true);
  await expect
    .poll(async () => (await commands(page)).map((item) => item.command))
    .toEqual([
      "start_handy_keys_recording",
      "stop_handy_keys_recording",
      "change_binding",
    ]);
});
