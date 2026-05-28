import React, { useEffect, useRef } from "react";
import { useClipboardStore } from "@/stores/clipboardStore";
import { ClipboardCard } from "./ClipboardCard";
import { ClipboardEmpty } from "./ClipboardEmpty";

export const ClipboardList: React.FC = () => {
  const items = useClipboardStore((s) => s.items);
  const itemOrder = useClipboardStore((s) => s.itemOrder);
  const selectedId = useClipboardStore((s) => s.selectedId);
  const hasMore = useClipboardStore((s) => s.hasMore);
  const isLoading = useClipboardStore((s) => s.isLoading);
  const isSearching = useClipboardStore((s) => s.isSearching);
  const loadMore = useClipboardStore((s) => s.loadMore);
  const toggleFavorite = useClipboardStore((s) => s.toggleFavorite);
  const togglePin = useClipboardStore((s) => s.togglePin);
  const deleteItem = useClipboardStore((s) => s.deleteItem);
  const copyItem = useClipboardStore((s) => s.copyItem);
  const showPreview = useClipboardStore((s) => s.showPreview);
  const setSelectedId = useClipboardStore((s) => s.setSelectedId);

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
    <div className="divide-y divide-mid-gray/10">
      {orderedItems.map((item) => (
        <ClipboardCard
          key={item.id}
          item={item}
          isSelected={selectedId === item.id}
          onToggleFavorite={() => toggleFavorite(item.id)}
          onTogglePin={() => togglePin(item.id)}
          onDelete={() => deleteItem(item.id)}
          onPreview={() => showPreview(item.id)}
          onCopy={() => copyItem(item.id)}
          onSelect={() => setSelectedId(item.id)}
        />
      ))}
      {isLoading && (
        <div className="px-4 py-3 text-center text-sm text-text/40">
          Loading...
        </div>
      )}
      <div ref={sentinelRef} className="h-1" />
    </div>
  );
};
