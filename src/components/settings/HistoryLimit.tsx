import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { SettingContainer } from "../ui/SettingContainer";

interface HistoryLimitProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const HistoryLimit: React.FC<HistoryLimitProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const historyLimit = getSetting("history_limit") ?? 5;
  const isUnlimited = historyLimit === 0;
  const finiteLimit = isUnlimited ? 5 : historyLimit;

  const handleChange = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const value = parseInt(event.target.value, 10);
    if (!isNaN(value) && value >= 1) {
      updateSetting("history_limit", value);
    }
  };

  const handleUnlimitedChange = (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    updateSetting("history_limit", event.target.checked ? 0 : finiteLimit);
  };

  return (
    <SettingContainer
      title={t("settings.debug.historyLimit.title")}
      description={t("settings.debug.historyLimit.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <div className="flex flex-wrap items-center gap-3">
        <Input
          type="number"
          min="1"
          max="1000"
          value={isUnlimited ? "" : finiteLimit}
          onChange={handleChange}
          placeholder={t("settings.debug.historyLimit.unlimited")}
          disabled={isUpdating("history_limit") || isUnlimited}
          className="w-20"
        />
        <span className="text-sm text-text">
          {isUnlimited
            ? t("settings.debug.historyLimit.unlimited")
            : t("settings.debug.historyLimit.entries")}
        </span>
        <label className="flex items-center gap-2 text-sm text-text">
          <input
            type="checkbox"
            checked={isUnlimited}
            onChange={handleUnlimitedChange}
            disabled={isUpdating("history_limit")}
            className="h-4 w-4 accent-logo-primary"
          />
          {t("settings.debug.historyLimit.unlimited")}
        </label>
      </div>
    </SettingContainer>
  );
};
