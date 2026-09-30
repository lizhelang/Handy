import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useSettings } from "../../hooks/useSettings";
import {
  checkProductUpdate,
  type ProductUpdateReply,
} from "@/lib/productUpdates";

interface UpdateCheckerProps {
  className?: string;
}

const UpdateChecker: React.FC<UpdateCheckerProps> = ({ className = "" }) => {
  const { t } = useTranslation();
  const { settings, isLoading, updateChecksLocked } = useSettings();
  const loaded = !isLoading && settings !== null && updateChecksLocked !== null;
  const enabled =
    loaded && settings.update_checks_enabled && updateChecksLocked === false;
  const [reply, setReply] = useState<ProductUpdateReply | null>(null);
  const [checking, setChecking] = useState(false);
  const generationRef = useRef(0);
  const busyRef = useRef(false);
  const retryRef = useRef<ReturnType<typeof setTimeout>>();
  const retryCountRef = useRef(0);
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const check = useCallback(async () => {
    if (!enabledRef.current || busyRef.current) return;
    clearTimeout(retryRef.current);
    busyRef.current = true;
    const generation = ++generationRef.current;
    setChecking(true);
    try {
      const result = await checkProductUpdate();
      if (generation === generationRef.current && enabledRef.current) {
        setReply(result);
        // 同进程旧请求仍在收尾时跟进查询；禁用或卸载会取消，最多等待两分钟。
        if (result.status === "checking" && retryCountRef.current < 60) {
          retryCountRef.current += 1;
          retryRef.current = setTimeout(() => {
            void check();
          }, 2_000);
        } else if (result.status === "checking") {
          retryCountRef.current = 0;
          setReply({
            status: "unavailable",
            reason: "check_timeout",
            version: null,
            release_id: null,
            installable: false,
          });
        } else {
          retryCountRef.current = 0;
        }
      }
    } catch {
      if (generation === generationRef.current && enabledRef.current) {
        setReply({
          status: "unavailable",
          reason: "network",
          version: null,
          release_id: null,
          installable: false,
        });
      }
    } finally {
      if (generation === generationRef.current) {
        busyRef.current = false;
        setChecking(false);
      }
    }
  }, []);

  useEffect(() => {
    if (!enabled) {
      setReply(null);
      setChecking(false);
      return;
    }
    void check();
    const unlisten = listen("check-for-updates", () => {
      void check();
    });
    return () => {
      generationRef.current += 1;
      busyRef.current = false;
      clearTimeout(retryRef.current);
      retryCountRef.current = 0;
      void unlisten.then((stop) => stop()).catch(() => {});
    };
  }, [enabled, check]);

  let label = t("footer.checkForUpdates");
  if (loaded && !enabled) label = t("footer.updateCheckingDisabled");
  else if (checking) label = t("footer.checkingUpdates");
  else if (reply?.status === "current") label = t("footer.upToDate");
  else if (reply?.status === "available")
    label = t("footer.pairedUpdateAvailable", { version: reply.version ?? "" });
  else if (reply?.status === "disabled")
    label = t("footer.updateCheckingDisabled");
  else if (reply?.status === "checking") label = t("footer.updateCheckBusy");
  else if (reply?.status === "unavailable") {
    const key =
      reply.reason === "source_unconfigured"
        ? "footer.updateSourceUnavailable"
        : reply.reason === "installation_repair"
          ? "footer.updateInstallationRepair"
          : reply.reason === "maintenance"
            ? "footer.updateMaintenance"
            : reply.reason === "trust_refresh_required"
              ? "footer.updateTrustRefresh"
              : "footer.updateCheckFailed";
    label = t(key);
  }

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <button
        type="button"
        onClick={() => {
          void check();
        }}
        disabled={!enabled || checking}
        aria-busy={checking}
        className="text-text/60 hover:text-text/80 disabled:opacity-50 tabular-nums"
        title={t("footer.checkForUpdates")}
      >
        <span role="status">{label}</span>
      </button>
    </div>
  );
};
export default UpdateChecker;
