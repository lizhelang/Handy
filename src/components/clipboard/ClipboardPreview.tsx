import React from "react";
import { useTranslation } from "react-i18next";
import {
  X,
  Star,
  Pin,
  Copy,
  FileText,
  Image,
  FileCode,
  File,
} from "lucide-react";
import type { ClipboardItem } from "@/lib/types/clipboard";

function formatDateTime(dateStr: string): string {
  const date = new Date(dateStr);
  return date.toLocaleString();
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

const contentIcons: Record<
  string,
  React.ComponentType<{ className?: string }>
> = {
  text: FileText,
  image: Image,
  richtext: FileCode,
  file: File,
};

interface ClipboardPreviewProps {
  item: ClipboardItem;
  onClose: () => void;
  onCopy: () => void;
  onToggleFavorite: () => void;
  onTogglePin: () => void;
}

export const ClipboardPreview: React.FC<ClipboardPreviewProps> = ({
  item,
  onClose,
  onCopy,
  onToggleFavorite,
  onTogglePin,
}) => {
  const { t } = useTranslation();
  const Icon = contentIcons[item.content_type] || FileText;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="bg-background border border-mid-gray/30 rounded-xl shadow-2xl max-w-lg w-full mx-4 max-h-[80vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-mid-gray/20">
          <div className="flex items-center gap-2">
            <Icon className="w-4 h-4 text-text/40" />
            <span className="text-sm font-medium text-text/80">
              {item.content_type.toUpperCase()}
            </span>
            <span className="text-xs text-text/40">
              {formatBytes(item.size_bytes)}
            </span>
          </div>
          <div className="flex items-center gap-1">
            <PreviewIconButton
              onClick={onToggleFavorite}
              title={t("settings.clipboard.toggleFavorite")}
              active={item.is_favorite}
            >
              <Star
                className="w-4 h-4"
                fill={item.is_favorite ? "currentColor" : "none"}
              />
            </PreviewIconButton>
            <PreviewIconButton
              onClick={onTogglePin}
              title={t("settings.clipboard.togglePin")}
              active={item.is_pinned}
            >
              <Pin
                className="w-4 h-4"
                fill={item.is_pinned ? "currentColor" : "none"}
              />
            </PreviewIconButton>
            <PreviewIconButton
              onClick={onCopy}
              title={t("settings.clipboard.copyToClipboard")}
            >
              <Copy className="w-4 h-4" />
            </PreviewIconButton>
            <PreviewIconButton onClick={onClose} title={t("common.close")}>
              <X className="w-4 h-4" />
            </PreviewIconButton>
          </div>
        </div>

        <div className="flex-1 overflow-auto p-4">
          {item.content_type === "image" ? (
            <div className="flex items-center justify-center py-8">
              <Image className="w-16 h-16 text-text/20" />
              <p className="text-sm text-text/40 ml-3">
                {item.content_preview}
              </p>
            </div>
          ) : (
            <pre className="text-sm text-text/90 whitespace-pre-wrap break-words font-mono leading-relaxed">
              {item.full_text || item.content_preview}
            </pre>
          )}
        </div>

        <div className="flex items-center justify-between px-4 py-2 border-t border-mid-gray/20 text-xs text-text/40">
          <span>{formatDateTime(item.created_at)}</span>
          {item.source_app && <span>{item.source_app}</span>}
        </div>
      </div>
    </div>
  );
};

const PreviewIconButton: React.FC<{
  onClick: () => void;
  title: string;
  active?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, active, children }) => (
  <button
    onClick={onClick}
    className={`p-1.5 rounded-md transition-colors cursor-pointer ${
      active
        ? "text-logo-primary hover:text-logo-primary/80"
        : "text-text/50 hover:text-logo-primary"
    }`}
    title={title}
  >
    {children}
  </button>
);
