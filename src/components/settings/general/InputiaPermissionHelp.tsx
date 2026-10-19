import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Button } from "../../ui/Button";
import { SettingsGroup } from "../../ui/SettingsGroup";

type PermissionStatus = {
  gate_open: boolean;
  probe_in_flight: boolean;
  recovery_pending?: boolean;
  maintenance_state: "active" | "retiring" | "restart_required" | "inactive";
  permission: { accessibility: string; input_monitoring: string };
  health: { enigo: string; shortcuts: string; targets: string };
  components: { ime?: { state?: string; stale?: boolean } };
};

function backgroundState(status: PermissionStatus | null) {
  if (!status) return "unknown";
  if (status.gate_open) return "ready";
  const health = [
    status.health.enigo,
    status.health.shortcuts,
    status.health.targets,
  ];
  if (status.maintenance_state === "restart_required") return "restart";
  if (
    status.permission.accessibility === "denied" ||
    status.permission.input_monitoring === "denied"
  )
    return "denied";
  if (status.maintenance_state === "active") return "paused";
  if (status.recovery_pending && status.maintenance_state === "inactive")
    return "checking";
  if (health.includes("restart_required")) return "restart";
  if (
    status.probe_in_flight ||
    status.maintenance_state === "retiring" ||
    health.includes("retiring")
  )
    return "checking";
  if (health.every((state) => state === "stopped")) return "paused";
  return "unknown";
}

function recoveryIsRunning(status: PermissionStatus | null) {
  return (
    backgroundState(status) === "checking" &&
    (status?.probe_in_flight === true || status?.recovery_pending === true)
  );
}

export function InputiaPermissionHelp() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<PermissionStatus | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const mounted = useRef(true);
  const rechecking = recoveryIsRunning(status);

  useEffect(() => {
    mounted.current = true;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await invoke<PermissionStatus>(
          "inputia_permission_status",
        );
        if (!cancelled) setStatus(next);
      } catch {
        if (!cancelled) setStatus(null);
      }
      if (!cancelled) timer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => {
      cancelled = true;
      mounted.current = false;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (!rechecking) {
      setMessage((current) =>
        current === "inputiaPermissionHelp.rechecking" ? null : current,
      );
    }
  }, [rechecking]);

  const perform = async (
    action: "component" | "settings" | "resume" | "assist" | "app",
  ) => {
    setBusy(true);
    setMessage(null);
    try {
      if (action === "resume") {
        const next = await invoke<PermissionStatus>(
          "inputia_permission_resume",
        );
        if (mounted.current) {
          setStatus(next);
          setMessage(
            recoveryIsRunning(next) ? "inputiaPermissionHelp.rechecking" : null,
          );
        }
      } else if (action === "settings") {
        let next = await invoke<PermissionStatus>(
          "inputia_permission_prepare_maintenance",
        );
        const deadline = Date.now() + 10000;
        while (
          next.maintenance_state === "retiring" &&
          Date.now() < deadline &&
          mounted.current
        ) {
          setStatus(next);
          await new Promise((resolve) => setTimeout(resolve, 250));
          next = await invoke<PermissionStatus>("inputia_permission_status");
        }
        if (!mounted.current) return;
        setStatus(next);
        if (next.maintenance_state !== "active") {
          setMessage("inputiaPermissionHelp.notStopped");
          return;
        }
        await invoke("open_inputia_permission_help", { action: "settings" });
        setMessage("inputiaPermissionHelp.pausedNote");
      } else if (action === "assist" || action === "app") {
        await invoke("open_inputia_permission_help", { action });
        setMessage(
          action === "assist"
            ? "accessibilityDragAssist.opened"
            : "accessibilityDragAssist.revealed",
        );
      } else {
        await invoke("open_inputia_permission_help", { action });
      }
    } catch {
      if (mounted.current) setMessage("inputiaPermissionHelp.failed");
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const backgroundKey = backgroundState(status);

  return (
    <SettingsGroup title={t("inputiaPermissionHelp.title")}>
      <div className="p-4 space-y-3">
        <p className="text-sm text-mid-gray">
          {t("inputiaPermissionHelp.description")}
        </p>
        <div className="space-y-1 text-sm" aria-live="polite">
          <p>
            {t("inputiaPermissionHelp.background")}:{" "}
            {t(`inputiaPermissionHelp.states.${backgroundKey}`)}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void perform("resume")}
          >
            {t("inputiaPermissionHelp.resume")}
          </Button>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void perform("settings")}
          >
            {t("inputiaPermissionHelp.settings")}
          </Button>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void perform("assist")}
          >
            {t("accessibilityDragAssist.openAssist")}
          </Button>
        </div>
        <p className="text-xs text-mid-gray">
          {t("inputiaPermissionHelp.note")}
        </p>
        {message && (
          <p role="status" className="text-sm text-mid-gray">
            {t(message)}
          </p>
        )}
      </div>
    </SettingsGroup>
  );
}
