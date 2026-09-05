import { create } from "zustand";
import type { UnifiedHistoryActionResult } from "@/lib/types/unifiedHistory";

export type OutputAction = "copy" | "insert";
type OutputStatus = UnifiedHistoryActionResult["status"] | "inflight";
export interface OutputAttempt {
  itemId: string;
  revision: number;
  action: OutputAction;
  operationId: string;
  status: OutputStatus;
}
const storageKey = "handy.unified-output-metadata.v1";
const keyOf = (itemId: string, action: OutputAction) =>
  JSON.stringify([itemId, action]);
export const outputNeedsAcknowledgement = (attempt?: OutputAttempt) =>
  attempt?.status === "uncertain" || attempt?.status === "dispatched";

// Only unresolved operation identities are persisted: never text, titles, paths or assets.
function load(): {
  attempts: Record<string, OutputAttempt>;
  storageFailed: boolean;
} {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(storageKey) || "[]");
    if (!Array.isArray(value)) throw new Error("Invalid output metadata");
    const attempts: Record<string, OutputAttempt> = {};
    for (const entry of value) {
      if (!entry || typeof entry !== "object")
        throw new Error("Invalid output metadata");
      const item = entry as OutputAttempt;
      if (
        typeof item.itemId !== "string" ||
        !Number.isSafeInteger(item.revision) ||
        item.revision < 0 ||
        typeof item.operationId !== "string" ||
        !/^[0-9a-f-]{36}$/.test(item.operationId) ||
        !["copy", "insert"].includes(item.action) ||
        !["inflight", "uncertain", "dispatched"].includes(item.status)
      )
        throw new Error("Invalid output metadata");
      attempts[keyOf(item.itemId, item.action)] = {
        itemId: item.itemId,
        revision: item.revision,
        action: item.action,
        operationId: item.operationId,
        status: item.status === "inflight" ? "uncertain" : item.status,
      };
    }
    return { attempts, storageFailed: false };
  } catch {
    // Never silently discard unreadable receipts and permit a fresh dispatch.
    return { attempts: {}, storageFailed: true };
  }
}

interface OutputStore {
  attempts: Record<string, OutputAttempt>;
  storageFailed: boolean;
  begin: (
    itemId: string,
    revision: number,
    action: OutputAction,
    receiptOnly?: boolean,
  ) => OutputAttempt | null;
  finish: (
    attempt: OutputAttempt,
    status: UnifiedHistoryActionResult["status"],
  ) => void;
  acknowledge: (itemId: string, action: OutputAction) => void;
}

export const useUnifiedOutputStore = create<OutputStore>((set, get) => {
  const persist = (attempts: Record<string, OutputAttempt>) => {
    try {
      localStorage.setItem(
        storageKey,
        JSON.stringify(
          Object.values(attempts).filter(
            (attempt) =>
              attempt.status === "inflight" ||
              outputNeedsAcknowledgement(attempt),
          ),
        ),
      );
      set({ attempts });
      return true;
    } catch {
      set({ attempts, storageFailed: true });
      return false;
    }
  };
  return {
    ...load(),
    begin: (itemId, revision, action, receiptOnly = false) => {
      const { attempts, storageFailed } = get();
      if (
        storageFailed ||
        Object.values(attempts).some((attempt) => attempt.status === "inflight")
      )
        return null;
      const key = keyOf(itemId, action);
      const previous = attempts[key];
      if (
        receiptOnly
          ? !outputNeedsAcknowledgement(previous)
          : outputNeedsAcknowledgement(previous)
      )
        return null;
      const attempt: OutputAttempt = receiptOnly
        ? { ...previous, status: "inflight" }
        : {
            itemId,
            revision,
            action,
            operationId: crypto.randomUUID(),
            status: "inflight",
          };
      return persist({ ...attempts, [key]: attempt }) ? attempt : null;
    },
    finish: (attempt, status) => {
      const key = keyOf(attempt.itemId, attempt.action);
      const attempts = get().attempts;
      if (attempts[key]?.operationId !== attempt.operationId) return;
      const next = { ...attempts };
      if (status === "uncertain" || status === "dispatched")
        next[key] = { ...attempt, status };
      else delete next[key];
      persist(next);
    },
    acknowledge: (itemId, action) => {
      const key = keyOf(itemId, action);
      if (!outputNeedsAcknowledgement(get().attempts[key])) return;
      const next = { ...get().attempts };
      delete next[key];
      persist(next);
    },
  };
});

export function getOutputAttempt(
  itemId: string,
  action: OutputAction,
): OutputAttempt | undefined {
  return useUnifiedOutputStore.getState().attempts[keyOf(itemId, action)];
}

export function requireActiveOutputAttempt(
  itemId: string,
  action: OutputAction,
): OutputAttempt {
  const attempt = getOutputAttempt(itemId, action);
  if (!attempt || attempt.status !== "inflight")
    throw new Error("Missing output operation identity");
  return attempt;
}
