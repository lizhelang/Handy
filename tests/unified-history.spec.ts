import { expect, test, type Page } from "@playwright/test";
import type { UnifiedHistoryItem } from "../src/bindings";

interface HistoryHarness {
  calls: { command: string; args?: Record<string, unknown> }[];
  actions: {
    action: string;
    item: UnifiedHistoryItem;
    patch?: unknown;
    operationId?: string;
  }[];
  items: UnifiedHistoryItem[];
  output: string;
  rejectMutation: boolean;
  rejectListener: boolean;
  recordingFailure: {
    operation: "retranscribe" | "openRecordings";
    message?: string;
  } | null;
  deferRetranscribe: boolean;
  audioUrl?: string;
  pending: Map<string, (value: unknown) => void>;
  deferQueries: boolean;
  emit: () => void;
  listeners: Map<number, number>;
  unmount: () => void;
  remount: () => void;
}
declare global {
  interface Window {
    __HISTORY_TEST__: HistoryHarness;
  }
}

// Browser mocks verify UI behavior only; they do not prove native insertion.
async function mountHistory(page: Page, recordingsOnly = false) {
  await page.route("**/__unified-history-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<html><head><link rel="stylesheet" href="/src/App.css"></head><body><div id="root" style="height:95vh;padding:24px"></div></body></html>',
    }),
  );
  await page.goto("/__unified-history-test");
  await page.evaluate(async (recordingsOnly) => {
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<number, number>();
    let next = 0;
    const fixture: UnifiedHistoryItem = {
      item_id: "voice-1",
      store_id: "voice-store",
      record_id: "1",
      revision: 1,
      source_kind: "voice",
      content_type: "text",
      text: "Voice fixture",
      title: null,
      starred: false,
      pinned: false,
      created_at_ms: 1000000,
      asset_ref: null,
      source_app: null,
    };
    const harness = {
      audioUrl: undefined as string | undefined,
      calls: [] as { command: string; args?: Record<string, unknown> }[],
      actions: [] as {
        action: string;
        item: typeof fixture;
        patch?: unknown;
        operationId?: string;
      }[],
      items: [
        fixture,
        {
          ...fixture,
          item_id: "file-2",
          record_id: "2",
          source_kind: "clipboard",
          content_type: "files",
          text: '["/synthetic/report.pdf"]',
        },
      ],
      output: "confirmed",
      rejectMutation: false,
      rejectListener: false,
      recordingFailure: null as HistoryHarness["recordingFailure"],
      deferRetranscribe: false,
      pending: new Map<string, (value: unknown) => void>(),
      deferQueries: false,
      emit: () => {
        for (const [id, handler] of listeners)
          callbacks.get(handler)?.({
            event: "unified-history-update",
            id,
            payload: { generation: 2 },
          });
      },
      listeners,
    };
    Object.assign(window, {
      __HISTORY_TEST__: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: {
        unregisterListener: (_event: string, id: number) =>
          listeners.delete(id),
      },
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (value: unknown) => void) => {
          const id = ++next;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: async (command: string, args?: Record<string, unknown>) => {
          harness.calls.push({ command, args });
          if (command === "plugin:event|listen") {
            if (harness.rejectListener) throw "listener unavailable";
            const id = ++next;
            listeners.set(id, Number(args?.handler));
            return id;
          }
          if (command === "plugin:event|unlisten") {
            listeners.delete(Number(args?.eventId));
            return;
          }
          if (command === "refresh_unified_history") return 2;
          if (command === "get_unified_history_revisions")
            return [
              { revision: 1, text: "Original revision", asset_ref: null },
            ];
          if (command === "get_unified_history") {
            const query = args?.query as {
              search: string | null;
              source_kind: string | null;
              content_type: string | null;
              starred_only: boolean;
              offset: number;
              limit: number;
            };
            if (harness.deferQueries)
              return new Promise((resolve) =>
                harness.pending.set(query.search || "", resolve),
              );
            return harness.items
              .filter(
                (item) =>
                  (!query.search || item.text?.includes(query.search)) &&
                  (!query.source_kind ||
                    item.source_kind === query.source_kind) &&
                  (!query.content_type ||
                    item.content_type === query.content_type) &&
                  (!query.starred_only || item.starred),
              )
              .slice(query.offset, query.offset + query.limit);
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
    // Vite's refresh preamble is needed when loading a TSX module in this isolated harness.
    const { default: RefreshRuntime } = await import("/@react-refresh");
    RefreshRuntime.injectIntoGlobalHook(window);
    Object.assign(window, {
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    });
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("en");
    const { UnifiedHistory } = await import(
      "/src/components/history/UnifiedHistory.tsx"
    );
    const { requireActiveOutputAttempt } = await import(
      "/src/stores/unifiedOutputStore.ts"
    );
    let root = ReactDOM.createRoot(document.getElementById("root")!);
    const render = () =>
      root.render(
        React.createElement(UnifiedHistory, {
          recordingsOnly,
          resolveAsset: async (item: typeof fixture) =>
            item.asset_ref
              ? (harness.audioUrl ??
                `data:audio/wav;base64,UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=`)
              : null,
          onCheckOutputReceipt: async (operationId: string) => {
            harness.calls.push({
              command: "get_unified_output_receipt",
              args: { operationId },
            });
            return harness.output === "missing"
              ? null
              : { status: harness.output };
          },
          onCopy: async (item: typeof fixture) => {
            harness.actions.push({
              action: "copy",
              item,
              operationId: requireActiveOutputAttempt(item.item_id, "copy")
                .operationId,
            });
            return { status: harness.output };
          },
          onInsert: async (item: typeof fixture) => {
            harness.actions.push({
              action: "insert",
              item,
              operationId: requireActiveOutputAttempt(item.item_id, "insert")
                .operationId,
            });
            if (harness.output === "defer")
              return new Promise((resolve) =>
                harness.pending.set("output", resolve),
              );
            if (harness.output === "throw") throw new Error("response lost");
            return { status: harness.output };
          },
          onUpdate: async (item: typeof fixture, patch: unknown) => {
            harness.actions.push({ action: "update", item, patch });
            if (harness.rejectMutation) throw new Error("conflict");
            harness.items = harness.items.map((entry) =>
              entry.item_id === item.item_id
                ? {
                    ...entry,
                    ...(patch as object),
                    revision: entry.revision + 1,
                  }
                : entry,
            );
          },
          onRetranscribe: async (item: typeof fixture) => {
            harness.calls.push({
              command: "retranscribe",
              args: { itemId: item.item_id, revision: item.revision },
            });
            if (harness.deferRetranscribe)
              await new Promise<void>((resolve) =>
                harness.pending.set("retranscribe", () => resolve()),
              );
            if (harness.recordingFailure?.operation === "retranscribe") {
              if (harness.recordingFailure.message)
                throw new Error(harness.recordingFailure.message);
              throw { code: "unknown" };
            }
            harness.items = harness.items.map((entry) =>
              entry.item_id === item.item_id
                ? {
                    ...entry,
                    text: "Recovered transcription",
                    revision: entry.revision + 1,
                  }
                : entry,
            );
          },
          onOpenRecordings: async () => {
            harness.calls.push({ command: "openRecordings" });
            if (harness.recordingFailure?.operation === "openRecordings") {
              if (harness.recordingFailure.message)
                throw new Error(harness.recordingFailure.message);
              throw { code: "unknown" };
            }
          },
        }),
      );
    Object.assign(harness, {
      unmount: () => root.unmount(),
      remount: () => {
        root.unmount();
        root = ReactDOM.createRoot(document.getElementById("root")!);
        render();
      },
    });
    render();
  }, recordingsOnly);
  await expect(page.getByText("Voice fixture", { exact: true })).toBeVisible();
}

test("single click previews; explicit copy preserves the file item and does not insert", async ({
  page,
}, testInfo) => {
  await mountHistory(page);
  await page.getByText("report.pdf", { exact: true }).click();
  await expect(
    page.getByRole("complementary", { name: "Preview" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  await expect(page.getByText("Copied.", { exact: true })).toBeVisible();
  const screenshot = testInfo.outputPath("unified-history-preview.png");
  await page.screenshot({ path: screenshot });
  await testInfo.attach(
    "Browser interaction preview (not native insertion evidence)",
    { path: screenshot, contentType: "image/png" },
  );
  expect(
    await page.evaluate(() =>
      window.__HISTORY_TEST__.actions.map((entry) => [
        entry.action,
        entry.item.content_type,
      ]),
    ),
  ).toEqual([["copy", "files"]]);
});

test("重转录失败持续显示后端原因，并允许再次重试成功", async ({ page }) => {
  await page.clock.install();
  await mountHistory(page);
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    harness.items[0] = { ...harness.items[0], asset_ref: "recording-1" };
    harness.recordingFailure = {
      operation: "retranscribe",
      message: "output truncated before end-of-stream: run: status 18",
    };
    harness.emit();
  });
  await page.getByText("Voice fixture", { exact: true }).click();
  const retry = page.getByRole("button", {
    name: "Re-transcribe",
    exact: true,
  });
  await retry.click();
  const failure = page
    .getByRole("complementary", { name: "Preview" })
    .getByRole("alert");
  await expect(failure).toContainText(
    "Failed to re-transcribe. Please try again.",
  );
  await expect(failure).toContainText(
    "output truncated before end-of-stream: run: status 18",
  );
  await expect(page.getByText(/Could not save/)).toHaveCount(0);
  await expect(retry).toBeEnabled();
  await page.clock.fastForward(5000);
  await expect(failure).toBeVisible();
  expect(await page.evaluate(() => window.__HISTORY_TEST__.items[0].text)).toBe(
    "Voice fixture",
  );

  await page.evaluate(() => {
    window.__HISTORY_TEST__.recordingFailure = null;
  });
  await retry.click();
  await expect(
    page.getByText("Saved and synchronized.", { exact: true }),
  ).toBeVisible();
  await expect(failure).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      window.__HISTORY_TEST__.calls.filter(
        (call) => call.command === "retranscribe",
      ),
    ),
  ).toEqual([
    { command: "retranscribe", args: { itemId: "voice-1", revision: 1 } },
    { command: "retranscribe", args: { itemId: "voice-1", revision: 1 } },
  ]);
  expect(await page.evaluate(() => window.__HISTORY_TEST__.items[0].text)).toBe(
    "Recovered transcription",
  );
});

for (const outcome of ["failure", "success"] as const) {
  test(`切换历史条目后，迟到的重转录${outcome === "failure" ? "失败" : "成功"}不会误归属到新条目`, async ({
    page,
  }) => {
    await mountHistory(page);
    await page.evaluate((result) => {
      const harness = window.__HISTORY_TEST__;
      harness.items[0] = { ...harness.items[0], asset_ref: "recording-1" };
      harness.deferRetranscribe = true;
      if (result === "failure")
        harness.recordingFailure = {
          operation: "retranscribe",
          message: "late failure for voice-1",
        };
      harness.emit();
    }, outcome);
    await page.getByText("Voice fixture", { exact: true }).click();
    await page
      .getByRole("button", { name: "Re-transcribe", exact: true })
      .click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__HISTORY_TEST__.pending.has("retranscribe"),
        ),
      )
      .toBe(true);
    await page.getByText("report.pdf", { exact: true }).click();
    const selectedFile = page
      .getByRole("listitem")
      .filter({ hasText: "report.pdf" });
    await expect(selectedFile).toHaveAttribute("aria-expanded", "true");
    await page.evaluate(() =>
      window.__HISTORY_TEST__.pending.get("retranscribe")!(null),
    );
    await expect(
      page.getByRole("button", { name: "Open Recordings Folder" }),
    ).toBeEnabled();
    await expect(selectedFile).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(
      page.getByText("Saved and synchronized.", { exact: true }),
    ).toHaveCount(0);
    expect(
      await page.evaluate(() => window.__HISTORY_TEST__.items[0].text),
    ).toBe(outcome === "failure" ? "Voice fixture" : "Recovered transcription");
  });
}

