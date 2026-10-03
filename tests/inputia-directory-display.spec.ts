import { expect, test } from "@playwright/test";

for (const language of ["en", "zh"] as const) {
  for (const failure of [false, true]) {
    test(`directory locations preserve legacy routing without exposing it: ${language}, failure=${failure}`, async ({
      page,
    }) => {
      await page.route("**/__inputia-directory-test", (route) =>
        route.fulfill({
          contentType: "text/html",
          body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
        }),
      );
      await page.goto("/__inputia-directory-test");
      await page.evaluate(
        async ({ language, failure }) => {
          const calls: string[] = [];
          const legacyPath = "/test/HandyUnifiedCandidate/trial-test/Handy";
          const { default: RefreshRuntime } = await import("/@react-refresh");
          RefreshRuntime.injectIntoGlobalHook(window);
          Object.assign(window, {
            $RefreshReg$: () => {},
            $RefreshSig$: () => (value: unknown) => value,
            __vite_plugin_react_preamble_installed__: true,
            __DIRECTORY_TEST_CALLS__: calls,
            __TAURI_INTERNALS__: {
              invoke: async (command: string, args?: { paths: string[] }) => {
                calls.push(command);
                if (command === "plugin:path|join")
                  return args!.paths.join("/");
                if (
                  command === "get_app_dir_path" ||
                  command === "get_log_dir_path"
                ) {
                  if (failure) throw new Error(`Unavailable: ${legacyPath}`);
                  return command === "get_app_dir_path"
                    ? legacyPath
                    : `${legacyPath}/logs`;
                }
                return null;
              },
            },
          });
          const { default: i18n } = await import("/src/i18n/index.ts");
          await i18n.changeLanguage(language);
          const { default: React } = await import(
            "/node_modules/.vite/deps/react.js"
          );
          const { default: ReactDOM } = await import(
            "/node_modules/.vite/deps/react-dom_client.js"
          );
          const { AppDataDirectory } = await import(
            "/src/components/settings/AppDataDirectory.tsx"
          );
          const { LogDirectory } = await import(
            "/src/components/settings/debug/LogDirectory.tsx"
          );
          const { DebugPaths } = await import(
            "/src/components/settings/debug/DebugPaths.tsx"
          );
          await i18n.changeLanguage(language);
          ReactDOM.createRoot(document.getElementById("root")!).render(
            React.createElement(
              React.Fragment,
              null,
              React.createElement(AppDataDirectory),
              React.createElement(LogDirectory),
              React.createElement(DebugPaths),
            ),
          );
        },
        { language, failure },
      );
      await page.waitForLoadState("networkidle");
      if (failure) {
        await expect(
          page.getByText(
            language === "en"
              ? "Directory unavailable. Try opening this page again."
              : "目录暂不可用，请重新打开此页面。",
            { exact: true },
          ),
        ).toHaveCount(3);
      } else {
        const open = page.getByRole("button", {
          name: language === "en" ? "Open" : "打开",
          exact: true,
        });
        await expect(open).toHaveCount(2);
        await open.nth(0).click();
        await open.nth(1).click();
        const calls = await page.evaluate(
          () =>
            (
              window as unknown as {
                __DIRECTORY_TEST_CALLS__: string[];
              }
            ).__DIRECTORY_TEST_CALLS__,
        );
        expect(calls).toContain("open_app_data_dir");
        expect(calls).toContain("open_log_dir");
        await expect(page.getByText(/settings_store\.json/)).toBeVisible();
      }
      // 包括错误提示、tooltip 和 DOM 属性，不能把兼容路径当品牌继续露出。
      expect(await page.locator("#root").innerHTML()).not.toMatch(/handy/i);
    });
  }
}
