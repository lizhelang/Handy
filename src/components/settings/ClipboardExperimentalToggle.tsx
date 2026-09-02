import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "../../stores/settingsStore";
import { ToggleSwitch } from "../ui/ToggleSwitch";

type ClipboardFeatureSettings = {
  clipboard_enabled?: boolean;
};

interface ClipboardExperimentalToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ClipboardExperimentalToggle: React.FC<ClipboardExperimentalToggleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const settings = useSettingsStore((state) => state.settings);
    const refreshSettings = useSettingsStore((state) => state.refreshSettings);
    const [isUpdating, setIsUpdating] = useState(false);

    const enabled = Boolean(
      (settings as ClipboardFeatureSettings | null)?.clipboard_enabled ?? false,
    );

    const handleChange = async (nextEnabled: boolean) => {
      setIsUpdating(true);
      try {
        await invoke("change_clipboard_enabled_setting", {
          enabled: nextEnabled,
        });
        await refreshSettings();
      } catch (error) {
        console.error(
          "Failed to update clipboard experimental feature:",
          error,
        );
      } finally {
        setIsUpdating(false);
      }
    };

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={handleChange}
        isUpdating={isUpdating}
        label={t("settings.advanced.clipboardFeature.label")}
        description={t("settings.advanced.clipboardFeature.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
