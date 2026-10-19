import { expect, test, type Page } from "@playwright/test";

type PermissionSnapshot = {
  gate_open: boolean;
  probe_in_flight: boolean;
  recovery_pending?: boolean;
  maintenance_state: "active" | "retiring" | "restart_required" | "inactive";
  permission: { accessibility: string; input_monitoring: string };
  health: { enigo: string; shortcuts: string; targets: string };
  components: Record<string, never>;
};

type PermissionHelpHarness = {
  status: PermissionSnapshot;
  resumeStatus: PermissionSnapshot;
  maintenanceStatus: PermissionSnapshot;
  failResume: boolean;
  calls: string[];
};

declare global {
  interface Window {
    __INPUTIA_PERMISSION_HELP__: PermissionHelpHarness;
  }
}

const stopped: PermissionSnapshot = {
  gate_open: false,
  probe_in_flight: false,
  maintenance_state: "inactive",
  permission: { accessibility: "granted", input_monitoring: "granted" },
  health: { enigo: "stopped", shortcuts: "stopped", targets: "stopped" },
  components: {},
};
const ready: PermissionSnapshot = {
  ...stopped,
  gate_open: true,
  health: { enigo: "ready", shortcuts: "ready", targets: "ready" },
};

async function mountPermissionHelp(
  page: Page,
  status: PermissionSnapshot,
  options: Partial<
    Pick<
      PermissionHelpHarness,
      "resumeStatus" | "maintenanceStatus" | "failResume"
    >
  > = {},
) {
  await page.route("**/__permission-help", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__permission-help");
  await page.evaluate(
    async ({ status, options }) => {
      const harness: PermissionHelpHarness = {
        status,
        resumeStatus: options.resumeStatus ?? {
          ...status,
          probe_in_flight: true,
        },
        maintenanceStatus: options.maintenanceStatus ?? status,
        failResume: options.failResume ?? false,
        calls: [],
      };
      Object.assign(window, {
        __INPUTIA_PERMISSION_HELP__: harness,
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args?: { action?: string }) => {
            harness.calls.push(`${command}:${args?.action ?? ""}`);
            if (command === "inputia_permission_status") return harness.status;
            if (command === "inputia_permission_resume") {
              if (harness.failResume) throw new Error("resume failed");
              harness.status = harness.resumeStatus;
              return harness.status;
            }
            if (command === "inputia_permission_prepare_maintenance") {
              harness.status = harness.maintenanceStatus;
              return harness.status;
            }
            return null;
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
    },
    { status, options },
  );
  await expect(page.getByText("Inputia 权限", { exact: true })).toBeVisible();
}

function background(page: Page) {
  return page.locator('[aria-live="polite"]');
}

async function publishStatus(page: Page, status: PermissionSnapshot) {
  await page.evaluate((next) => {
    window.__INPUTIA_PERMISSION_HELP__.status = next;
  }, status);
}

async function calls(page: Page) {
  return page.evaluate(() => window.__INPUTIA_PERMISSION_HELP__.calls);
}

test("停止且未探测时显示已暂停，不声称仍在检查", async ({ page }) => {
  await mountPermissionHelp(page, stopped);
  await expect(background(page)).toHaveText("Inputia 输入控制: 已暂停");
  await expect(page.getByText("正在检查或停止", { exact: false })).toHaveCount(
    0,
  );
});

for (const [name, status] of [
  ["权限探测", { ...stopped, probe_in_flight: true }],
  ["维护退休", { ...stopped, maintenance_state: "retiring" }],
  [
    "目标工作线程退休",
    { ...stopped, health: { ...stopped.health, targets: "retiring" } },
  ],
] as const) {
  test(`${name}进行中时显示正在检查或停止`, async ({ page }) => {
    await mountPermissionHelp(page, status);
    await expect(background(page)).toHaveText(
      "Inputia 输入控制: 正在检查或停止",
    );
  });
}

test("门关闭但无探测或退休证据时显示尚未确认", async ({ page }) => {
  await mountPermissionHelp(page, { ...ready, gate_open: false });
  await expect(background(page)).toHaveText("Inputia 输入控制: 尚未确认");
});

test("目标校验服务需要重启时不会显示检查中", async ({ page }) => {
  await mountPermissionHelp(page, {
    ...stopped,
    health: { ...stopped.health, targets: "restart_required" },
  });
  await expect(background(page)).toHaveText("Inputia 输入控制: 需要重启组件");
});

for (const immediate of [false, true]) {
  test(`恢复就绪后清理临时检查提示（立即就绪=${immediate}）`, async ({
    page,
  }) => {
    await mountPermissionHelp(page, stopped, {
      resumeStatus: immediate ? ready : { ...stopped, probe_in_flight: true },
    });
    await page
      .getByRole("button", { name: "重新检测并恢复", exact: true })
      .click();
    if (!immediate) {
      await expect(page.getByRole("status")).toContainText(
        "正在检查权限并重建输入控制",
      );
      await expect(background(page)).toHaveText(
        "Inputia 输入控制: 正在检查或停止",
      );
      await publishStatus(page, ready);
    }
    await expect(background(page)).toHaveText("Inputia 输入控制: 已就绪");
    await expect(page.getByRole("status")).toHaveCount(0);
    expect(
      (await calls(page)).filter((call) =>
        call.startsWith("inputia_permission_resume:"),
      ),
    ).toHaveLength(1);
  });
}

test("轮询恢复就绪不会吞掉真实操作失败", async ({ page }) => {
  await mountPermissionHelp(page, stopped, { failResume: true });
  await page
    .getByRole("button", { name: "重新检测并恢复", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText("无法打开，请重试");
  await publishStatus(page, ready);
  await expect(background(page)).toHaveText("Inputia 输入控制: 已就绪");
  await expect(page.getByRole("status")).toContainText("无法打开，请重试");
});

test("自动恢复退避期间保留检查提示，重试耗尽后显示已暂停", async ({ page }) => {
  const recovering = { ...stopped, recovery_pending: true };
  await mountPermissionHelp(page, stopped, { resumeStatus: recovering });
  await page
    .getByRole("button", { name: "重新检测并恢复", exact: true })
    .click();
  await expect(background(page)).toHaveText("Inputia 输入控制: 正在检查或停止");
  await expect(page.getByRole("status")).toContainText(
    "正在检查权限并重建输入控制",
  );
  await publishStatus(page, { ...recovering, probe_in_flight: true });
  await expect
    .poll(
      async () =>
        (await calls(page)).filter((call) =>
          call.startsWith("inputia_permission_status:"),
        ).length,
    )
    .toBeGreaterThan(1);
  await expect(page.getByRole("status")).toContainText(
    "正在检查权限并重建输入控制",
  );
  await publishStatus(page, { ...stopped, recovery_pending: false });
  await expect(background(page)).toHaveText("Inputia 输入控制: 已暂停");
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("资源退休失败仍有恢复计划时显示检查，计划耗尽后才要求重启", async ({
  page,
}) => {
  const recovering = {
    ...stopped,
    recovery_pending: true,
    health: { ...stopped.health, targets: "restart_required" },
  };
  await mountPermissionHelp(page, recovering, { resumeStatus: recovering });
  await page
    .getByRole("button", { name: "重新检测并恢复", exact: true })
    .click();
  await expect(background(page)).toHaveText("Inputia 输入控制: 正在检查或停止");
  await expect(page.getByRole("status")).toContainText(
    "正在检查权限并重建输入控制",
  );
  await publishStatus(page, { ...recovering, recovery_pending: false });
  await expect(background(page)).toHaveText("Inputia 输入控制: 需要重启组件");
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("维护需要重启时优先于旧恢复标记", async ({ page }) => {
  const maintenanceRestart: PermissionSnapshot = {
    ...stopped,
    recovery_pending: true,
    maintenance_state: "restart_required",
  };
  await mountPermissionHelp(page, maintenanceRestart, {
    resumeStatus: maintenanceRestart,
  });
  await page
    .getByRole("button", { name: "重新检测并恢复", exact: true })
    .click();
  await expect(background(page)).toHaveText("Inputia 输入控制: 需要重启组件");
  await expect(page.getByRole("status")).toHaveCount(0);
});

for (const [name, terminal, label] of [
  ["停止", stopped, "已暂停"],
  [
    "重启",
    {
      ...stopped,
      probe_in_flight: true,
      health: { ...stopped.health, targets: "restart_required" },
    },
    "需要重启组件",
  ],
  [
    "拒权",
    {
      ...stopped,
      probe_in_flight: true,
      recovery_pending: true,
      permission: { accessibility: "denied", input_monitoring: "granted" },
    },
    "权限不可用",
  ],
  [
    "维护",
    {
      ...stopped,
      probe_in_flight: true,
      recovery_pending: true,
      maintenance_state: "active",
    },
    "已暂停",
  ],
] as const) {
  test(`恢复进入${name}终态后清理临时检查提示`, async ({ page }) => {
    await mountPermissionHelp(page, stopped, {
      resumeStatus: { ...stopped, probe_in_flight: true },
    });
    await page
      .getByRole("button", { name: "重新检测并恢复", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText(
      "正在检查权限并重建输入控制",
    );
    await publishStatus(page, terminal);
    await expect(background(page)).toHaveText(`Inputia 输入控制: ${label}`);
    await expect(page.getByRole("status")).toHaveCount(0);
  });
}

test("恢复立即返回拒权时不显示检查提示", async ({ page }) => {
  const denied = {
    ...stopped,
    permission: { accessibility: "denied", input_monitoring: "granted" },
  };
  await mountPermissionHelp(page, denied, { resumeStatus: denied });
  await page
    .getByRole("button", { name: "重新检测并恢复", exact: true })
    .click();
  await expect(background(page)).toHaveText("Inputia 输入控制: 权限不可用");
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("确认维护停止后才打开设置，导航不请求系统授权", async ({ page }) => {
  await mountPermissionHelp(page, ready, {
    maintenanceStatus: { ...stopped, maintenance_state: "active" },
  });
  await page
    .getByRole("button", { name: "暂停输入控制后打开权限设置", exact: true })
    .click();
  await expect(background(page)).toHaveText("Inputia 输入控制: 已暂停");
  await expect(page.getByRole("status")).toContainText("输入控制已暂停");
  const observed = await calls(page);
  expect(
    observed.filter(
      (call) =>
        call.startsWith("inputia_permission_prepare_maintenance:") ||
        call.startsWith("open_inputia_permission_help:"),
    ),
  ).toEqual([
    "inputia_permission_prepare_maintenance:",
    "open_inputia_permission_help:settings",
  ]);
  expect(
    observed.some(
      (call) =>
        call.includes("request_accessibility_permission") ||
        call.includes("request_microphone_permission"),
    ),
  ).toBe(false);
});
