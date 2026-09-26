import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { PersonalizationPanel } from "./PersonalizationPanel";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

interface Source {
  id: string;
  name: string;
  kind: string;
  path: string | null;
  enabled: boolean;
  external_access: boolean;
  capture_enabled?: boolean;
  capture_epoch?: number;
  status: string;
  description?: string;
  item_count: number | null;
}
interface KnowledgeStatus {
  managed_path: string;
  sources: Source[];
}
interface Item {
  id: string;
  revision: string | number;
  source_id: string;
  title: string;
  text: string;
  locator: string;
  kind: string;
  truncated?: boolean;
  next_offset?: number | null;
}
interface SearchResult {
  items: Item[];
  warnings: string[];
  has_more?: boolean;
}
interface Connection {
  prompt: string;
  skill_path: string;
  cli_path: string;
}
const request = <T,>(action: string, payload: Record<string, unknown> = {}) =>
  invoke<T>("knowledge_request", { action, payload });
const button =
  "rounded-lg border border-mid-gray/30 px-3 py-2 text-sm hover:bg-mid-gray/10 disabled:opacity-40 disabled:cursor-not-allowed";
const field =
  "w-full rounded-lg border border-mid-gray/30 bg-transparent p-2 text-sm";

export function KnowledgePage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<KnowledgeStatus>();
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [operationWarnings, setOperationWarnings] = useState<string[]>([]);
  const [searchError, setSearchError] = useState("");
  const [notice, setNotice] = useState("");
  const [query, setQuery] = useState("");
  const [sourceId, setSourceId] = useState("");
  const [results, setResults] = useState<SearchResult>({
    items: [],
    warnings: [],
  });
  const [selected, setSelected] = useState<Item>();
  const [connection, setConnection] = useState<Connection>();
  const [note, setNote] = useState(false);
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [typedRemoval, setTypedRemoval] = useState<Item | "all">();
  const [remove, setRemove] = useState<Source>();
  const generation = useRef(0);
  const previewGeneration = useRef(0);
  const reload = useCallback(
    async () => setStatus(await request<KnowledgeStatus>("status")),
    [],
  );
  const run = async (operation: () => Promise<void>) => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await operation();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    let active = true;
    request<KnowledgeStatus>("status")
      .then((value) => {
        if (active) setStatus(value);
      })
      .catch((cause) => {
        if (active) setError(String(cause));
      });
    return () => {
      active = false;
    };
  }, []);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    const current = ++generation.current;
    ++previewGeneration.current;
    setSelected(undefined);
    setLoading(true);
    const timer = setTimeout(() => {
      request<SearchResult>("search", {
        query,
        source_id: sourceId || null,
        limit: 50,
      })
        .then((value) => {
          if (generation.current === current) {
            setSearchError("");
            setResults(value);
            setLoading(false);
          }
        })
        .catch((cause) => {
          if (generation.current === current) {
            setResults({ items: [], warnings: [] });
            setSearchError(String(cause));
            setLoading(false);
          }
        });
    }, 200);
    return () => {
      clearTimeout(timer);
      ++generation.current;
    };
  }, [query, sourceId, refresh]);
  const mutate = async (
    action: string,
    payload: Record<string, unknown> = {},
  ) => {
    const outcome = await request<{
      warnings?: string[];
      errors?: string[];
      partial?: boolean;
    } | null>(action, payload);
    if (action === "delete_typed" || action === "clear_typed") {
      ++previewGeneration.current;
      ++generation.current;
      setRefresh((value) => value + 1);
      setSelected(undefined);
      setResults({ items: [], warnings: [] });
      setTypedRemoval(undefined);
    }
    setOperationWarnings([
      ...(outcome?.partial ? [t("knowledge.partialImport")] : []),
      ...(outcome?.warnings ?? []),
      ...(outcome?.errors ?? []),
    ]);
    await reload();
    if (action !== "delete_typed" && action !== "clear_typed") {
      setRefresh((value) => value + 1);
    }
    setConnection(undefined);
  };
  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 p-6">
      <header>
        <h1 className="text-xl font-semibold">{t("knowledge.title")}</h1>
        <p className="mt-2 text-sm opacity-65">{t("knowledge.description")}</p>
      </header>
      {error && (
        <div
          role="alert"
          className="rounded-lg border border-red-500/40 p-3 text-sm text-red-500"
        >
          {t("knowledge.error", { error })}
        </div>
      )}
      {searchError && (
        <p role="alert" className="text-sm text-red-500">
          {t("knowledge.error", { error: searchError })}
        </p>
      )}
      {operationWarnings.map((warning, index) => (
        <p key={index} role="status" className="text-sm text-amber-600">
          {warning}
        </p>
      ))}
      {(busy || notice) && (
        <p role="status" className="text-sm opacity-70">
          {busy ? t("knowledge.working") : notice}
        </p>
      )}
      <section
        className="rounded-xl border border-mid-gray/20 p-4"
        aria-label={t("knowledge.sources")}
      >
        <h2 className="font-medium">{t("knowledge.sources")}</h2>
        <p className="my-2 break-all text-xs opacity-60">
          {status?.managed_path}
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            className={button}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const path = await open({ directory: true, multiple: false });
                if (typeof path === "string")
                  await mutate("add_directory", { path });
              })
            }
          >
            {t("knowledge.addDirectory")}
          </button>
          <button
            className={button}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const paths = await open({ multiple: true, directory: false });
                if (paths)
                  await mutate("import_files", {
                    paths: Array.isArray(paths) ? paths : [paths],
                  });
              })
            }
          >
            {t("knowledge.addFiles")}
          </button>
          <button
            className={button}
            disabled={busy}
            onClick={() => setNote((value) => !value)}
          >
            {t("knowledge.newNote")}
          </button>
          <button
            className={button}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await request("open_managed");
              })
            }
          >
            {t("knowledge.openFolder")}
          </button>
          <button
            className={button}
            disabled={busy}
            onClick={() =>
              void run(async () => {
                const path = await open({ directory: true, multiple: false });
                if (typeof path === "string")
                  await mutate("set_managed_path", { path });
              })
            }
          >
            {t("knowledge.changeFolder")}
          </button>
          <button
            className={button}
            disabled={busy}
            onClick={() => void run(() => mutate("sync"))}
          >
            {t("knowledge.sync")}
          </button>
        </div>
        {note && (
          <form
            className="mt-4 space-y-2"
            onSubmit={(event) => {
              event.preventDefault();
              void run(async () => {
                await mutate("save_note", { title: title.trim(), text });
                setTitle("");
                setText("");
                setNote(false);
              });
            }}
          >
            <input
              className={field}
              disabled={busy}
              aria-label={t("knowledge.noteTitle")}
              placeholder={t("knowledge.noteTitle")}
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              required
            />
            <textarea
              className={field}
              rows={5}
              disabled={busy}
              aria-label={t("knowledge.noteText")}
              placeholder={t("knowledge.noteText")}
              value={text}
              onChange={(event) => setText(event.target.value)}
              required
            />
            <button
              className={button}
              disabled={busy || !title.trim() || !text.trim()}
            >
              {t("knowledge.saveNote")}
            </button>
          </form>
        )}
        <p className="my-3 text-xs opacity-60">
          {t("knowledge.sourceHelp")} {t("knowledge.managedHelp")}{" "}
          {t("knowledge.formats")}
        </p>
        {!status && !error && <p role="status">{t("knowledge.loading")}</p>}
        <ul className="divide-y divide-mid-gray/15">
          {status?.sources.map((source) => (
            <li key={source.id} className="py-3" aria-label={source.name}>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h3 className="text-sm font-medium">{source.name}</h3>
                  <p className="break-all text-xs opacity-60">
                    {source.path || source.kind}
                  </p>
                  <p className="mt-1 text-xs opacity-60">
                    {source.item_count === null
                      ? t("knowledge.unknownCount")
                      : t("knowledge.sourceStatus", {
                          count: source.item_count ?? undefined,
                          defaultValue: "",
                          status: t(`knowledge.states.${source.status}`, {
                            defaultValue: source.status,
                          }),
                        })}
                  </p>
                </div>
                {source.kind === "directory" && (
                  <button
                    className={button}
                    disabled={busy}
                    onClick={() => setRemove(source)}
                  >
                    {t("knowledge.remove")}
                  </button>
                )}
              </div>
              {source.kind === "saved_snippet" &&
                source.status !== "unavailable" && (
                  <div className="mt-3 space-y-2">
                    <label className="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={source.capture_enabled ?? false}
                        disabled={busy}
                        onChange={(event) =>
                          void run(() =>
                            mutate("typed_capture", {
                              enabled: event.target.checked,
                            }),
                          )
                        }
                      />
                      {t("knowledge.captureTyping")}
                    </label>
                    <p className="text-xs opacity-65">
                      {t("knowledge.captureTypingHelp")}
                    </p>
                    <button
                      className={button}
                      disabled={busy}
                      onClick={() => setTypedRemoval("all")}
                    >
                      {t("knowledge.clearTyping")}
                    </button>
                  </div>
                )}
              {source.description && (
                <p className="mt-2 text-xs opacity-65">
                  {source.kind === "saved_snippet" &&
                  source.status === "unavailable"
                    ? t("knowledge.typingUnavailable")
                    : source.description}
                </p>
              )}
              <div className="mt-2 flex flex-wrap gap-4 text-sm">
                {(["enabled", "external_access"] as const).map((key) => (
                  <label key={key} className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={source[key]}
                      disabled={busy || source.status === "unavailable"}
                      onChange={(event) =>
                        void run(() =>
                          mutate("update_source", {
                            id: source.id,
                            enabled: source.enabled,
                            external_access: source.external_access,
                            [key]: event.target.checked,
                          }),
                        )
                      }
                    />
                    {t(
                      key === "enabled"
                        ? "knowledge.enabled"
                        : "knowledge.externalAccess",
                    )}
                  </label>
                ))}
              </div>
            </li>
          ))}
        </ul>
        {remove && (
          <div
            className="mt-3 rounded-lg border border-mid-gray/30 p-3"
            role="group"
            aria-label={t("knowledge.remove")}
          >
            <p className="mb-2 text-sm">
              {t("knowledge.removeConfirm", { name: remove.name })}
            </p>
            <div className="flex gap-2">
              <button
                className={button}
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await mutate("remove_source", { id: remove.id });
                    if (sourceId === remove.id) setSourceId("");
                    setRemove(undefined);
                  })
                }
              >
                {t("knowledge.confirmRemove")}
              </button>
              <button
                className={button}
                disabled={busy}
                onClick={() => setRemove(undefined)}
              >
                {t("knowledge.cancel")}
              </button>
            </div>
          </div>
        )}
      </section>
      {typedRemoval && (
        <div
          role="group"
          aria-label={t("knowledge.confirmTypingRemoval")}
          className="rounded-lg border border-mid-gray/30 p-4"
        >
          <p className="mb-3 text-sm">
            {typedRemoval === "all"
              ? t("knowledge.clearTypingConfirm")
              : t("knowledge.deleteTypingConfirm", {
                  title: typedRemoval.title,
                })}
          </p>
          <div className="flex gap-2">
            <button
              className={button}
              disabled={busy}
              onClick={() =>
                void run(() =>
                  typedRemoval === "all"
                    ? mutate("clear_typed")
                    : mutate("delete_typed", { id: typedRemoval.id }),
                )
              }
            >
              {t("knowledge.confirmTypingRemoval")}
            </button>
            <button
              className={button}
              disabled={busy}
              onClick={() => setTypedRemoval(undefined)}
            >
              {t("knowledge.cancel")}
            </button>
          </div>
        </div>
      )}
      <section aria-label={t("knowledge.search")}>
        <div className="flex gap-2">
          <input
            type="search"
            className={field}
            aria-label={t("knowledge.search")}
            placeholder={t("knowledge.searchPlaceholder")}
            value={query}
            onChange={(event) => {
              setError("");
              setQuery(event.target.value);
            }}
          />
          <select
            className={`${field} max-w-48`}
            aria-label={t("knowledge.sourceFilter")}
            value={sourceId}
            onChange={(event) => setSourceId(event.target.value)}
          >
            <option value="">{t("knowledge.allSources")}</option>
            {status?.sources.map((source) => (
              <option key={source.id} value={source.id}>
                {source.name}
              </option>
            ))}
          </select>
        </div>
        {results.has_more && (
          <p role="status" className="mt-2 text-sm text-amber-600">
            {t("knowledge.moreResults")}
          </p>
        )}
        {results.warnings.map((warning, index) => (
          <p key={index} role="status" className="mt-2 text-sm text-amber-600">
            {warning}
          </p>
        ))}
        {loading ? (
          <p className="py-6 text-sm opacity-60" role="status">
            {t("knowledge.loading")}
          </p>
        ) : (
          <div className="mt-3 grid gap-3 lg:grid-cols-2">
            <ul className="space-y-2" aria-label={t("knowledge.results")}>
              {results.items.length === 0 && (
                <li className="py-6 text-sm opacity-60">
                  {t("knowledge.empty")}
                </li>
              )}
              {results.items.map((item) => (
                <li key={item.id}>
                  <button
                    className="w-full rounded-lg border border-mid-gray/20 p-3 text-left hover:bg-mid-gray/10"
                    onClick={() =>
                      void run(async () => {
                        const current = ++previewGeneration.current;
                        const full = await request<Item>("read", {
                          id: item.id,
                          revision: item.revision,
                        });
                        if (current === previewGeneration.current)
                          setSelected(full);
                      })
                    }
                    disabled={busy}
                  >
                    <p className="text-sm font-medium">{item.title}</p>
                    <p className="mt-1 line-clamp-2 whitespace-pre-wrap text-xs opacity-70">
                      {item.text}
                    </p>
                    <p className="mt-2 truncate text-xs opacity-50">
                      {item.locator}
                    </p>
                  </button>
                  {item.kind === "saved_snippet" && (
                    <button
                      className={`${button} mt-1`}
                      disabled={busy}
                      onClick={(event) => {
                        event.stopPropagation();
                        setTypedRemoval(item);
                      }}
                    >
                      {t("knowledge.deleteTyping")}
                    </button>
                  )}
                </li>
              ))}
            </ul>
            {selected && (
              <aside
                className="rounded-lg border border-mid-gray/20 p-4"
                aria-label={t("knowledge.preview")}
              >
                <h3 className="font-medium">{selected.title}</h3>
                <p className="my-2 break-all text-xs opacity-60">
                  {selected.locator}
                </p>
                <div className="max-h-96 overflow-auto whitespace-pre-wrap break-words text-sm">
                  {selected.text}
                </div>
                {selected.truncated && (
                  <p role="status" className="mt-2 text-sm text-amber-600">
                    {t("knowledge.truncated")}
                  </p>
                )}
              </aside>
            )}
          </div>
        )}
      </section>
      <PersonalizationPanel />
      <section
        className="rounded-xl border border-mid-gray/20 p-4"
        aria-label={t("knowledge.connect")}
      >
        <h2 className="font-medium">{t("knowledge.connect")}</h2>
        <p className="my-2 text-sm opacity-65">
          {t("knowledge.connectionHelp")}
        </p>
        <button
          className={button}
          disabled={busy}
          onClick={() =>
            void run(async () =>
              setConnection(await request<Connection>("export_connection")),
            )
          }
        >
          {t("knowledge.generatePrompt")}
        </button>
        {connection && (
          <div className="mt-3 space-y-3">
            <textarea
              readOnly
              className={`${field} font-mono`}
              rows={10}
              aria-label={t("knowledge.prompt")}
              value={connection.prompt}
            />
            <p className="break-all text-xs opacity-60">
              {t("knowledge.skillPath", { path: connection.skill_path })}
            </p>
            <p className="break-all text-xs opacity-60">
              {t("knowledge.cliPath", { path: connection.cli_path })}
            </p>
            <button
              className={button}
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await writeText(connection.prompt);
                  setNotice(t("knowledge.copied"));
                })
              }
            >
              {t("knowledge.copyPrompt")}
            </button>
          </div>
        )}
      </section>
    </div>
  );
}
