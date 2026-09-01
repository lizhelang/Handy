import React, { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useClipboardStore } from "@/stores/clipboardStore";
import { ClipboardGridCard } from "./ClipboardGridCard";
import { ClipboardEmpty } from "./ClipboardEmpty";

export const ClipboardGrid: React.FC = () => {
  const { t } = useTranslation();
  const items = useClipboardStore((s) => s.items);
  const itemOrder = useClipboardStore((s) => s.itemOrder);
  const hasMore = useClipboardStore((s) => s.hasMore);
  const isLoading = useClipboardStore((s) => s.isLoading);
  const isSearching = useClipboardStore((s) => s.isSearching);
  const loadMore = useClipboardStore((s) => s.loadMore);
  const toggleFavorite = useClipboardStore((s) => s.toggleFavorite);
  const togglePin = useClipboardStore((s) => s.togglePin);
  const deleteItem = useClipboardStore((s) => s.deleteItem);
  const copyItem = useClipboardStore((s) => s.copyItem);
  const showPreview = useClipboardStore((s) => s.showPreview);

  const sentinelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore || isLoading || isSearching) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting) {
          loadMore();
        }
      },
      { threshold: 0 },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, isLoading, isSearching, loadMore]);

  const orderedItems = itemOrder.map((id) => items[id]).filter(Boolean);

  if (orderedItems.length === 0 && !isLoading) {
    return <ClipboardEmpty />;
  }

  return (
    <div className="p-4">
      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-3">
        {orderedItems.map((item) => (
          <ClipboardGridCard
            key={item.id}
            item={item}
            onToggleFavorite={() => toggleFavorite(item.id)}
            onTogglePin={() => togglePin(item.id)}
            onDelete={() => deleteItem(item.id)}
            onCopy={() => copyItem(item.id)}
            onPreview={() => showPreview(item.id)}
          />
        ))}
      </div>
      {isLoading && (
        <div className="py-3 text-center text-sm text-text/40">
          {t("common.loading")}
        </div>
      )}
      <div ref={sentinelRef} className="h-1" />
    </div>
  );
};
