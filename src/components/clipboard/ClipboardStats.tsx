import React from "react";
import { useTranslation } from "react-i18next";
import type { ClipboardStats as StatsType } from "@/lib/types/clipboard";

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

interface ClipboardStatsBarProps {
  stats: StatsType | null;
}

export const ClipboardStatsBar: React.FC<ClipboardStatsBarProps> = ({
  stats,
}) => {
  const { t } = useTranslation();

  if (!stats) return null;

  return (
    <div className="flex items-center gap-4 px-4 py-2 text-xs text-text/50">
      <span>
        {t("settings.clipboard.totalItems", { count: stats.total_items })}
      </span>
      <span className="text-text/20">|</span>
      <span>
        {t("settings.clipboard.totalSize", {
          size: formatBytes(stats.total_size_bytes),
        })}
      </span>
      {stats.favorites_count > 0 && (
        <>
          <span className="text-text/20">|</span>
          <span>
            {stats.favorites_count} {t("settings.clipboard.toggleFavorite")}
          </span>
        </>
      )}
      {stats.pinned_count > 0 && (
        <>
          <span className="text-text/20">|</span>
          <span>
            {stats.pinned_count} {t("settings.clipboard.togglePin")}
          </span>
        </>
      )}
    </div>
  );
};
