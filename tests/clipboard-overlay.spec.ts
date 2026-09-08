import { expect, test, type Page } from "@playwright/test";

type MockClipboardItem = {
  id: number;
  title?: string | null;
  content_type: "text" | "image" | "richtext" | "file";
  content_preview: string;
  content_hash: string;
  full_text?: string;
  image_path?: string;
  source_app?: string;
  is_favorite: boolean;
  is_pinned: boolean;
  created_at: string;
  size_bytes: number;
};

const mockItems: MockClipboardItem[] = [
  {
    id: 101,
    content_type: "text",
    content_preview: "Ropy parity text item",
    content_hash: "hash-text-101",
    full_text: "Ropy parity text item",
    source_app: "Notes",
    is_favorite: false,
    is_pinned: false,
    created_at: "2026-09-02T03:20:58Z",
    size_bytes: 21,
  },
  {
    id: 202,
    content_type: "file",
    content_preview: "Files",
    content_hash: "hash-file-202",
    full_text: JSON.stringify([
      "/Users/lzl/Library/Containers/whbalzac.Dongtaizhuomian/Data/Documents/Videos",
      "/Users/lzl/Library/Containers/whbalzac.Dongtaizhuomian/Data/Documents/Images",
    ]),
    source_app: "Finder",
    is_favorite: false,
    is_pinned: false,
    created_at: "2026-09-02T03:19:33Z",
    size_bytes: 128,
  },
];

