import { expect, test } from "@playwright/test";

test("permission navigation never requests grants and reports a missing component", async ({
  page,
}) => {
  await page.route("**/__permission-help", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__permission-help");
  await page.evaluate(async () => {
    const calls: string[] = [];
    Object.assign(window, {
      permissionHelpCalls: calls,
      __TAURI_INTERNALS__: {
        invoke: async (command: string, args: { action: string }) => {
          calls.push(`${command}:${args.action}`);
          if (args.action === "component") throw "component_missing";
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
    await i18n.changeLanguage("zh");
    const { InputiaPermissionHelp } = await import(
      "/src/components/settings/general/InputiaPermissionHelp.tsx"
    );
    ReactDOM.createRoot(document.getElementById("root")!).render(
      React.createElement(InputiaPermissionHelp),
    );
  });
  await expect(page.getByText("Inputia 权限", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "定位输入法组件" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "候选版不会回退到日常输入法",
  );
  await page.getByRole("button", { name: "打开权限设置" }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(
    await page.evaluate(() => Reflect.get(window, "permissionHelpCalls")),
  ).toEqual([
    "get_app_settings:undefined",
    "open_inputia_permission_help:component",
    "open_inputia_permission_help:settings",
  ]);
  await page.screenshot({
    path: "test-results/inputia-permission-help.png",
    fullPage: true,
  });
});