test("未知录音操作错误按实际操作显示本地化提示", async ({ page }) => {
  await mountHistory(page);
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    harness.items[0] = { ...harness.items[0], asset_ref: "recording-1" };
    harness.recordingFailure = { operation: "retranscribe" };
    harness.emit();
  });
  await page.getByText("Voice fixture", { exact: true }).click();
  await page
    .getByRole("button", { name: "Re-transcribe", exact: true })
    .click();
  const preview = page.getByRole("complementary", { name: "Preview" });
  await expect(preview.getByRole("alert")).toHaveText(
    "Failed to re-transcribe. Please try again.",
  );

  await page.evaluate(() => {
    window.__HISTORY_TEST__.recordingFailure = {
      operation: "openRecordings",
    };
  });
  const open = page.getByRole("button", { name: "Open Recordings Folder" });
  await open.click();
  await expect(page.getByRole("alert")).toContainText("Open Recordings Folder");
  await expect(page.getByRole("alert")).toContainText(
    "The action failed. Your content is kept in history.",
  );
  await expect(preview.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText(/Could not save/)).toHaveCount(0);
  await expect(page.getByText("[object Object]")).toHaveCount(0);
  await expect(open).toBeEnabled();
});

test("switching voice previews keeps a single recording player", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    harness.items = [1, 2, 3, 4, 5, 6].map((n) => ({
      item_id: `voice-${n}`,
      store_id: "voice-store",
      record_id: String(n),
      revision: 1,
      source_kind: "voice",
      content_type: "text",
      text: `Voice recording ${n}`,
      title: null,
      starred: false,
      pinned: false,
      created_at_ms: 1000000 + n,
      asset_ref: `asset://${n}.wav`,
      source_app: null,
    }));
  });
  await page.getByRole("button", { name: "Refresh" }).click();
  for (const n of [1, 2, 3, 4, 5, 6]) {
    await page
      .getByRole("list", { name: "History" })
      .getByText(`Voice recording ${n}`, { exact: true })
      .click();
    const preview = page.getByRole("complementary", { name: "Preview" });
    await expect(preview).toBeVisible();
    await expect(
      preview.getByText(`Voice recording ${n}`, { exact: true }),
    ).toBeVisible();
    await expect
      .poll(async () =>
        page.evaluate(
          () => document.querySelectorAll("[data-history-recording]").length,
        ),
      )
      .toBe(1);
    await expect
      .poll(async () =>
        page.evaluate(() => document.querySelectorAll("audio").length),
      )
      .toBe(1);
    await expect(page.locator("audio[controls]")).toHaveCount(0);
  }
});

