import { useCallback, useEffect, useRef } from "react";
import { commands } from "@/bindings";
import { toast } from "sonner";
import i18n from "i18next";

interface CaptureEnd {
  released: boolean;
  canSave: boolean;
}
const NOT_RELEASED: CaptureEnd = { released: false, canSave: false };

/** 只释放本组件收到的 token；迟到 begin 回复不会进入已卸载的录制界面。 */
export function useShortcutCapture(
  native: boolean,
  onSecureInputBlocked?: () => void,
  onExpired?: () => void,
) {
  const expired = useRef(onExpired);
  expired.current = onExpired;
  const deadlineTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const clearDeadline = useCallback(() => {
    if (deadlineTimer.current !== null) clearTimeout(deadlineTimer.current);
    deadlineTimer.current = null;
  }, []);
  const secureInputBlocked = useRef(onSecureInputBlocked);
  secureInputBlocked.current = onSecureInputBlocked;
  const mounted = useRef(true);
  const generation = useRef(0);
  const token = useRef<string | null>(null);
  const starting = useRef(false);
  const finishing = useRef<Promise<CaptureEnd> | null>(null);
  const notifyFailure = useCallback(() => {
    toast.error(i18n.t("unifiedHistory.feedback.updateFailed"));
  }, []);
  const releaseToken = useCallback(
    async (ownedToken: string) => {
      const result = native
        ? await commands.stopHandyKeysRecording(ownedToken)
        : await commands.resumeAllBindings(ownedToken);
      if (result.status === "error")
        throw new Error("shortcut_capture_release_failed");
      return result.data;
    },
    [native],
  );
  const end = useCallback(async (): Promise<CaptureEnd> => {
    if (finishing.current) return finishing.current;
    const ownedToken = token.current;
    if (!ownedToken) return NOT_RELEASED;
    const request = (async () => {
      try {
        const canSave = await releaseToken(ownedToken);
        if (token.current === ownedToken) {
          token.current = null;
          clearDeadline();
        }
        if (!canSave) notifyFailure();
        return {
          released: mounted.current,
          canSave: mounted.current && canSave,
        };
      } catch {
        notifyFailure();
        return NOT_RELEASED;
      }
    })();
    finishing.current = request;
    try {
      return await request;
    } finally {
      if (finishing.current === request) finishing.current = null;
    }
  }, [notifyFailure, releaseToken, clearDeadline]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      clearDeadline();
      void end();
    };
  }, [end, clearDeadline]);
  const begin = useCallback(
    async (bindingId: string): Promise<boolean> => {
      if (
        !mounted.current ||
        starting.current ||
        token.current ||
        finishing.current
      )
        return false;
      starting.current = true;
      const attempt = ++generation.current;
      let accepted = false;
      // 从请求发出时开始，比后端 begin 的固定60秒期限更保守；回复/重试不能续租。
      deadlineTimer.current = setTimeout(() => {
        if (generation.current !== attempt) return;
        generation.current += 1;
        expired.current?.();
        void end();
      }, 60_000);
      try {
        const result = native
          ? await commands.startHandyKeysRecording(bindingId)
          : await commands.suspendAllBindings(bindingId);
        if (result.status === "error") {
          if (
            result.error === "secure-input-active" &&
            secureInputBlocked.current
          )
            secureInputBlocked.current();
          else notifyFailure();
          return false;
        }
        if (!mounted.current || generation.current !== attempt) {
          // 只结束迟到响应自己的 token，不读取或释放后来录制的 token。
          await releaseToken(result.data);
          return false;
        }
        token.current = result.data;
        accepted = true;
        return true;
      } catch {
        notifyFailure();
        return false;
      } finally {
        if (!accepted) clearDeadline();
        starting.current = false;
      }
    },
    [native, notifyFailure, releaseToken, clearDeadline, end],
  );
  const owns = useCallback(
    (receivedToken: string) =>
      mounted.current && token.current === receivedToken,
    [],
  );
  return { begin, end, owns };
}
