import React, { useCallback } from "react";
import { useTranslation } from "react-i18next";
import {
  Search,
  LayoutList,
  LayoutGrid,
  ArrowUpDown,
  Filter,
  X,
} from "lucide-react";
import { useClipboardStore } from "@/stores/clipboardStore";
import type {
  ClipboardViewMode,
  ClipboardSortOrder,
  ClipboardContentTypeFilter,
} from "@/lib/types/clipboard";

const contentTypeOptions: {
  value: ClipboardContentTypeFilter;
  labelKey: string;
}[] = [
  { value: "all", labelKey: "settings.clipboard.filterAll" },
  { value: "text", labelKey: "settings.clipboard.filterText" },
  { value: "image", labelKey: "settings.clipboard.filterImage" },
  { value: "richtext", labelKey: "settings.clipboard.filterRichtext" },
  { value: "file", labelKey: "settings.clipboard.filterFiles" },
];

export const ClipboardToolbar: React.FC = () => {
  const { t } = useTranslation();
  const searchQuery = useClipboardStore((s) => s.searchQuery);
  const viewMode = useClipboardStore((s) => s.viewMode);
  const sortOrder = useClipboardStore((s) => s.sortOrder);
  const contentTypeFilter = useClipboardStore((s) => s.contentTypeFilter);
  const search = useClipboardStore((s) => s.search);
  const setViewMode = useClipboardStore((s) => s.setViewMode);
  const setSortOrder = useClipboardStore((s) => s.setSortOrder);
  const setContentTypeFilter = useClipboardStore((s) => s.setContentTypeFilter);

  const handleSearch = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      search(e.target.value);
    },
    [search],
  );

  const clearSearch = useCallback(() => {
    search("");
  }, [search]);

  return (
    <div className="flex flex-col gap-2 px-4 py-3">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-text/30" />
          <input
            type="text"
            value={searchQuery}
            onChange={handleSearch}
            placeholder={t("settings.clipboard.searchPlaceholder")}
            className="w-full pl-8 pr-8 py-1.5 text-sm bg-mid-gray/10 border border-mid-gray/20 rounded-lg focus:outline-none focus:border-logo-primary/50 placeholder:text-text/30"
          />
          {searchQuery && (
            <button
              onClick={clearSearch}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-text/30 hover:text-text/60 cursor-pointer"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
      </div>
      <div className="flex items-center gap-1.5">
        <FilterDropdown
          value={contentTypeFilter}
          options={contentTypeOptions}
          onChange={setContentTypeFilter}
          icon={<Filter className="w-3.5 h-3.5" />}
          t={t}
        />
        <button
          onClick={() =>
            setSortOrder(sortOrder === "newest" ? "oldest" : "newest")
          }
          className="flex items-center gap-1 px-2 py-1 text-xs text-text/60 hover:text-text/90 hover:bg-mid-gray/10 rounded-md transition-colors cursor-pointer"
          title={t("settings.clipboard.sortNewest")}
        >
          <ArrowUpDown className="w-3.5 h-3.5" />
          <span>
            {sortOrder === "newest"
              ? t("settings.clipboard.sortNewest")
              : t("settings.clipboard.sortOldest")}
          </span>
        </button>
        <div className="flex-1" />
        <div className="flex items-center border border-mid-gray/20 rounded-md overflow-hidden">
          <button
            onClick={() => setViewMode("list")}
            className={`p-1.5 transition-colors cursor-pointer ${
              viewMode === "list"
                ? "bg-logo-primary/20 text-logo-primary"
                : "text-text/40 hover:text-text/70"
            }`}
            title={t("settings.clipboard.viewList")}
          >
            <LayoutList className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => setViewMode("grid")}
            className={`p-1.5 transition-colors cursor-pointer ${
              viewMode === "grid"
                ? "bg-logo-primary/20 text-logo-primary"
                : "text-text/40 hover:text-text/70"
            }`}
            title={t("settings.clipboard.viewGrid")}
          >
            <LayoutGrid className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
};

interface FilterDropdownProps {
  value: ClipboardContentTypeFilter;
  options: { value: ClipboardContentTypeFilter; labelKey: string }[];
  onChange: (value: ClipboardContentTypeFilter) => void;
  icon: React.ReactNode;
  t: (key: string) => string;
}

const FilterDropdown: React.FC<FilterDropdownProps> = ({
  value,
  options,
  onChange,
  icon,
  t,
}) => {
  const currentLabel = options.find((o) => o.value === value);

  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value as ClipboardContentTypeFilter)}
      className="flex items-center gap-1 px-2 py-1 text-xs text-text/60 bg-transparent border border-mid-gray/20 rounded-md cursor-pointer hover:border-logo-primary/50 focus:outline-none focus:border-logo-primary/50 appearance-none pr-5"
      style={{
        backgroundImage: `url("data:image/svg+xml,%3csvg xmlns='http://www.w3.org/2000/svg' fill='none' viewBox='0 0 20 20'%3e%3cpath stroke='%236b7280' stroke-linecap='round' stroke-linejoin='round' stroke-width='1.5' d='M6 8l4 4 4-4'/%3e%3c/svg%3e")`,
        backgroundPosition: "right 4px center",
        backgroundRepeat: "no-repeat",
        backgroundSize: "16px",
      }}
    >
      {options.map((opt) => (
        <option key={opt.value} value={opt.value}>
          {t(opt.labelKey)}
        </option>
      ))}
    </select>
  );
};