test("无转写正文的录音展开后读取真实时长，读取元数据不会自动播放", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    // 合成 159.24 秒 PCM16 WAV，不读取或保存用户录音。
    const sampleRate = 16000;
    const samples = Math.round(159.24 * sampleRate);
    const bytes = new ArrayBuffer(44 + samples * 2);
    const header = new DataView(bytes);
    const text = (offset: number, value: string) =>
      [...value].forEach((char, index) =>
        header.setUint8(offset + index, char.charCodeAt(0)),
      );
    text(0, "RIFF");
    header.setUint32(4, bytes.byteLength - 8, true);
    text(8, "WAVE");
    text(12, "fmt ");
    header.setUint32(16, 16, true);
    header.setUint16(20, 1, true);
    header.setUint16(22, 1, true);
    header.setUint32(24, sampleRate, true);
    header.setUint32(28, sampleRate * 2, true);
    header.setUint16(32, 2, true);
    header.setUint16(34, 16, true);
    text(36, "data");
    header.setUint32(40, samples * 2, true);
    const harness = window.__HISTORY_TEST__;
    harness.audioUrl = URL.createObjectURL(
      new Blob([bytes], { type: "audio/wav" }),
    );
    harness.items = [
      {
        ...harness.items[0],
        text: "",
        title: "Untranscribed recording",
        asset_ref: "asset://untranscribed.wav",
      },
    ];
  });
  await page.getByRole("button", { name: "Refresh" }).click();
  await page.getByText("Untranscribed recording", { exact: true }).click();
  const player = page.locator("[data-history-recording]");
  await expect(player.getByText("2:39", { exact: true })).toBeVisible();
  await expect(player.getByText("0:00", { exact: true })).toHaveCount(1);
  await expect(player.getByRole("slider")).toHaveAttribute("max", "159.24");
  expect(
    await page.locator("audio").evaluate((audio: HTMLAudioElement) => ({
      paused: audio.paused,
      currentTime: audio.currentTime,
      duration: audio.duration,
    })),
  ).toEqual({ paused: true, currentTime: 0, duration: 159.24 });
});

