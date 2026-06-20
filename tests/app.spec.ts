import { test, expect, type Page } from "@playwright/test";

type InvokeRecord = {
  cmd: string;
  args: Record<string, unknown>;
};

declare global {
  interface Window {
    __HANDY_TEST_INVOKES__: InvokeRecord[];
    __HANDY_TEST_EVENT_HANDLERS__: Record<string, number[]>;
    __HANDY_TEST_EMIT_TAURI_EVENT__: (event: string, payload: unknown) => void;
    __HANDY_TEST_RESOLVE_COPY__?: () => void;
    __TAURI_EVENT_PLUGIN_INTERNALS__: {
      unregisterListener: () => void;
    };
    __TAURI_INTERNALS__: {
      callbacks: Map<number, (data: unknown) => unknown>;
      convertFileSrc: (filePath: string) => string;
      invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
      metadata: {
        currentWebview: { label: string; windowLabel: string };
        currentWindow: { label: string };
      };
      runCallback: (id: number, data: unknown) => unknown;
      transformCallback: (callback: (data: unknown) => unknown) => number;
      unregisterCallback: (id: number) => boolean;
    };
    __TAURI_OS_PLUGIN_INTERNALS__: Record<string, string>;
  }
}

const clipboardOverlayItems = [
  {
    id: 101,
    title: "DeepSeek API",
    content_type: "text",
    content_preview: "第一条剪贴板内容",
    content_hash: "hash-101",
    full_text: "第一条剪贴板内容",
    image_path: null,
    source_app: null,
    is_favorite: true,
    is_pinned: false,
    created_at: "2026-06-14T09:33:03+08:00",
    size_bytes: 24,
  },
  {
    id: 102,
    title: null,
    content_type: "text",
    content_preview: "第二条剪贴板内容",
    content_hash: "hash-102",
    full_text: "第二条剪贴板内容",
    image_path: null,
    source_app: null,
    is_favorite: false,
    is_pinned: false,
    created_at: "2026-06-14T09:31:36+08:00",
    size_bytes: 24,
  },
];

const installClipboardOverlayMocks = async (page: Page) => {
  await page.addInitScript((items) => {
    const callbacks = new Map<number, (data: unknown) => unknown>();

    window.__HANDY_TEST_INVOKES__ = [];
    window.__HANDY_TEST_EVENT_HANDLERS__ = {};
    window.__HANDY_TEST_EMIT_TAURI_EVENT__ = (event, payload) => {
      for (const handlerId of window.__HANDY_TEST_EVENT_HANDLERS__[event] ??
        []) {
        window.__TAURI_INTERNALS__.runCallback(handlerId, {
          event,
          id: handlerId,
          payload,
        });
      }
    };
    window.__TAURI_OS_PLUGIN_INTERNALS__ = {
      arch: "aarch64",
      eol: "\n",
      exe_extension: "",
      family: "unix",
      os_type: "macos",
      platform: "macos",
      version: "15.0",
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: () => undefined,
    };
    window.__TAURI_INTERNALS__ = {
      callbacks,
      convertFileSrc: (filePath: string) => `asset://localhost/${filePath}`,
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        window.__HANDY_TEST_INVOKES__.push({ cmd, args });

        switch (cmd) {
          case "get_app_settings":
            return { app_language: "en", external_script_path: null };
          case "plugin:os|locale":
            return "en-US";
          case "plugin:event|listen":
            window.__HANDY_TEST_EVENT_HANDLERS__[args.event as string] ??= [];
            window.__HANDY_TEST_EVENT_HANDLERS__[args.event as string].push(
              args.handler as number,
            );
            return args.handler;
          case "plugin:event|unlisten":
          case "plugin:window|hide":
          case "plugin:window|start_dragging":
          case "plugin:window|set_always_on_top":
            return null;
          case "get_clipboard_stats":
            return {
              favorites_count: items.filter((item) => item.is_favorite).length,
              pinned_count: 0,
              total_items: items.length,
              total_size_bytes: 48,
            };
          case "get_clipboard_settings":
            return {
              confirm_mode: "copy",
              hotkey: "CommandOrControl+Shift+V",
              max_records: 500,
            };
          case "get_clipboard_items": {
            const contentType = args.contentType as string | undefined;
            const favoriteOnly = Boolean(args.favoriteOnly);
            const page = Number(args.page ?? 0);
            const pageSize = Number(args.pageSize ?? items.length);
            const filteredItems = (
              items as typeof clipboardOverlayItems
            ).filter((item) => {
              const matchesFavorite = !favoriteOnly || item.is_favorite;
              const matchesType =
                !contentType ||
                contentType === "all" ||
                (contentType === "text" &&
                  ["text", "richtext"].includes(item.content_type)) ||
                item.content_type === contentType;
              return matchesFavorite && matchesType;
            });
            const start = page * pageSize;
            return {
              has_more: start + pageSize < filteredItems.length,
              items: filteredItems.slice(start, start + pageSize),
            };
          }
          case "copy_clipboard_to_system":
            return new Promise((resolve) => {
              window.__HANDY_TEST_RESOLVE_COPY__ = () => resolve(null);
            });
          case "toggle_clipboard_favorite":
          case "toggle_clipboard_pin":
          case "update_clipboard_title":
          case "delete_clipboard_item":
            return null;
          default:
            return null;
        }
      },
      metadata: {
        currentWebview: { label: "clipboard", windowLabel: "clipboard" },
        currentWindow: { label: "clipboard" },
      },
      runCallback: (id: number, data: unknown) => callbacks.get(id)?.(data),
      transformCallback: (callback: (data: unknown) => unknown) => {
        const id = window.crypto.getRandomValues(new Uint32Array(1))[0];
        callbacks.set(id, callback);
        return id;
      },
      unregisterCallback: (id: number) => callbacks.delete(id),
    };
  }, clipboardOverlayItems);
};

