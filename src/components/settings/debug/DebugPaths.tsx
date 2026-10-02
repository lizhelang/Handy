import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { join } from "@tauri-apps/api/path";
import { commands } from "@/bindings";
import { SettingContainer } from "../../ui/SettingContainer";

interface DebugPathsProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const DebugPaths: React.FC<DebugPathsProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [paths, setPaths] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const loadPaths = async () => {
      try {
        const result = await commands.getAppDirPath();
        if (result.status === "error") throw new Error(result.error);
        const appDataPath = result.data;
        const resolved = [
          appDataPath,
          await join(appDataPath, "models"),
          await join(appDataPath, "settings_store.json"),
        ];
        if (active) setPaths(resolved);
      } catch (err) {
        if (active) setError(err instanceof Error ? err.message : String(err));
      }
    };
    void loadPaths();
    return () => {
      active = false;
    };
  }, []);

  const labels = ["appData", "models", "settings"] as const;

  return (
    <SettingContainer
      title={t("settings.debug.paths.title")}
      description={t("settings.debug.paths.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <div className="text-sm text-gray-600 space-y-2">
        {error ? (
          <p>{t("errors.loadDirectory", { error })}</p>
        ) : paths.length === 0 ? (
          <p>{t("common.loading")}</p>
        ) : (
          labels.map((label, index) => (
            <div key={label}>
              <span className="font-medium">
                {t(`settings.debug.paths.${label}`)}
              </span>{" "}
              <span className="font-mono text-xs select-text">
                {paths[index]}
              </span>
            </div>
          ))
        )}
      </div>
    </SettingContainer>
  );
};
