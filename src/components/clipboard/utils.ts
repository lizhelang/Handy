import type { TFunction } from "i18next";
import type { ClipboardItem } from "@/lib/types/clipboard";

const imagePreviewPattern = /^Image\s+(\d+)x(\d+)$/i;

export function formatClipboardRelativeTime(
  dateStr: string,
  locale: string,
): string {
  const date = new Date(dateStr);

  if (Number.isNaN(date.getTime())) {
    return dateStr;
  }

  const diffInSeconds = Math.floor((date.getTime() - Date.now()) / 1000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const absDiffInSeconds = Math.abs(diffInSeconds);

  if (absDiffInSeconds < 60) {
    return rtf.format(diffInSeconds, "second");
  }

  const diffInMinutes = Math.round(diffInSeconds / 60);
  if (Math.abs(diffInMinutes) < 60) {
    return rtf.format(diffInMinutes, "minute");
  }

  const diffInHours = Math.round(diffInMinutes / 60);
  if (Math.abs(diffInHours) < 24) {
    return rtf.format(diffInHours, "hour");
  }

  const diffInDays = Math.round(diffInHours / 24);
  return rtf.format(diffInDays, "day");
}

export function getClipboardTypeLabel(
  t: TFunction,
  contentType: ClipboardItem["content_type"],
): string {
  switch (contentType) {
    case "text":
      return t("settings.clipboard.filterText");
    case "image":
      return t("settings.clipboard.filterImage");
    case "richtext":
      return t("settings.clipboard.filterRichtext");
    case "file":
      return t("settings.clipboard.filterFiles");
    default:
      return contentType;
  }
}

export function getClipboardItemLabel(
  t: TFunction,
  item: ClipboardItem,
): string {
  if (item.content_type !== "image") {
    return item.content_preview;
  }

  const match = item.content_preview.match(imagePreviewPattern);
  if (match) {
    return `${getClipboardTypeLabel(t, item.content_type)} ${match[1]}x${match[2]}`;
  }

  return getClipboardTypeLabel(t, item.content_type);
}