test.describe("Handy App", () => {
  test("dev server responds", async ({ page }) => {
    // Just verify the dev server is running and responds
    const response = await page.goto("/");
    expect(response?.status()).toBe(200);
  });

  test("page has html structure", async ({ page }) => {
    await page.goto("/");

    // Verify basic HTML structure exists
    const html = await page.content();
    expect(html).toContain("<html");
    expect(html).toContain("<body");
  });

  test("clipboard overlay copies an item on single click", async ({ page }) => {
    await installClipboardOverlayMocks(page);
    await page.goto("/src/overlay/clipboard/index.html");

    const items = page.getByTestId("clipboard-overlay-item");
    await expect(items).toHaveCount(2);

    await items.nth(1).click();

    const copyInvokes = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "copy_clipboard_to_system",
      ),
    );
    expect(copyInvokes).toEqual([
      {
        cmd: "copy_clipboard_to_system",
        args: {
          id: 102,
        },
      },
    ]);
    const hideInvokes = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "plugin:window|hide",
      ),
    );
    expect(hideInvokes).toHaveLength(1);
    await expect(items.nth(1)).toHaveClass(/copied/);

    await page.evaluate(() => {
      window.__HANDY_TEST_EMIT_TAURI_EVENT__("clipboard-update-payload", {
        action: "updated",
        item: {
          id: 102,
          title: null,
          content_type: "text",
          content_preview: "第二条剪贴板内容",
          content_hash: "hash-102",
          full_text: "第二条剪贴板内容",
          image_path: null,
          source_app: null,
          is_favorite: false,
          is_pinned: false,
          created_at: "2026-06-14T09:40:00+08:00",
          size_bytes: 24,
        },
      });
    });

    await expect(items.first()).toHaveAttribute(
      "data-clipboard-item-id",
      "102",
    );
    await expect(items.first()).toHaveClass(/selected/);
    await expect(items.first()).toHaveClass(/copied/);
    await expect(items.nth(1)).not.toHaveClass(/selected/);

    await items.nth(1).locator(".clipboard-overlay-star-button").click();
    const copyInvokesAfterAction = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "copy_clipboard_to_system",
      ),
    );
    expect(copyInvokesAfterAction).toHaveLength(1);
    const hideInvokesAfterAction = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "plugin:window|hide",
      ),
    );
    expect(hideInvokesAfterAction).toHaveLength(1);

    await page.evaluate(() => window.__HANDY_TEST_RESOLVE_COPY__?.());
  });

  test("clipboard overlay loads filtered pages instead of all records", async ({
    page,
  }) => {
    await installClipboardOverlayMocks(page);
    await page.goto("/src/overlay/clipboard/index.html");

    await expect(page.getByTestId("clipboard-overlay-item")).toHaveCount(2);

    await page.locator(".clipboard-overlay-favorite-filter").click();
    await expect(page.getByTestId("clipboard-overlay-item")).toHaveCount(1);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__HANDY_TEST_INVOKES__
              .filter((record) => record.cmd === "get_clipboard_items")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        contentType: "all",
        favoriteOnly: true,
        page: 0,
        pageSize: 20,
      });

    await page.locator(".clipboard-overlay-favorite-filter").click();
    await page.getByTitle("Images").click();
    await expect(page.getByTestId("clipboard-overlay-item")).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__HANDY_TEST_INVOKES__
              .filter((record) => record.cmd === "get_clipboard_items")
              .at(-1)?.args,
        ),
      )
      .toMatchObject({
        contentType: "image",
        favoriteOnly: false,
        page: 0,
        pageSize: 20,
      });
  });

  test("clipboard overlay edits item titles without copying", async ({
    page,
  }) => {
    await installClipboardOverlayMocks(page);
    await page.goto("/src/overlay/clipboard/index.html");

    const firstItem = page.getByTestId("clipboard-overlay-item").first();
    const secondItem = page.getByTestId("clipboard-overlay-item").nth(1);
    await expect(firstItem.locator(".clipboard-overlay-item-title")).toHaveText(
      "DeepSeek API",
    );
    await expect(
      firstItem.locator(".clipboard-overlay-item-action-row > button"),
    ).toHaveCount(3);
    await expect(
      secondItem.locator(".clipboard-overlay-item-title"),
    ).toHaveCount(0);

    await secondItem.locator(".clipboard-overlay-star-button").click();
    await expect(
      secondItem.locator(".clipboard-overlay-item-title"),
    ).toHaveText("第二条剪贴板");

    await secondItem.locator(".clipboard-overlay-title-button").click();
    const emptyTitleInput = secondItem.locator(
      ".clipboard-overlay-title-input",
    );
    await expect(emptyTitleInput).toBeFocused();
    await emptyTitleInput.press("Escape");
    await expect(
      secondItem.locator(".clipboard-overlay-title-input"),
    ).toHaveCount(0);
    await expect(
      secondItem.locator(".clipboard-overlay-item-title"),
    ).toHaveText("第二条剪贴板");

    await firstItem.locator(".clipboard-overlay-title-button").click();
    const titleInput = firstItem.locator(".clipboard-overlay-title-input");
    await expect(titleInput).toBeFocused();
    await titleInput.fill("DeepSeek 生产 API Key");
    await firstItem.locator(".clipboard-overlay-title-button").click();

    await expect(firstItem.locator(".clipboard-overlay-item-title")).toHaveText(
      "DeepSeek 生产 API Key",
    );
    await expect(titleInput).toHaveCount(0);
    await expect(
      firstItem.locator(".clipboard-overlay-title-button"),
    ).not.toHaveClass(/editing/);

    const titleInvokes = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "update_clipboard_title",
      ),
    );
    expect(titleInvokes).toEqual([
      {
        cmd: "update_clipboard_title",
        args: {
          id: 101,
          title: "DeepSeek 生产 API Key",
        },
      },
    ]);

    await firstItem.locator(".clipboard-overlay-title-button").click();
    await expect(titleInput).toBeFocused();
    await expect(
      firstItem.locator(".clipboard-overlay-title-button"),
    ).toHaveClass(/editing/);
    await titleInput.fill("");
    await titleInput.press("Enter");
    await expect(firstItem.locator(".clipboard-overlay-item-title")).toHaveText(
      "第一条剪贴板",
    );

    const clearTitleInvokes = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "update_clipboard_title",
      ),
    );
    expect(clearTitleInvokes).toEqual([
      {
        cmd: "update_clipboard_title",
        args: {
          id: 101,
          title: "DeepSeek 生产 API Key",
        },
      },
      {
        cmd: "update_clipboard_title",
        args: {
          id: 101,
          title: null,
        },
      },
    ]);

    const copyInvokes = await page.evaluate(() =>
      window.__HANDY_TEST_INVOKES__.filter(
        (record) => record.cmd === "copy_clipboard_to_system",
      ),
    );
    expect(copyInvokes).toHaveLength(0);
  });
});
