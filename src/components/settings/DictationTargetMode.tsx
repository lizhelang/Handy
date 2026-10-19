import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown, type DropdownOption } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import type { DictationTargetMode } from "@/bindings";

interface DictationTargetModeProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const DictationTargetModeSetting: React.FC<DictationTargetModeProps> =
  React.memo(({ descriptionMode = "inline", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const osType = useOsType();
    const selectedMode =
      getSetting("dictation_target_mode") ?? "current_cursor";
    const options: DropdownOption[] = [
      {
        value: "current_cursor",
        label: t("settings.advanced.dictationTargetMode.options.currentCursor"),
        description: t(
          "settings.advanced.dictationTargetMode.descriptions.currentCursor",
        ),
      },
      {
        value: "recording_application",
        label: t(
          "settings.advanced.dictationTargetMode.options.recordingApplication",
        ),
        description: t(
          "settings.advanced.dictationTargetMode.descriptions.recordingApplication",
        ),
      },
    ];

    if (osType !== "macos") return null;

    return (
      <SettingContainer
        title={t("settings.advanced.dictationTargetMode.title")}
        description={t("settings.advanced.dictationTargetMode.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          options={options}
          selectedValue={selectedMode}
          onSelect={(value) =>
            updateSetting("dictation_target_mode", value as DictationTargetMode)
          }
          disabled={isUpdating("dictation_target_mode")}
          menuClassName="right-0 min-w-[280px] w-[340px] max-w-[calc(100vw-2rem)]"
        />
      </SettingContainer>
    );
  });