async function installTauriMocks(
  page: Page,
  status = "confirmed",
  mode = "copy",
) {
  await page.addInitScript(
    ({ items, status, mode }) => {
      type InvokeRecord = { cmd: string; args?: Record<string, unknown> };
      const records: InvokeRecord[] = [];
      const callbacks = new Map<number, (payload: unknown) => void>();
      let callbackId = 1;
      const currentItems = (items as MockClipboardItem[]).map((item) => ({
        ...item,
      }));

      Object.assign(window, {
        __HANDY_PLAYWRIGHT_INVOKES__: records,
      });

      Object.assign(window, {
        __TAURI_EVENT_PLUGIN_INTERNALS__: {
          unregisterListener: (_event: string, id: number) => {
            callbacks.delete(id);
          },
        },
        __TAURI_INTERNALS__: {
          callbacks,
          metadata: {
            currentWindow: { label: "clipboard_overlay" },
            currentWebview: { label: "clipboard_overlay" },
          },
          transformCallback: (
            callback?: (payload: unknown) => void,
            once = false,
          ) => {
            const id = callbackId++;
            callbacks.set(id, (payload: unknown) => {
              if (once) {
                callbacks.delete(id);
              }
              callback?.(payload);
            });
            return id;
          },
          unregisterCallback: (id: number) => {
            callbacks.delete(id);
          },
          runCallback: (id: number, payload: unknown) => {
            callbacks.get(id)?.(payload);
          },
          convertFileSrc: (filePath: string, protocol = "asset") =>
            `${protocol}://localhost/${encodeURIComponent(filePath)}`,
          invoke: async (cmd: string, args?: Record<string, unknown>) => {
            records.push({ cmd, args });

            if (cmd === "plugin:event|listen") {
              return Number(args?.handler ?? 0);
            }
            if (cmd === "plugin:event|unlisten") {
              return null;
            }
            if (cmd === "get_clipboard_stats") {
              return {
                total_items: currentItems.length,
                favorites_count: currentItems.filter((item) => item.is_favorite)
                  .length,
                pinned_count: currentItems.filter((item) => item.is_pinned)
                  .length,
                total_size_bytes: currentItems.reduce(
                  (total, item) => total + item.size_bytes,
                  0,
                ),
              };
            }
            if (cmd === "get_clipboard_settings") {
              return {
                max_records: 500,
                hotkey: "Alt+Shift+Space",
                confirm_mode: mode,
              };
            }
            if (cmd === "get_unified_history") {
              const query = args?.query as {
                search?: string;
                content_type?: string;
                starred_only?: boolean;
              };
              return currentItems
                .map((item) => ({
                  item_id: `clipboard:store:${item.id}`,
                  store_id: "store",
                  record_id: String(item.id),
                  revision: 3,
                  source_kind: item.id === 101 ? "voice" : "clipboard",
                  content_type:
                    item.content_type === "file" ? "files" : item.content_type,
                  text: item.full_text ?? null,
                  title: item.title ?? null,
                  starred: item.is_favorite,
                  pinned: item.is_pinned,
                  created_at_ms: Date.parse(item.created_at),
                  asset_ref: item.id === 101 ? "recording:opaque" : null,
                  source_app: item.source_app,
                }))
                .filter(
                  (item) =>
                    (!query.search ||
                      item.text
                        ?.toLowerCase()
                        .includes(query.search.toLowerCase())) &&
                    (!query.content_type ||
                      item.content_type === query.content_type) &&
                    (!query.starred_only || item.starred),
                );
            }
            if (cmd === "update_unified_history_item") {
              const item = currentItems.find(
                (item) => `clipboard:store:${item.id}` === args?.itemId,
              );
              const patch = args?.patch as {
                starred?: boolean;
                pinned?: boolean;
                title?: string;
              };
              if (item) {
                if (patch.starred !== null)
                  item.is_favorite = Boolean(patch.starred);
                if (patch.pinned !== null)
                  item.is_pinned = Boolean(patch.pinned);
                if (patch.title !== null) item.title = patch.title;
              }
              return 4;
            }
            if (cmd === "delete_unified_history_item") {
              const index = currentItems.findIndex(
                (item) => `clipboard:store:${item.id}` === args?.itemId,
              );
              if (index >= 0) currentItems.splice(index, 1);
              return true;
            }
            if (
              [
                "copy_unified_history_item",
                "insert_unified_history_item",
                "copy_unified_history_item_as_text",
              ].includes(cmd)
            ) {
              const persisted = JSON.parse(
                localStorage.getItem("handy.unified-output-metadata.v1") ||
                  "[]",
              );
              if (
                !persisted.some(
                  (entry: { operationId: string }) =>
                    entry.operationId === args?.operationId,
                )
              )
                throw new Error("Missing persisted UUID");
              return { operation_id: args?.operationId, status };
            }
            if (cmd === "get_unified_output_receipt")
              return { operation_id: args?.operationId, status };
            if (cmd === "get_clipboard_items") {
              return { items: currentItems, has_more: false };
            }
            if (cmd === "search_clipboard") {
              const query = String(args?.query ?? "").toLowerCase();
              return currentItems.filter((item) =>
                [
                  item.title,
                  item.content_preview,
                  item.full_text,
                  item.source_app,
                ]
                  .filter(Boolean)
                  .join("\n")
                  .toLowerCase()
                  .includes(query),
              );
            }
            if (cmd === "update_unified_history_item") {
              const item = currentItems.find((entry) => entry.id === args?.id);
              if (item) {
                item.is_favorite = !item.is_favorite;
              }
              return null;
            }
            if (cmd === "update_unified_history_item") {
              const item = currentItems.find((entry) => entry.id === args?.id);
              if (item) {
                item.is_pinned = !item.is_pinned;
              }
              return null;
            }
            if (cmd === "delete_unified_history_item") {
              const index = currentItems.findIndex(
                (entry) => entry.id === args?.id,
              );
              if (index >= 0) {
                currentItems.splice(index, 1);
              }
              return null;
            }
            if (
              cmd === "copy_unified_history_item" ||
              cmd === "copy_unified_history_item_as_text" ||
              cmd === "hide_clipboard_overlay" ||
              cmd === "set_clipboard_overlay_pinned"
            ) {
              return null;
            }

            throw new Error(`Unhandled mock invoke: ${cmd}`);
          },
        },
      });
    },
    { items: mockItems, status, mode },
  );
}

async function openClipboardOverlay(
  page: Page,
  status = "confirmed",
  mode = "copy",
) {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });
  await installTauriMocks(page, status, mode);
  await page.setViewportSize({ width: 400, height: 550 });
  await page.goto("/src/overlay/clipboard/index.html");
  await expect(page.locator(".clipboard-overlay")).toBeVisible();
  return { consoleErrors };
}

async function invokeCommands(page: Page) {
  return page.evaluate(() =>
    (
      window as typeof window & {
        __HANDY_PLAYWRIGHT_INVOKES__: Array<{
          cmd: string;
          args?: Record<string, unknown>;
        }>;
      }
    ).__HANDY_PLAYWRIGHT_INVOKES__.map((record) => record.cmd),
  );
}