test("录音元数据读取失败显示错误和未知时长，不伪报为零秒", async ({ page }) => {
  await mountHistory(page);
  await page.route("**/__missing-recording.wav", (route) => route.abort());
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    harness.audioUrl = "/__missing-recording.wav";
    harness.items = [
      {
        ...harness.items[0],
        asset_ref: "asset://missing.wav",
      },
    ];
  });
  await page.getByRole("button", { name: "Refresh" }).click();
  await page.getByText("Voice fixture", { exact: true }).click();
  const player = page.locator("[data-history-recording]");
  await expect(player.getByRole("alert")).toBeVisible();
  await expect(player.getByText("—:—", { exact: true })).toBeVisible();
  await expect(player.getByText("0:00", { exact: true })).toHaveCount(1);
  await expect(player.getByRole("slider")).toBeDisabled();
  await expect(
    player.getByRole("button", { name: "Play recording" }),
  ).toBeEnabled();
});

test("composition and search letters are not intercepted; Enter inserts and Escape closes preview", async ({
  page,
}) => {
  await mountHistory(page);
  await page.getByText("Voice fixture", { exact: true }).click();
  const search = page.getByRole("searchbox");
  await search.dispatchEvent("keydown", { key: "Enter", isComposing: true });
  await search.dispatchEvent("keydown", { key: "j" });
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
  await search.press("Enter");
  await expect(page.getByText("Inserted.", { exact: true })).toBeVisible();
  await search.press("Escape");
  await expect(
    page.getByRole("complementary", { name: "Preview" }),
  ).toHaveCount(0);
});

