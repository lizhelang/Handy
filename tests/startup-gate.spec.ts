import { expect, test, type Page } from "@playwright/test";

test("real main entry keeps settings and model IPC closed in recovery", async ({
  page,
}) => {
  await page.addInitScript(() => {
    localStorage.setItem("i18nextLng", "en");
    const calls: string[] = [];
    Object.assign(window, {
      __STARTUP_MAIN_CALLS__: calls,
      __TAURI_OS_PLUGIN_INTERNALS__: { platform: "macos", os_type: "macos" },
      __TAURI_INTERNALS__: {
        invoke: async (command: string) => {
          calls.push(command);
          if (command === "control_settings_status")
            return {
              phase: "recovery",
              reason: "settings_invalid",
              settings_editable: true,
            };
          throw new Error("unexpected_business_ipc");
        },
      },
    });
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Restart Inputia" }),
  ).toBeVisible();
  const calls = await page.evaluate(
    () =>
      (window as unknown as { __STARTUP_MAIN_CALLS__: string[] })
        .__STARTUP_MAIN_CALLS__,
  );
  expect(calls.length).toBeGreaterThan(0);
  expect(
    calls.filter((command) => command !== "control_settings_status"),
  ).toEqual([]);
});

declare global {
  interface Window {
    __STARTUP_GATE_TEST__: {
      status: unknown;
      initializeCount: number;
      childCount: number;
      failInitialize: boolean;
      failAction: boolean;
      actions: string[];
    };
  }
}

// 临时浏览器里的真实组件与模拟 IPC；不作为原生启动或数据恢复验收。
async function mount(page: Page, status: unknown) {
  await page.route("**/__startup-gate-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__startup-gate-test");
  await page.evaluate(async (initialStatus) => {
    const harness = (window.__STARTUP_GATE_TEST__ = {
      status: initialStatus,
      initializeCount: 0,
      childCount: 0,
      failInitialize: false,
      failAction: false,
      actions: [] as string[],
    });
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: { action?: string }) => {
          if (command === "control_settings_status")
            return structuredClone(harness.status);
          if (command === "control_settings_recovery") {
            harness.actions.push(args.action ?? "");
            if (harness.failAction)
              throw new Error("synthetic-private-payload");
            return null;
          }
          throw new Error("unexpected command");
        },
      },
    });
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
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    const { StartupGate } = await import("/src/components/StartupGate.tsx");
    function Child() {
      harness.childCount += 1;
      return React.createElement("p", null, "business-mounted");
    }
    const initialize = async () => {
      harness.initializeCount += 1;
      if (harness.failInitialize) throw new Error("synthetic-private-payload");
    };
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(
        React.StrictMode,
        null,
        React.createElement(
          StartupGate,
          { initialize },
          React.createElement(Child),
        ),
      ),
    );
  }, status);
}

test("recovery never mounts business UI and actions contain only the chosen fixed action", async ({
  page,
}) => {
  await mount(page, {
    phase: "recovery",
    reason: "settings_invalid",
    settings_editable: true,
  });
  await expect(
    page.getByText("Your existing data has been kept.", { exact: false }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => [
      window.__STARTUP_GATE_TEST__.initializeCount,
      window.__STARTUP_GATE_TEST__.childCount,
    ]),
  ).toEqual([0, 0]);
  await page.getByRole("button", { name: "Open data folder" }).click();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.actions),
  ).toEqual(["open_data_folder"]);
  await page.evaluate(() => {
    window.__STARTUP_GATE_TEST__.failAction = true;
  });
  await page.getByRole("button", { name: "Restart Inputia" }).click();
  await expect(page.getByRole("alert")).toHaveText(
    "The action could not be completed. Try again.",
  );
  await expect(page.locator("body")).not.toContainText(
    "synthetic-private-payload",
  );
});

test("starting waits for ready and bootstraps once under StrictMode", async ({
  page,
}) => {
  await mount(page, { phase: "starting" });
  await expect(page.getByText("Starting Inputia…")).toBeVisible();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.childCount),
  ).toBe(0);
  await page.evaluate(() => {
    window.__STARTUP_GATE_TEST__.status = { phase: "ready" };
  });
  await expect(page.getByText("business-mounted")).toBeVisible();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.initializeCount),
  ).toBe(1);
});

test("unknown status does not fall back to onboarding and a failed bootstrap can be checked again", async ({
  page,
}) => {
  await mount(page, { phase: "ready", unexpected: true });
  await expect(
    page.getByText(
      "The startup status could not be confirmed. Retry the check.",
    ),
  ).toBeVisible();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.childCount),
  ).toBe(0);
  await page.evaluate(() => {
    window.__STARTUP_GATE_TEST__.status = { phase: "ready" };
    window.__STARTUP_GATE_TEST__.failInitialize = true;
  });
  await page.getByRole("button", { name: "Check again" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.__STARTUP_GATE_TEST__.initializeCount),
    )
    .toBe(1);
  await expect(page.getByRole("button", { name: "Check again" })).toBeVisible();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.childCount),
  ).toBe(0);
  await page.evaluate(() => {
    window.__STARTUP_GATE_TEST__.failInitialize = false;
  });
  await page.getByRole("button", { name: "Check again" }).click();
  await expect(page.getByText("business-mounted")).toBeVisible();
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.initializeCount),
  ).toBe(2);
});

test("pending recovery does not tell the user to edit settings yet", async ({
  page,
}) => {
  await mount(page, {
    phase: "recovery",
    reason: "migration_recovery_required",
    settings_editable: false,
  });
  await expect(
    page.getByText("Recovery is not finished.", { exact: false }),
  ).toBeVisible();
  await expect(page.locator("body")).not.toContainText(
    "Check the settings file",
  );
  expect(
    await page.evaluate(() => window.__STARTUP_GATE_TEST__.childCount),
  ).toBe(0);
});