async function invokeRecords(page: Page) {
  return page.evaluate(
    () =>
      (
        window as typeof window & {
          __HANDY_PLAYWRIGHT_INVOKES__: Array<{
            cmd: string;
            args?: Record<string, unknown>;
          }>;
        }
      ).__HANDY_PLAYWRIGHT_INVOKES__,
  );
}

async function clearInvokes(page: Page) {
  await page.evaluate(() => {
    (
      window as typeof window & {
        __HANDY_PLAYWRIGHT_INVOKES__: Array<{
          cmd: string;
          args?: Record<string, unknown>;
        }>;
      }
    ).__HANDY_PLAYWRIGHT_INVOKES__.length = 0;
  });
}

async function blurSearch(page: Page) {
  await page.locator(".clipboard-overlay-search").evaluate((node) => {
    (node as HTMLInputElement).blur();
  });
}

test.describe("clipboard overlay", () => {
  test("renders without the Tauri event ACL error", async ({ page }) => {
    const { consoleErrors } = await openClipboardOverlay(page);

    await expect(page.locator("body")).not.toContainText(
      "Command plugin:event|listen not allowed by ACL",
    );
    expect(consoleErrors).toEqual([]);
    await expect(page.getByText("Ropy parity text item")).toBeVisible();
  });

  test("keeps a single self-drawn rounded overlay without a native titlebar gap", async ({
    page,
  }) => {
    await openClipboardOverlay(page);

    const metrics = await page
      .locator(".clipboard-overlay")
      .evaluate((node) => {
        const style = window.getComputedStyle(node);
        const box = node.getBoundingClientRect();
        return {
          top: box.top,
          left: box.left,
          width: box.width,
          height: box.height,
          borderRadius: style.borderRadius,
          overflow: style.overflow,
        };
      });
    const stage = await page.locator(".clipboard-overlay-stage").boundingBox();

    expect(metrics.top).toBe(0);
    expect(metrics.left).toBe(0);
    expect(metrics.width).toBe(400);
    expect(metrics.height).toBe(550);
    expect(Number.parseFloat(metrics.borderRadius)).toBeGreaterThanOrEqual(22);
    expect(metrics.overflow).toBe("hidden");
    expect(stage?.y).toBe(0);
  });

  test("uses Ropy-sized controls and rounded pills", async ({ page }) => {
    await openClipboardOverlay(page);

    const pinButton = page.locator(".clipboard-overlay-icon-button").first();
    const searchPill = page.locator(".clipboard-overlay-search-pill");
    const toolButton = page.locator(".clipboard-overlay-tool-button").first();

    await expect(pinButton).toHaveCSS("width", "40px");
    await expect(pinButton).toHaveCSS("height", "40px");
    await expect(searchPill).toHaveCSS("border-radius", "21px");
    await expect(toolButton).toHaveCSS("width", "36px");
    await expect(toolButton).toHaveCSS("height", "36px");

    const controlsFit = await page
      .locator(".clipboard-overlay-controls")
      .last()
      .evaluate((node) => node.scrollWidth === node.clientWidth);
    expect(controlsFit).toBe(true);
  });

  test("renders file items with first filename and full path", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await page.getByTitle(/files/i).click();

    const item = page.locator('[data-clipboard-item-id="clipboard:store:202"]');
    await expect(item.locator(".clipboard-overlay-item-title")).toContainText(
      "Videos",
    );
    await expect(item.locator(".clipboard-overlay-file-count")).toHaveText(
      "2 items",
    );
    await expect(item.locator(".clipboard-overlay-item-text")).toContainText(
      "/Users/lzl/Library/Containers/whbalzac.Dongtaizhuomian/Data/Documents/Videos",
    );
    await expect(item.locator(".clipboard-overlay-item-text")).toContainText(
      "/Users/lzl/Library/Containers/whbalzac.Dongtaizhuomian/Data/Documents/Images",
    );
  });

  test("focuses search when slash is pressed outside the input", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.press("/");

    await expect(page.locator(".clipboard-overlay-search")).toBeFocused();
  });

  test("confirms the first result when 1 is pressed", async ({ page }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.press("1");

    const commands = await invokeCommands(page);
    expect(commands).toContain("copy_unified_history_item");
    expect(commands).toContain("hide_clipboard_overlay");
  });

  test("pins the selected result when p is pressed", async ({ page }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.press("p");

    const commands = await invokeCommands(page);
    expect(commands).toContain("update_unified_history_item");
    await expect(
      page
        .locator(
          '[data-clipboard-item-id="clipboard:store:101"] .clipboard-overlay-pin-button',
        )
        .first(),
    ).toHaveClass(/pinned/);
  });

  test("favorites the selected result when f is pressed", async ({ page }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.press("f");

    const commands = await invokeCommands(page);
    expect(commands).toContain("update_unified_history_item");
    await expect(
      page.locator(
        '[data-clipboard-item-id="clipboard:store:101"] .clipboard-overlay-star-button',
      ),
    ).toHaveClass(/favorited/);
  });

  test("copies the selected text as plain text when Shift+Enter is pressed", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await clearInvokes(page);
    await blurSearch(page);

    await page.keyboard.press("Shift+Enter");

    const plainTextCopy = (await invokeRecords(page)).find(
      (record) => record.cmd === "copy_unified_history_item_as_text",
    );
    expect(plainTextCopy?.args).toMatchObject({
      itemId: "clipboard:store:101",
      expectedRevision: 3,
      operationId: expect.stringMatching(/^[0-9a-f-]{36}$/),
    });
  });

  test("deletes the selected result when d is pressed", async ({ page }) => {
    await openClipboardOverlay(page);
    await clearInvokes(page);
    await blurSearch(page);

    await page.keyboard.press("d");

    await expect(page.getByRole("alertdialog")).toBeVisible();
    expect(await invokeCommands(page)).not.toContain(
      "delete_unified_history_item",
    );
    await page.getByRole("alertdialog").getByRole("button").last().click();

    const commands = await invokeCommands(page);
    expect(commands).toContain("delete_unified_history_item");
  });

  test("deletes the selected result when Delete is pressed", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await clearInvokes(page);
    await blurSearch(page);

    await page.keyboard.press("Delete");

    await expect(page.getByRole("alertdialog")).toBeVisible();
    expect(await invokeCommands(page)).not.toContain(
      "delete_unified_history_item",
    );
    await page.getByRole("alertdialog").getByRole("button").last().click();

    const commands = await invokeCommands(page);
    expect(commands).toContain("delete_unified_history_item");
  });

  test("ignores item shortcuts while the search field is focused", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await clearInvokes(page);
    await page.locator(".clipboard-overlay-search").focus();

    await page.keyboard.press("p");
    await page.keyboard.press("f");
    await page.keyboard.press("1");
    await page.keyboard.press("Space");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Shift+Enter");

    const commands = await invokeCommands(page);
    expect(commands).not.toContain("update_unified_history_item");
    expect(commands).not.toContain("update_unified_history_item");
    expect(commands).not.toContain("copy_unified_history_item");
    expect(commands).not.toContain("copy_unified_history_item_as_text");
    expect(commands).not.toContain("hide_clipboard_overlay");
    await expect(page.locator(".clipboard-overlay-preview")).toBeHidden();
  });

  test("shows a preview for the selected result while space is held", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.down("Space");

    await expect(page.locator(".clipboard-overlay-preview")).toBeVisible();
    await expect(page.locator(".clipboard-overlay-preview")).toContainText(
      "Ropy parity text item",
    );
  });

  test("hides the selected result preview when Space is released", async ({
    page,
  }) => {
    await openClipboardOverlay(page);
    await blurSearch(page);

    await page.keyboard.down("Space");
    await expect(page.locator(".clipboard-overlay-preview")).toBeVisible();
    await page.keyboard.up("Space");

    await expect(page.locator(".clipboard-overlay-preview")).toBeHidden();
  });
});

