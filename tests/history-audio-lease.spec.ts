import { test, expect, type Page } from "@playwright/test";

declare global {
  interface Window {
    __AUDIO_LEASE_TEST__: {
      calls: { command: string; args?: Record<string, unknown> }[];
      defer: boolean;
      rejectPlay: boolean;
      rejectCancelOnce: boolean;
      plays: number;
      mediaEvents: string[];
      resolve: () => void;
      unmount: () => void;
    };
  }
}

// 真实 React 播放组件和 typed invoke wrapper；模拟媒体/Tauri，不冒充原生附件 GC 验收。
async function mount(page: Page) {
  await page.route("**/__audio-lease-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><body><div id="root"></div></body></html>',
    }),
  );
  await page.goto("/__audio-lease-test");
  await page.evaluate(async () => {
    const harness = {
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      defer: false,
      rejectPlay: false,
      rejectCancelOnce: false,
      plays: 0,
      mediaEvents: [] as string[],
      resolve: () => {},
      unmount: () => {},
    };
    Object.assign(window, {
      __AUDIO_LEASE_TEST__: harness,
      __TAURI_INTERNALS__: {
        convertFileSrc: () => "data:audio/wav;base64,UklGRg==",
        invoke: async (command: string, args?: Record<string, unknown>) => {
          harness.calls.push({ command, args });
          if (command === "get_app_settings") return { app_language: "en" };
          if (command === "acquire_history_attachment") {
            if (harness.defer)
              await new Promise<void>((resolve) => {
                harness.resolve = resolve;
              });
            return {
              lease: {
                lease_id: String(args?.operationId),
                instance_id: "process",
                attachment_id: "asset",
                purpose: "active",
              },
              path: "/fixture/voice.wav",
              revision: 1,
            };
          }
          if (command === "release_history_attachment_operation") {
            harness.mediaEvents.push("release");
            if (harness.rejectCancelOnce) {
              harness.rejectCancelOnce = false;
              throw new Error("busy");
            }
            return;
          }
          throw new Error("unexpected command");
        },
      },
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    HTMLMediaElement.prototype.play = async function () {
      harness.plays++;
      if (harness.rejectPlay) throw new Error("media unavailable");
      Object.defineProperty(this, "paused", {
        configurable: true,
        value: false,
      });
      this.dispatchEvent(new Event("play"));
    };
    HTMLMediaElement.prototype.pause = function () {
      harness.mediaEvents.push("pause");
      Object.defineProperty(this, "paused", {
        configurable: true,
        value: true,
      });
      this.dispatchEvent(new Event("pause"));
    };
    HTMLMediaElement.prototype.load = function () {
      harness.mediaEvents.push(
        this.hasAttribute("src") ? "load-source" : "load-empty",
      );
    };
    // @ts-expect-error 浏览器通过 Vite 动态载入实际组件。
    const { default: React } = await import(
      "/node_modules/.vite/deps/react.js"
    );
    // @ts-expect-error 浏览器通过 Vite 动态载入实际组件。
    const { default: ReactDOM } = await import(
      "/node_modules/.vite/deps/react-dom_client.js"
    );
    // @ts-expect-error 浏览器通过 Vite 动态载入实际组件。
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    // @ts-expect-error 浏览器通过 Vite 动态载入实际组件。
    const { AudioPlayer } = await import("/src/components/ui/AudioPlayer.tsx");
    // @ts-expect-error 浏览器通过 Vite 动态载入实际组件。
    const { acquireHistoryAudio } = await import(
      "/src/lib/historyAttachment.ts"
    );
    const root = ReactDOM.createRoot(document.getElementById("root"));
    root.render(
      React.createElement(AudioPlayer, {
        onLoadRequest: (signal: AbortSignal) =>
          acquireHistoryAudio(7, "macos", signal),
      }),
    );
    harness.unmount = () => root.unmount();
  });
  await expect(
    page.getByRole("button", { name: "Play recording" }),
  ).toBeVisible();
}
const calls = async (page: Page, command: string) =>
  page.evaluate(
    (command) =>
      window.__AUDIO_LEASE_TEST__.calls.filter((c) => c.command === command),
    command,
  );

test("暂停释放来源，继续播放使用新租约，结束与卸载均释放", async ({ page }) => {
  await mount(page);
  await page.getByRole("button", { name: "Play recording" }).click();
  await page.getByRole("button", { name: "Pause recording" }).click();
  await expect
    .poll(
      async () =>
        (await calls(page, "release_history_attachment_operation")).length,
    )
    .toBe(1);
  await expect(page.locator("audio")).not.toHaveAttribute("src");
  await page.getByRole("button", { name: "Play recording" }).click();
  const acquired = await calls(page, "acquire_history_attachment");
  expect(acquired).toHaveLength(2);
  expect(acquired[0].args?.operationId).not.toBe(acquired[1].args?.operationId);
  await page.locator("audio").dispatchEvent("ended");
  await expect
    .poll(
      async () =>
        (await calls(page, "release_history_attachment_operation")).length,
    )
    .toBe(2);
  await page.getByRole("button", { name: "Play recording" }).click();
  const detached = await page.evaluate(() => {
    const audio = document.querySelector("audio")!;
    window.__AUDIO_LEASE_TEST__.mediaEvents = [];
    window.__AUDIO_LEASE_TEST__.unmount();
    return {
      paused: audio.paused,
      hasSource: audio.hasAttribute("src"),
      events: window.__AUDIO_LEASE_TEST__.mediaEvents,
    };
  });
  expect(detached).toEqual({
    paused: true,
    hasSource: false,
    events: ["pause", "load-empty", "release"],
  });
  await expect
    .poll(
      async () =>
        (await calls(page, "release_history_attachment_operation")).length,
    )
    .toBe(3);
});

test("卸载取消尚未返回的获取操作，迟到成功不会播放", async ({ page }) => {
  await mount(page);
  await page.evaluate(() => {
    window.__AUDIO_LEASE_TEST__.defer = true;
  });
  await page.getByRole("button", { name: "Play recording" }).click();
  await expect
    .poll(async () => (await calls(page, "acquire_history_attachment")).length)
    .toBe(1);
  await page.evaluate(() => window.__AUDIO_LEASE_TEST__.unmount());
  await expect
    .poll(
      async () =>
        (await calls(page, "release_history_attachment_operation")).length,
    )
    .toBe(1);
  await page.evaluate(() => window.__AUDIO_LEASE_TEST__.resolve());
  expect(await page.evaluate(() => window.__AUDIO_LEASE_TEST__.plays)).toBe(0);
});

test("播放失败保留重试入口，暂时失败的释放会按同操作重试", async ({ page }) => {
  await mount(page);
  await page.evaluate(() => {
    window.__AUDIO_LEASE_TEST__.rejectPlay = true;
    window.__AUDIO_LEASE_TEST__.rejectCancelOnce = true;
  });
  await page.getByRole("button", { name: "Play recording" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Play recording" }),
  ).toBeEnabled();
  await expect
    .poll(
      async () =>
        (await calls(page, "release_history_attachment_operation")).length,
      { timeout: 5000 },
    )
    .toBe(2);
  const released = await calls(page, "release_history_attachment_operation");
  expect(released[0].args?.operationId).toBe(released[1].args?.operationId);
});
