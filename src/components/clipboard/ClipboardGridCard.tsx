import React, { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  Star,
  Pin,
  Trash2,
  Copy,
  Check,
  FileText,
  Image,
  FileCode,
  File,
} from "lucide-react";
import type { ClipboardItem } from "@/lib/types/clipboard";
import {
  formatClipboardRelativeTime,
  getClipboardItemLabel,
  getClipboardTypeLabel,
} from "./utils";

const contentIcons: Record<
  string,
  React.ComponentType<{ className?: string }>
> = {
  text: FileText,
  image: Image,
  richtext: FileCode,
  file: File,
};

interface ClipboardGridCardProps {
  item: ClipboardItem;
  onToggleFavorite: () => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onCopy: () => void;
  onPreview: () => void;
}

export const ClipboardGridCard: React.FC<ClipboardGridCardProps> = ({
  item,
  onToggleFavorite,
  onTogglePin,
  onDelete,
  onCopy,
  onPreview,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  const [imageError, setImageError] = useState(false);
  const Icon = contentIcons[item.content_type] || FileText;
  const itemLabel = getClipboardItemLabel(t, item);
  const imageAlt = getClipboardTypeLabel(t, item.content_type);

  const getImageUrl = useCallback((path: string) => {
    try {
      return convertFileSrc(path);
    } catch {
      return null;
    }
  }, []);

  const handleCopy = (e: React.MouseEvent) => {
    e.stopPropagation();
    onCopy();
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 1500);
  };

  return (
    <div
      className="flex flex-col border border-mid-gray/20 rounded-lg overflow-hidden hover:border-logo-primary/30 transition-colors cursor-pointer group"
      onDoubleClick={onPreview}
    >
      <div className="flex-1 p-3 min-h-[80px] flex items-center justify-center bg-mid-gray/5">
        {item.content_type === "image" ? (
          item.image_path && !imageError ? (
            <img
              src={getImageUrl(item.image_path) || ""}
              alt={imageAlt}
              className="w-full h-full object-cover rounded"
              onError={() => setImageError(true)}
            />
          ) : (
            <Image className="w-8 h-8 text-text/20" />
          )
        ) : (
          <p className="text-xs text-text/70 line-clamp-4 text-center leading-relaxed">
            {itemLabel}
          </p>
        )}
      </div>

      <div className="flex items-center justify-between px-2.5 py-1.5 border-t border-mid-gray/10">
        <div className="flex items-center gap-1.5 min-w-0">
          <Icon className="w-3 h-3 text-text/30 shrink-0" />
          <span className="text-xs text-text/40 truncate">
            {formatClipboardRelativeTime(item.created_at, i18n.language)}
          </span>
        </div>
        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
          <GridIconButton
            onClick={(e) => {
              e.stopPropagation();
              onToggleFavorite();
            }}
            title={t("settings.clipboard.toggleFavorite")}
            active={item.is_favorite}
          >
            <Star
              className="w-3 h-3"
              fill={item.is_favorite ? "currentColor" : "none"}
            />
          </GridIconButton>
          <GridIconButton
            onClick={(e) => {
              e.stopPropagation();
              onTogglePin();
            }}
            title={t("settings.clipboard.togglePin")}
            active={item.is_pinned}
          >
            <Pin
              className="w-3 h-3"
              fill={item.is_pinned ? "currentColor" : "none"}
            />
          </GridIconButton>
          <GridIconButton
            onClick={handleCopy}
            title={t("settings.clipboard.copyToClipboard")}
          >
            {showCopied ? (
              <Check className="w-3 h-3 text-green-500" />
            ) : (
              <Copy className="w-3 h-3" />
            )}
          </GridIconButton>
          <GridIconButton
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
            title={t("settings.clipboard.deleteItem")}
            danger
          >
            <Trash2 className="w-3 h-3" />
          </GridIconButton>
        </div>
      </div>
    </div>
  );
};

const GridIconButton: React.FC<{
  onClick: (e: React.MouseEvent) => void;
  title: string;
  active?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, active, danger, children }) => (
  <button
    onClick={onClick}
    className={`p-0.5 rounded transition-colors cursor-pointer ${
      active
        ? "text-logo-primary"
        : danger
          ? "text-text/40 hover:text-red-400"
          : "text-text/40 hover:text-logo-primary"
    }`}
    title={title}
  >
    {children}
  </button>
);
