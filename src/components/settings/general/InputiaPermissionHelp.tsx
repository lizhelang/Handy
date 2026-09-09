import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Button } from "../../ui/Button";
import { SettingsGroup } from "../../ui/SettingsGroup";

export function InputiaPermissionHelp() {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const navigate = async (action: "component" | "settings") => {
    setBusy(true);
    setError(null);
    try {
      await invoke("open_inputia_permission_help", { action });
    } catch (reason) {
      setError(
        t(
          reason === "component_missing"
            ? "inputiaPermissionHelp.missing"
            : "inputiaPermissionHelp.failed",
        ),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsGroup title={t("inputiaPermissionHelp.title")}>
      <div className="p-4 space-y-3">
        <p className="text-sm text-mid-gray">
          {t("inputiaPermissionHelp.description")}
        </p>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void navigate("component")}
          >
            {t("inputiaPermissionHelp.component")}
          </Button>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void navigate("settings")}
          >
            {t("inputiaPermissionHelp.settings")}
          </Button>
        </div>
        <p className="text-xs text-mid-gray">
          {t("inputiaPermissionHelp.note")}
        </p>
        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}
      </div>
    </SettingsGroup>
  );
}