for (const status of [
  "pending_target",
  "uncertain",
  "rejected",
  "failed",
  "dispatched",
]) {
  test(`output ${status} is not presented as success`, async ({ page }) => {
    await mountHistory(page);
    await page.evaluate((value) => {
      window.__HISTORY_TEST__.output = value;
    }, status);
    await page.getByText("Voice fixture", { exact: true }).dblclick();
    await expect(page.getByText("Inserted.", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("status")).toBeVisible();
    if (status === "uncertain" || status === "dispatched") {
      await expect(
        page.getByRole("button", { name: "Insert", exact: true }),
      ).toBeDisabled();
      await page.getByRole("searchbox").press("Enter");
      expect(
        await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
      ).toBe(1);
    }
  });
}

test("a lost insertion response requires explicit acknowledgement before another action", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "throw";
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  await page.getByRole("searchbox").press("Enter");
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page
    .getByRole("button", {
      name: "I checked the result; allow another insertion",
      exact: true,
    })
    .click();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "confirmed";
  });
  await page.getByRole("button", { name: "Insert", exact: true }).click();
  await expect(page.getByText("Inserted.", { exact: true })).toBeVisible();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(2);
});

test("stale search replies cannot replace newer results", async ({ page }) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.deferQueries = true;
  });
  await page.getByRole("searchbox").fill("old");
  await page.getByRole("searchbox").fill("new");
  await expect
    .poll(() => page.evaluate(() => window.__HISTORY_TEST__.pending.has("new")))
    .toBe(true);
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.pending.get("new")([{ ...h.items[0], text: "New result" }]);
  });
  await expect(page.getByText("New result", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.pending.get("old")([{ ...h.items[0], text: "Stale result" }]);
  });
  await expect(page.getByText("Stale result", { exact: true })).toHaveCount(0);
  await expect(page.getByText("New result", { exact: true })).toBeVisible();
});

