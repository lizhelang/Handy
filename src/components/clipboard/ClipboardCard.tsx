import React, { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  Star,
  Pin,
  Eye,
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

interface ClipboardCardProps {
  item: ClipboardItem;
  isSelected: boolean;
  onToggleFavorite: () => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onPreview: () => void;
  onCopy: () => void;
  onSelect: () => void;
}

export const ClipboardCard: React.FC<ClipboardCardProps> = ({
  item,
  isSelected,
  onToggleFavorite,
  onTogglePin,
  onDelete,
  onPreview,
  onCopy,
  onSelect,
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
      className={`flex items-center gap-3 px-4 py-2.5 cursor-pointer transition-colors group ${
        isSelected
          ? "bg-logo-primary/10 border-l-2 border-logo-primary"
          : "hover:bg-mid-gray/5 border-l-2 border-transparent"
      }`}
      onClick={onSelect}
      onDoubleClick={onPreview}
    >
      {item.content_type === "image" && item.image_path && !imageError ? (
        <img
          src={getImageUrl(item.image_path) || ""}
          alt={imageAlt}
          className="w-8 h-8 rounded object-cover shrink-0"
          onError={() => setImageError(true)}
        />
      ) : (
        <Icon className="w-4 h-4 text-text/30 shrink-0" />
      )}

      <div className="flex-1 min-w-0">
        <p className="text-sm text-text/90 truncate">{itemLabel}</p>
        <div className="flex items-center gap-2 mt-0.5">
          <span className="text-xs text-text/40">
            {formatClipboardRelativeTime(item.created_at, i18n.language)}
          </span>
          {item.source_app && (
            <span className="text-xs text-text/30">· {item.source_app}</span>
          )}
        </div>
      </div>

      <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        <IconButton
          onClick={(e) => {
            e.stopPropagation();
            onToggleFavorite();
          }}
          title={t("settings.clipboard.toggleFavorite")}
          active={item.is_favorite}
        >
          <Star
            className="w-3.5 h-3.5"
            fill={item.is_favorite ? "currentColor" : "none"}
          />
        </IconButton>
        <IconButton
          onClick={(e) => {
            e.stopPropagation();
            onTogglePin();
          }}
          title={t("settings.clipboard.togglePin")}
          active={item.is_pinned}
        >
          <Pin
            className="w-3.5 h-3.5"
            fill={item.is_pinned ? "currentColor" : "none"}
          />
        </IconButton>
        <IconButton
          onClick={handleCopy}
          title={t("settings.clipboard.copyToClipboard")}
        >
          {showCopied ? (
            <Check className="w-3.5 h-3.5 text-green-500" />
          ) : (
            <Copy className="w-3.5 h-3.5" />
          )}
        </IconButton>
        <IconButton
          onClick={(e) => {
            e.stopPropagation();
            onPreview();
          }}
          title={t("settings.clipboard.preview")}
        >
          <Eye className="w-3.5 h-3.5" />
        </IconButton>
        <IconButton
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
          title={t("settings.clipboard.deleteItem")}
          danger
        >
          <Trash2 className="w-3.5 h-3.5" />
        </IconButton>
      </div>
    </div>
  );
};

const IconButton: React.FC<{
  onClick: (e: React.MouseEvent) => void;
  title: string;
  active?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, active, danger, children }) => (
  <button
    onClick={onClick}
    className={`p-1 rounded-md flex items-center justify-center transition-colors cursor-pointer ${
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
