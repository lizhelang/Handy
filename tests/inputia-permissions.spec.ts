import { expect, test, type Page } from "@playwright/test";

type PermissionHarness = {
  accessibilityChecks: Array<boolean | "reject">;
  microphoneChecks: Array<boolean | "reject">;
  calls: string[];
  completed: boolean;
  rejectDeviceRefresh: boolean;
  failInitializeCount: number;
};

declare global {
  interface Window {
    __INPUTIA_PERMISSIONS__: PermissionHarness;
  }
}

async function mountPermissions(
  page: Page,
  accessibilityChecks: PermissionHarness["accessibilityChecks"],
  microphoneChecks: PermissionHarness["microphoneChecks"],
  options: { rejectDeviceRefresh?: boolean; failInitializeCount?: number } = {},
) {
  await page.route("**/__inputia-permissions-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__inputia-permissions-test");
  await page.evaluate(
    async ({
      accessibilityChecks,
      microphoneChecks,
      rejectDeviceRefresh,
      failInitializeCount,
    }) => {
      const harness: PermissionHarness = {
        accessibilityChecks: [...accessibilityChecks],
        microphoneChecks: [...microphoneChecks],
        calls: [],
        completed: false,
        rejectDeviceRefresh,
        failInitializeCount,
      };
      Object.assign(window, {
        __INPUTIA_PERMISSIONS__: harness,
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "macos",
          os_type: "macos",
          family: "unix",
          version: "15.0",
          arch: "aarch64",
          eol: "\n",
          exe_extension: "",
        },
        __TAURI_INTERNALS__: {
          invoke: async (command: string) => {
            harness.calls.push(command);
            if (command === "plugin:app|name") {
              return "Inputia Candidate";
            }
            if (
              command ===
              "plugin:macos-permissions|check_accessibility_permission"
            ) {
              const result = harness.accessibilityChecks.shift() ?? false;
              if (result === "reject") throw new Error("accessibility busy");
              return result;
            }
            if (
              command === "plugin:macos-permissions|check_microphone_permission"
            ) {
              const result = harness.microphoneChecks.shift() ?? false;
              if (result === "reject") throw new Error("microphone busy");
              return result;
            }
            if (
              command ===
                "plugin:macos-permissions|request_accessibility_permission" ||
              command ===
                "plugin:macos-permissions|request_microphone_permission"
            ) {
              return null;
            }
            if (command === "initialize_enigo") {
              if (harness.failInitializeCount > 0) {
                harness.failInitializeCount -= 1;
                throw "init failed";
              }
              return null;
            }
            if (command === "initialize_shortcuts") {
              return [];
            }
            if (
              command === "get_available_microphones" ||
              command === "get_available_output_devices"
            ) {
              if (harness.rejectDeviceRefresh) {
                throw new Error("device refresh failed");
              }
              return [];
            }
            throw new Error(`Unhandled command: ${command}`);
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
      if (rejectDeviceRefresh) {
        const { useSettingsStore } = await import(
          "/src/stores/settingsStore.ts"
        );
        useSettingsStore.setState({
          refreshAudioDevices: async () => {
            throw new Error("audio refresh failed");
          },
          refreshOutputDevices: async () => {
            throw new Error("output refresh failed");
          },
        });
      }
      const { default: AccessibilityOnboarding } = await import(
        "/src/components/onboarding/AccessibilityOnboarding.tsx"
      );
      ReactDOM.createRoot(document.getElementById("root")!).render(
        React.createElement(AccessibilityOnboarding, {
          onComplete: () => {
            harness.completed = true;
          },
        }),
      );
    },
    {
      accessibilityChecks,
      microphoneChecks,
      rejectDeviceRefresh: options.rejectDeviceRefresh ?? false,
      failInitializeCount: options.failInitializeCount ?? 0,
    },
  );
}

async function calls(page: Page) {
  return page.evaluate(() => window.__INPUTIA_PERMISSIONS__.calls);
}

test.describe("Inputia permissions onboarding", () => {
  test("blocked onboarding exposes navigation without granting permissions", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 800, height: 600 });
    await mountPermissions(page, [false], [false]);
    await page.getByRole("button", { name: "Locate input method" }).click();
    await page
      .getByRole("button", { name: "Open permission settings" })
      .click();
    const observed = await calls(page);
    expect(
      observed.filter((command) => command === "open_inputia_permission_help"),
    ).toHaveLength(2);
    expect(
      observed.some(
        (command) =>
          command.includes("request_accessibility_permission") ||
          command.includes("request_microphone_permission"),
      ),
    ).toBe(false);
    expect(
      await page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed),
    ).toBe(false);
    await expect(
      page.getByRole("button", { name: "Check Again" }),
    ).toBeEnabled();
  });
  test("keeps one granted permission when the other check fails", async ({
    page,
  }) => {
    await mountPermissions(page, [true], ["reject"]);

    await expect(page.getByText("Inputia Candidate")).toBeVisible();
    await expect(page.getByText("Granted")).toBeVisible();
    await expect(
      page.getByText("Failed to check permissions. Please try again."),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Grant Permission" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Check Again" }),
    ).toBeVisible();
    expect(
      await page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed),
    ).toBe(false);
    expect(await calls(page)).not.toContain(
      "plugin:macos-permissions|request_microphone_permission",
    );
    expect(await calls(page)).not.toContain("initialize_enigo");
    expect(await calls(page)).not.toContain("initialize_shortcuts");
  });

  test("recheck is read-only and does not bypass the permission gate", async ({
    page,
  }) => {
    await mountPermissions(page, [false, false], [false, false]);
    await page.getByRole("button", { name: "Check Again" }).click();

    await expect(
      page.getByRole("button", { name: "Grant Permission" }),
    ).toHaveCount(2);
    expect(
      await page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed),
    ).toBe(false);
    expect(await calls(page)).not.toContain(
      "plugin:macos-permissions|request_accessibility_permission",
    );
    expect(await calls(page)).not.toContain(
      "plugin:macos-permissions|request_microphone_permission",
    );
    expect(await calls(page)).not.toContain("initialize_enigo");
    expect(await calls(page)).not.toContain("initialize_shortcuts");
  });

  test("recheck initializes and completes after permissions recover", async ({
    page,
  }) => {
    await mountPermissions(page, [false, true], [false, true]);
    await page.getByRole("button", { name: "Check Again" }).click();

    await expect(page.getByText("All set!")).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed))
      .toBe(true);
    expect(await calls(page)).toContain("initialize_enigo");
    expect(await calls(page)).toContain("initialize_shortcuts");
  });

  test("recheck handles completion failures without an unhandled rejection", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await mountPermissions(page, [false, true], [false, true], {
      rejectDeviceRefresh: true,
    });

    await page.getByRole("button", { name: "Check Again" }).click();

    await expect(
      page.getByText(
        "Permissions are confirmed, but input services could not initialize. Check again.",
      ),
    ).toBeVisible();
    expect(
      await page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed),
    ).toBe(false);
    expect(pageErrors).toEqual([]);
    expect(await calls(page)).toContain("initialize_enigo");
    expect(await calls(page)).toContain("initialize_shortcuts");
  });

  test("recheck can recover after input service initialization fails", async ({
    page,
  }) => {
    await mountPermissions(page, [false, true, true], [false, true, true], {
      failInitializeCount: 1,
    });

    await page.getByRole("button", { name: "Check Again" }).click();

    await expect(
      page.getByText(
        "Permissions are confirmed, but input services could not initialize. Check again.",
      ),
    ).toBeVisible();
    expect(
      await page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed),
    ).toBe(false);

    await page.getByRole("button", { name: "Check Again" }).click();

    await expect(page.getByText("All set!")).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => window.__INPUTIA_PERMISSIONS__.completed))
      .toBe(true);
  });
});
