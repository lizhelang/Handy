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
  className?: string;
}

export const ClipboardStatsBar: React.FC<ClipboardStatsBarProps> = ({
  stats,
  className = "",
}) => {
  const { t } = useTranslation();

  if (!stats) return null;

  const statItems = [
    t("settings.clipboard.totalItems", { count: stats.total_items }),
    t("settings.clipboard.totalSize", {
      size: formatBytes(stats.total_size_bytes),
    }),
    stats.favorites_count > 0
      ? `${stats.favorites_count} ${t("settings.clipboard.toggleFavorite")}`
      : null,
    stats.pinned_count > 0
      ? `${stats.pinned_count} ${t("settings.clipboard.togglePin")}`
      : null,
  ].filter(Boolean) as string[];

  return (
    <div
      className={`flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-2 text-xs text-text/50 ${className}`}
    >
      {statItems.map((item, index) => (
        <span key={`${item}-${index}`} className="flex items-center gap-1.5">
          {index > 0 && (
            <span aria-hidden="true" className="text-text/20">
              |
            </span>
          )}
          <span className="whitespace-nowrap">{item}</span>
        </span>
      ))}
    </div>
  );
};