test("shared voice identity preserves recording attachment and explicit insert", async ({
  page,
}) => {
  await openClipboardOverlay(page, "confirmed", "paste");
  await page.locator('[data-clipboard-item-id="clipboard:store:101"]').click();
  const records = await invokeRecords(page);
  expect(
    records.find((record) => record.cmd === "insert_unified_history_item")
      ?.args,
  ).toMatchObject({
    itemId: "clipboard:store:101",
    expectedRevision: 3,
    operationId: expect.stringMatching(/^[0-9a-f-]{36}$/),
  });
  expect(
    records.some((record) => record.cmd === "get_unified_history_asset"),
  ).toBe(false);
  expect(records.some((record) => record.cmd === "get_clipboard_items")).toBe(
    false,
  );
});

for (const status of ["dispatched", "uncertain", "future_status"]) {
  test(`blocks replay and keeps window open for ${status}`, async ({
    page,
  }) => {
    await openClipboardOverlay(page, status);
    const item = page.locator('[data-clipboard-item-id="clipboard:store:101"]');
    await item.click();
    await expect(
      page.getByRole("button", { name: "Check previous output receipt" }),
    ).toBeVisible();
    await item.click();
    expect(
      (await invokeCommands(page)).filter(
        (cmd) => cmd === "copy_unified_history_item",
      ),
    ).toHaveLength(1);
    expect(await invokeCommands(page)).not.toContain("hide_clipboard_overlay");
    await page
      .getByRole("button", { name: "Check previous output receipt" })
      .click();
    expect(await invokeCommands(page)).toContain("get_unified_output_receipt");
    expect(
      (await invokeCommands(page)).filter(
        (cmd) => cmd === "copy_unified_history_item",
      ),
    ).toHaveLength(1);
    await page
      .getByRole("button", {
        name: "I checked the clipboard; allow another copy",
      })
      .click();
    await page.getByRole("button", { name: "Copy", exact: true }).click();
    expect(
      (await invokeCommands(page)).filter(
        (cmd) => cmd === "copy_unified_history_item",
      ),
    ).toHaveLength(2);
  });
}