test("navigation remount preserves an in-flight operation and its eventual uncertain receipt", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "defer";
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  await page.evaluate(() => window.__HISTORY_TEST__.remount());
  await page.getByText("Voice fixture", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  await page.getByRole("searchbox").press("Enter");
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page.evaluate(() =>
    window.__HISTORY_TEST__.pending.get("output")!({ status: "uncertain" }),
  );
  await expect(
    page.getByRole("button", {
      name: "I checked the result; allow another insertion",
      exact: true,
    }),
  ).toBeEnabled();
  await page.evaluate(() => window.__HISTORY_TEST__.remount());
  await page.getByText("Voice fixture", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
});

test("reload retains only output metadata and receipt queries reuse the previous operation ID", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "defer";
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  const original = await page.evaluate(() => ({
    id: window.__HISTORY_TEST__.actions[0].operationId,
    stored: localStorage.getItem("handy.unified-output-metadata.v1"),
  }));
  expect(original.stored).not.toContain("Voice fixture");
  expect(original.stored).not.toContain("synthetic");
  await mountHistory(page);
  await page.getByText("Voice fixture", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Check previous output receipt", exact: true })
    .click();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
  expect(
    await page.evaluate(
      () =>
        window.__HISTORY_TEST__.calls.find(
          (call) => call.command === "get_unified_output_receipt",
        )?.args?.operationId,
    ),
  ).toBe(original.id);
  await expect(page.getByText("Inserted.", { exact: true })).toBeVisible();
});

test("uncertain copy is also blocked across remount and cannot overwrite a newer clipboard", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "uncertain";
  });
  await page.getByText("Voice fixture", { exact: true }).click();
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  await page.evaluate(() => window.__HISTORY_TEST__.remount());
  await page.getByText("Voice fixture", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Copy", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
});

test("missing receipts stay uncertain until explicit acknowledgement creates a fresh operation", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "uncertain";
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  const firstId = await page.evaluate(
    () => window.__HISTORY_TEST__.actions[0].operationId,
  );
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "missing";
  });
  await page
    .getByRole("button", { name: "Check previous output receipt", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(1);
  await page
    .getByRole("button", {
      name: "I checked the result; allow another insertion",
      exact: true,
    })
    .click();
  await page.evaluate(() => {
    window.__HISTORY_TEST__.output = "confirmed";
  });
  await page.getByRole("button", { name: "Insert", exact: true }).click();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions[1].operationId),
  ).not.toBe(firstId);
});

test("failure to persist an output identity prevents the side effect", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    Storage.prototype.setItem = () => {
      throw new Error("synthetic storage failure");
    };
  });
  await page.getByText("Voice fixture", { exact: true }).dblclick();
  await expect(
    page.getByRole("button", { name: "Insert", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions.length),
  ).toBe(0);
});

