import React from "react";
import { useTranslation } from "react-i18next";
import { ClipboardList } from "lucide-react";

export const ClipboardEmpty: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col items-center justify-center py-16 px-4 text-center">
      <ClipboardList className="w-12 h-12 text-text/20 mb-4" />
      <h3 className="text-sm font-medium text-text/60 mb-1">
        {t("settings.clipboard.emptyTitle")}
      </h3>
      <p className="text-xs text-text/40 max-w-xs">
        {t("settings.clipboard.emptyDescription")}
      </p>
    </div>
  );
};