test("cancelling record deletion never mutates or triggers underlying shortcuts", async ({
  page,
}) => {
  await openClipboardOverlay(page);
  await blurSearch(page);
  await clearInvokes(page);
  await page.keyboard.press("Delete");
  await expect(page.getByRole("alertdialog")).toBeVisible();
  for (const key of ["d", "f", "p", "1"]) await page.keyboard.press(key);
  // 即使外部焦点移动，底层列表也不能执行快捷键。
  await page.locator(".clipboard-overlay").focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Delete");
  const commands = await invokeCommands(page);
  expect(commands).not.toContain("delete_unified_history_item");
  expect(commands).not.toContain("copy_unified_history_item");
  expect(commands).not.toContain("update_unified_history_item");
  await page
    .getByRole("alertdialog")
    .getByRole("button", { name: "Cancel" })
    .click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(
    page.locator('[data-clipboard-item-id="clipboard:store:101"]'),
  ).toBeVisible();
  expect(await invokeCommands(page)).not.toContain(
    "delete_unified_history_item",
  );
});

test("pending target leaves the overlay open without success feedback", async ({
  page,
}) => {
  await openClipboardOverlay(page, "pending_target", "paste");
  await page.locator('[data-clipboard-item-id="clipboard:store:101"]').click();
  await expect(page.getByRole("status")).toContainText(
    "Waiting for a valid input target",
  );
  expect(await invokeCommands(page)).not.toContain("hide_clipboard_overlay");
  await expect(page.locator(".clipboard-overlay-item.copied")).toHaveCount(0);
});

test("image preview resolves the managed asset instead of using the opaque reference", async ({
  page,
}) => {
  await installTauriMocks(page);
  await page.addInitScript(() => {
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (
            cmd: string,
            args?: Record<string, unknown>,
          ) => Promise<unknown>;
          convertFileSrc: (path: string) => string;
        };
      }
    ).__TAURI_INTERNALS__;
    const originalInvoke = internals.invoke;
    internals.invoke = async (cmd, args) => {
      if (cmd === "get_unified_history") {
        return [
          {
            item_id: "clipboard:asset:image",
            store_id: "asset",
            record_id: "image",
            revision: 9,
            source_kind: "clipboard",
            content_type: "image",
            text: null,
            title: null,
            starred: false,
            pinned: false,
            created_at_ms: 1,
            asset_ref: "opaque:must-not-be-a-path",
            source_app: null,
          },
        ];
      }
      if (cmd === "get_unified_history_asset") {
        if (
          args?.itemId !== "clipboard:asset:image" ||
          args?.expectedRevision !== 9
        )
          throw new Error("Wrong asset identity");
        return "/managed/safe-image.png";
      }
      return originalInvoke(cmd, args);
    };
    internals.convertFileSrc = (path) => {
      if (path !== "/managed/safe-image.png")
        throw new Error("Opaque reference used as path");
      return "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/%3E";
    };
  });
  await page.goto("/src/overlay/clipboard/index.html");
  await expect(
    page.locator(".clipboard-overlay-image-preview img"),
  ).toHaveAttribute("src", /^data:image/);
});