test("live changes preserve edit revision and failed mutations do not report success", async ({
  page,
}) => {
  await mountHistory(page);
  await page.getByText("Voice fixture", { exact: true }).click();
  await page.getByRole("button", { name: "Edit", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Text", exact: true })
    .fill("My correction");
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.items[0] = {
      ...h.items[0],
      revision: 2,
      text: "Other window correction",
    };
    h.rejectMutation = true;
    h.emit();
  });
  await expect(
    page.getByText("Other window correction", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText(/Could not save/)).toBeVisible();
  await expect(
    page.getByRole("complementary", { name: "Preview" }).getByRole("alert"),
  ).toContainText("conflict");
  await expect(
    page.getByText("Failed to re-transcribe. Please try again.", {
      exact: true,
    }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__HISTORY_TEST__.actions[0].item.revision),
  ).toBe(1);
  await expect(
    page.getByText("Saved and synchronized.", { exact: true }),
  ).toHaveCount(0);
});

test("source/type/favorites filters and revision view use their real command contracts", async ({
  page,
}) => {
  await mountHistory(page);
  await page
    .getByRole("combobox", { name: "Source", exact: true })
    .selectOption("clipboard");
  await expect(page.getByText("Voice fixture", { exact: true })).toHaveCount(0);
  await page
    .getByRole("combobox", { name: "Content type" })
    .selectOption("files");
  await page.getByText("report.pdf", { exact: true }).click();
  await page.getByText("Revisions", { exact: true }).click();
  await expect(
    page.getByText("Original revision", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Add to favorites" }).click();
  await expect(
    page.getByText("Saved and synchronized.", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Favorites", exact: true }).click();
  await expect(page.getByText("report.pdf", { exact: true })).toBeVisible();
  await page.evaluate(() => {
    window.__HISTORY_TEST__.unmount();
  });
  await expect
    .poll(() => page.evaluate(() => window.__HISTORY_TEST__.listeners.size))
    .toBe(0);
});

test("pagination retains unique results and requests the next offset", async ({
  page,
}) => {
  await mountHistory(page);
  await page.evaluate(() => {
    const h = window.__HISTORY_TEST__;
    h.items = Array.from({ length: 45 }, (_, index) => ({
      ...h.items[0],
      item_id: `item-${index}`,
      text: `Entry ${index}`,
    }));
    h.emit();
  });
  await expect(
    page.getByRole("list", { name: "History", exact: true }).locator("li"),
  ).toHaveCount(40);
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(
    page.getByRole("list", { name: "History", exact: true }).locator("li"),
  ).toHaveCount(45);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
});

test("late listener registration is disposed and failed registration can be retried", async ({
  page,
}) => {
  await mountHistory(page);
  const result = await page.evaluate(async () => {
    const { createUnifiedHistoryStore } = await import(
      "/src/stores/unifiedHistoryStore.ts"
    );
    const h = window.__HISTORY_TEST__;
    const baseline = h.listeners.size;
    const store = createUnifiedHistoryStore();
    const dispose = store.getState().subscribe();
    dispose();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const afterDispose = h.listeners.size;
    h.rejectListener = true;
    const secondDispose = store.getState().subscribe();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const failed = store.getState().subscriptionError;
    h.rejectListener = false;
    await store.getState().refresh();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const recovered =
      !store.getState().subscriptionError && h.listeners.size === baseline + 1;
    secondDispose();
    return { baseline, afterDispose, failed, recovered };
  });
  expect(result.afterDispose).toBe(result.baseline);
  expect(result.failed).toBe(true);
  expect(result.recovered).toBe(true);
});

test("录音页在搜索、分页、收藏和实时刷新中始终只请求语音", async ({ page }) => {
  await mountHistory(page, true);
  await page.evaluate(async () => {
    const { default: i18n } = await import("/src/i18n/index.ts");
    await i18n.changeLanguage("zh");
  });
  await expect(
    page.getByRole("heading", { name: "录音", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("searchbox")).toHaveAttribute(
    "placeholder",
    "搜索录音和转写文字",
  );
  await expect(page.getByRole("combobox")).toHaveCount(0);
  await expect(page.getByText("report.pdf", { exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    const base = harness.items[0];
    harness.items = Array.from({ length: 45 }, (_, index) => [
      {
        ...base,
        item_id: `voice-${index}`,
        record_id: String(index),
        text: `Shared voice ${index}`,
        starred: index % 2 === 0,
      },
      {
        ...base,
        item_id: `clipboard-${index}`,
        record_id: String(index),
        source_kind: "clipboard",
        text: `Shared clipboard ${index}`,
        starred: true,
      },
    ]).flat();
    harness.emit();
  });
  const rows = page.getByRole("list").first().locator(":scope > li");
  await expect(rows).toHaveCount(40);
  await expect(page.getByText(/Shared clipboard/)).toHaveCount(0);
  await page.getByRole("button", { name: "加载更多", exact: true }).click();
  await expect(rows).toHaveCount(45);
  await page.getByRole("searchbox").fill("Shared");
  await expect(rows).toHaveCount(40);
  await page.getByRole("button", { name: "收藏", exact: true }).click();
  await expect(rows).toHaveCount(23);
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await expect(rows).toHaveCount(23);
  await page.evaluate(() => {
    const harness = window.__HISTORY_TEST__;
    harness.items.push(
      { ...harness.items[0], item_id: "voice-live", text: "Shared voice live" },
      {
        ...harness.items[1],
        item_id: "clipboard-live",
        text: "Shared clipboard live",
      },
    );
    harness.emit();
  });
  await expect(rows).toHaveCount(24);
  await expect(
    page.getByText("Shared voice live", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Shared clipboard/)).toHaveCount(0);
  await page.evaluate(async () => {
    const { useRecordingHistoryStore } = await import(
      "/src/stores/unifiedHistoryStore.ts"
    );
    useRecordingHistoryStore.getState().setFilters({ source: "clipboard" });
  });
  await expect(rows).toHaveCount(24);
  await page.getByRole("searchbox").fill("Shared clipboard");
  await expect(rows).toHaveCount(0);
  await expect(page.getByText("没有找到录音。", { exact: true })).toBeVisible();
  const evidence = await page.evaluate(async () => {
    const { useRecordingHistoryStore } = await import(
      "/src/stores/unifiedHistoryStore.ts"
    );
    return {
      source: useRecordingHistoryStore.getState().source,
      queries: window.__HISTORY_TEST__.calls
        .filter((call) => call.command === "get_unified_history")
        .map(
          (call) =>
            call.args?.query as {
              source_kind: string;
              offset: number;
              search: string | null;
              starred_only: boolean;
            },
        ),
      commands: window.__HISTORY_TEST__.calls.map((call) => call.command),
    };
  });
  expect(evidence.source).toBe("voice");
  expect(evidence.queries.every((query) => query.source_kind === "voice")).toBe(
    true,
  );
  expect(evidence.queries.some((query) => query.offset === 40)).toBe(true);
  expect(evidence.queries.some((query) => query.search === "Shared")).toBe(
    true,
  );
  expect(evidence.queries.some((query) => query.starred_only)).toBe(true);
  expect(
    evidence.commands.some((command) =>
      /delete|clear|update_unified/.test(command),
    ),
  ).toBe(false);
});

test("来源限定store无法被筛选覆盖，通用store仍支持混合历史", async ({
  page,
}) => {
  await mountHistory(page);
  const evidence = await page.evaluate(async () => {
    const { createUnifiedHistoryStore } = await import(
      "/src/stores/unifiedHistoryStore.ts"
    );
    const voice = createUnifiedHistoryStore("voice");
    const clipboard = createUnifiedHistoryStore("clipboard");
    const generic = createUnifiedHistoryStore();
    voice.getState().setFilters({ source: "all" });
    clipboard.getState().setFilters({ source: "voice" });
    await Promise.all([
      voice.getState().load(),
      clipboard.getState().load(),
      generic.getState().load(),
    ]);
    return [voice, clipboard, generic].map((store) => ({
      source: store.getState().source,
      kinds: store.getState().items.map((item) => item.source_kind),
    }));
  });
  expect(evidence).toEqual([
    { source: "voice", kinds: ["voice"] },
    { source: "clipboard", kinds: ["clipboard"] },
    { source: "all", kinds: ["voice", "clipboard"] },
  ]);
});
