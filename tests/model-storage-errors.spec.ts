import { expect, test } from "@playwright/test";

for (const viaEvent of [false, true]) {
  test(`model storage failure keeps a localized reason and clears progress: ${viaEvent}`, async ({
    page,
  }) => {
    await page.route("**/__model-storage-errors", (route) =>
      route.fulfill({
        contentType: "text/html",
        body: "<html><body></body></html>",
      }),
    );
    await page.goto("/__model-storage-errors");
    const result = await page.evaluate(async (viaEvent) => {
      const callbacks = new Map<number, (value: unknown) => void>();
      const listeners = new Map<string, number>();
      Object.assign(window, {
        __TAURI_INTERNALS__: {
          transformCallback: (callback: (value: unknown) => void) => {
            const id = callbacks.size + 1;
            callbacks.set(id, callback);
            return id;
          },
          unregisterCallback: () => undefined,
          invoke: async (
            command: string,
            args?: { event?: string; handler?: number },
          ) => {
            if (command === "get_available_models") return [];
            if (command === "get_current_model") return "";
            if (command === "download_model")
              throw "model_storage_download_limit";
            if (
              command === "plugin:event|listen" &&
              args?.event &&
              args.handler
            ) {
              listeners.set(args.event, args.handler);
              return args.handler;
            }
            throw new Error("unexpected fixture command");
          },
        },
      });
      const moduleText = await (
        await fetch("/src/stores/modelStore.ts")
      ).text();
      const dependency = moduleText.match(
        /from ["']([^"']*\/i18next\.js[^"']*)["']/,
      );
      if (!dependency) throw new Error("i18n dependency missing");
      const { default: i18n } = await import(dependency[1]);
      const { default: translations } = await import(
        "/src/i18n/locales/zh/translation.json"
      );
      await i18n.init({
        lng: "zh",
        resources: { zh: { translation: translations } },
      });
      const { useModelStore } = await import("/src/stores/modelStore.ts");
      const store = useModelStore.getState();
      let saved = false;
      if (viaEvent) {
        await store.initialize();
        useModelStore.setState({
          downloadingModels: { fixture: true },
          extractingModels: { fixture: true },
          downloadProgress: { fixture: { downloaded: 1 } },
        });
        for (const event of [
          "model-extraction-failed",
          "model-download-failed",
        ]) {
          const callback = callbacks.get(listeners.get(event) ?? 0);
          if (!callback) throw new Error("listener missing");
          callback({
            event,
            payload: {
              model_id: "fixture",
              error: "model_storage_insufficient_space",
            },
          });
        }
      } else {
        saved = await store.downloadModel("fixture");
      }
      const state = useModelStore.getState();
      return {
        saved,
        error: state.error,
        downloading: state.downloadingModels,
        progress: state.downloadProgress,
        extracting: state.extractingModels,
      };
    }, viaEvent);
    expect(result.saved).toBe(false);
    expect(result.error).toContain(viaEvent ? "磁盘空间不足" : "单次下载上限");
    expect(result.error).not.toContain("model_storage_");
    expect(result.downloading).toEqual({});
    expect(result.progress).toEqual({});
    expect(result.extracting).toEqual({});
  });
}
