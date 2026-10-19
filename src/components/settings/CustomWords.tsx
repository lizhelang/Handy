import React, { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowDown, ArrowUp, Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";

interface CustomWordsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

type SortMode = "az" | "freq_desc" | "freq_asc" | "default";
type FilterMode = "all" | "used" | "unused";

interface LearnedTermEntry {
  text?: string;
  score?: number;
}

interface PersonalizationStatusPayload {
  learned_terms?: LearnedTermEntry[];
}

const PINYIN_COLLATOR = new Intl.Collator("zh-Hans-u-co-pinyin", {
  sensitivity: "base",
  numeric: true,
});

const PINYIN_BOUNDARIES: [string, string][] = [
  ["A", "吖"],
  ["B", "八"],
  ["C", "嚓"],
  ["D", "哒"],
  ["E", "妸"],
  ["F", "发"],
  ["G", "旮"],
  ["H", "哈"],
  ["J", "丌"],
  ["K", "咔"],
  ["L", "垃"],
  ["M", "妈"],
  ["N", "拏"],
  ["O", "噢"],
  ["P", "妑"],
  ["Q", "七"],
  ["R", "呥"],
  ["S", "仨"],
  ["T", "他"],
  ["W", "屲"],
  ["X", "夕"],
  ["Y", "丫"],
  ["Z", "帀"],
];

const getInitialLetter = (word: string): string => {
  const trimmed = word.trim();
  if (!trimmed) return "#";
  const first = trimmed[0];
  if (/[a-zA-Z]/.test(first)) {
    return first.toUpperCase();
  }
  if (/[\u4e00-\u9fa5]/.test(first)) {
    for (let i = PINYIN_BOUNDARIES.length - 1; i >= 0; i--) {
      if (PINYIN_COLLATOR.compare(first, PINYIN_BOUNDARIES[i][1]) >= 0) {
        return PINYIN_BOUNDARIES[i][0];
      }
    }
    return "A";
  }
  return "#";
};

const normalizeCustomWord = (word: string) =>
  word
    .replace(/[<>"']/g, "")
    .replace(/\s+/g, " ")
    .trim();

const countSubstringOccurrences = (
  haystack: string,
  needle: string,
): number => {
  if (!haystack || !needle) return 0;
  let count = 0;
  let pos = 0;
  while (true) {
    const idx = haystack.indexOf(needle, pos);
    if (idx === -1) break;
    count += 1;
    pos = idx + needle.length;
  }
  return count;
};

export const CustomWords: React.FC<CustomWordsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [newWord, setNewWord] = useState("");
    const [searchQuery, setSearchQuery] = useState("");
    const [sortMode, setSortMode] = useState<SortMode>("freq_desc");
    const [filterMode, setFilterMode] = useState<FilterMode>("all");
    const [activeLetter, setActiveLetter] = useState<string | null>(null);
    const [historyTexts, setHistoryTexts] = useState<string[]>([]);
    const [learnedScores, setLearnedScores] = useState<Record<string, number>>(
      {},
    );

    const customWords = getSetting("custom_words") || [];
    const normalizedWord = normalizeCustomWord(newWord);

    useEffect(() => {
      let cancelled = false;
      const loadUsageSignals = async () => {
        try {
          const historyRes = await commands.getUnifiedHistory({
            search: null,
            source_kind: null,
            content_type: "text",
            starred_only: false,
            limit: 300,
            offset: 0,
          });
          if (
            !cancelled &&
            historyRes &&
            typeof historyRes === "object" &&
            "status" in historyRes &&
            historyRes.status === "ok" &&
            Array.isArray(historyRes.data)
          ) {
            setHistoryTexts(
              historyRes.data
                .map((item) => (item.text || "").toLowerCase())
                .filter(Boolean),
            );
          }
        } catch {
          // Ignore when running in unit/playwright mocks without unified history
        }

        try {
          const status = await invoke<PersonalizationStatusPayload | null>(
            "knowledge_request",
            {
              action: "personalization_status",
              payload: {},
            },
          );
          if (
            !cancelled &&
            status &&
            typeof status === "object" &&
            Array.isArray(status.learned_terms)
          ) {
            const map: Record<string, number> = {};
            for (const entry of status.learned_terms) {
              const key = (entry.text || "").trim().toLowerCase();
              if (key) {
                map[key] = Math.max(
                  map[key] || 0,
                  Math.round(Number(entry.score) || 0),
                );
              }
            }
            setLearnedScores(map);
          }
        } catch {
          // Ignore when personalization_status is unavailable
        }
      };

      void loadUsageSignals();
      return () => {
        cancelled = true;
      };
    }, [customWords.length]);

    const usageMap = useMemo(() => {
      const counts: Record<string, number> = {};
      for (const word of customWords) {
        const lower = word.toLowerCase();
        let total = learnedScores[lower] || 0;
        for (const text of historyTexts) {
          total += countSubstringOccurrences(text, lower);
        }
        counts[word] = total;
      }
      return counts;
    }, [customWords, historyTexts, learnedScores]);

    const exactExistingWord = useMemo(() => {
      if (!normalizedWord) return null;
      const target = normalizedWord.toLowerCase();
      return customWords.find((word) => word.toLowerCase() === target) ?? null;
    }, [customWords, normalizedWord]);

    const similarExistingWords = useMemo(() => {
      if (!normalizedWord) return [];
      const target = normalizedWord.toLowerCase();
      return customWords
        .filter((word) => {
          const lower = word.toLowerCase();
          return (
            lower !== target &&
            (lower.includes(target) || target.includes(lower))
          );
        })
        .slice(0, 6);
    }, [customWords, normalizedWord]);

    const usedCount = useMemo(
      () => customWords.filter((word) => (usageMap[word] || 0) > 0).length,
      [customWords, usageMap],
    );
    const unusedCount = customWords.length - usedCount;

    const filteredAndSortedWords = useMemo(() => {
      const query = searchQuery.trim().toLowerCase();
      const filtered = customWords.filter((word) => {
        const count = usageMap[word] || 0;
        if (filterMode === "used" && count === 0) return false;
        if (filterMode === "unused" && count > 0) return false;
        if (activeLetter && getInitialLetter(word) !== activeLetter) {
          return false;
        }
        if (query) {
          const lower = word.toLowerCase();
          const initial = getInitialLetter(word).toLowerCase();
          return lower.includes(query) || initial === query;
        }
        return true;
      });

      const indexed = filtered.map((word, index) => ({
        word,
        index,
        count: usageMap[word] || 0,
        letter: getInitialLetter(word),
      }));

      indexed.sort((a, b) => {
        if (sortMode === "freq_desc") {
          if (b.count !== a.count) return b.count - a.count;
          return PINYIN_COLLATOR.compare(a.word, b.word);
        }
        if (sortMode === "freq_asc") {
          if (a.count !== b.count) return a.count - b.count;
          return PINYIN_COLLATOR.compare(a.word, b.word);
        }
        if (sortMode === "az") {
          if (a.letter !== b.letter) {
            if (a.letter === "#") return 1;
            if (b.letter === "#") return -1;
            return a.letter.localeCompare(b.letter);
          }
          return PINYIN_COLLATOR.compare(a.word, b.word);
        }
        return a.index - b.index;
      });

      return indexed.map((item) => item.word);
    }, [
      activeLetter,
      customWords,
      filterMode,
      searchQuery,
      sortMode,
      usageMap,
    ]);

    const availableLetters = useMemo(() => {
      const set = new Set<string>();
      for (const word of customWords) {
        set.add(getInitialLetter(word));
      }
      return Array.from(set).sort((a, b) => {
        if (a === "#") return 1;
        if (b === "#") return -1;
        return a.localeCompare(b);
      });
    }, [customWords]);

    const groupedByLetter = useMemo(() => {
      const groups: { letter: string; words: string[] }[] = [];
      for (const word of filteredAndSortedWords) {
        const letter = getInitialLetter(word);
        const last = groups[groups.length - 1];
        if (!last || last.letter !== letter) {
          groups.push({ letter, words: [word] });
        } else {
          last.words.push(word);
        }
      }
      return groups;
    }, [filteredAndSortedWords]);

    const handleAddWord = () => {
      if (
        !isUpdating("custom_words") &&
        normalizedWord &&
        normalizedWord.length <= 50
      ) {
        if (customWords.includes(normalizedWord) || exactExistingWord) {
          toast.error(
            t("settings.advanced.customWords.duplicate", {
              word: exactExistingWord || normalizedWord,
            }),
          );
          return;
        }
        updateSetting("custom_words", [...customWords, normalizedWord]);
        setNewWord("");
      }
    };

    const handleRemoveWord = (wordToRemove: string) => {
      updateSetting(
        "custom_words",
        customWords.filter((word) => word !== wordToRemove),
      );
    };

    const handleKeyPress = (e: React.KeyboardEvent) => {
      if (e.nativeEvent.isComposing || e.keyCode === 229) return;
      if (e.key === "Enter") {
        e.preventDefault();
        handleAddWord();
      }
    };

    const renderWordPill = (word: string) => {
      const count = usageMap[word] || 0;
      const isExactInputMatch =
        exactExistingWord !== null && exactExistingWord === word;
      const isSimilarInputMatch = similarExistingWords.includes(word);

      return (
        <Button
          key={word}
          onClick={() => handleRemoveWord(word)}
          disabled={isUpdating("custom_words")}
          variant="secondary"
          size="sm"
          className={`inline-flex max-w-full items-center gap-1.5 cursor-pointer transition-colors ${
            isExactInputMatch
              ? "ring-2 ring-amber-500/70 bg-amber-500/10"
              : isSimilarInputMatch
                ? "ring-1 ring-logo-primary/60 bg-logo-primary/5"
                : ""
          }`}
          aria-label={t("settings.advanced.customWords.remove", { word })}
        >
          <span className="min-w-0 break-words font-medium [overflow-wrap:anywhere]">
            {word}
          </span>
          <span
            className={`inline-flex items-center rounded px-1.5 py-0.5 text-[11px] leading-none ${
              count > 0
                ? "bg-logo-primary/15 text-logo-primary font-semibold"
                : "bg-mid-gray/15 text-text/45"
            }`}
          >
            {t("settings.advanced.customWords.usageCount", { count })}
          </span>
          <svg
            className="w-3 h-3 shrink-0 opacity-60 hover:opacity-100"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </Button>
      );
    };

    return (
      <div className="space-y-3">
        <SettingContainer
          title={t("settings.advanced.customWords.title")}
          description={t("settings.advanced.customWords.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
          layout="stacked"
        >
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Input
                type="text"
                className="min-w-0 flex-1"
                aria-label={t("settings.advanced.customWords.placeholder")}
                value={newWord}
                onChange={(e) => setNewWord(e.target.value)}
                onKeyDown={handleKeyPress}
                placeholder={t("settings.advanced.customWords.placeholder")}
                variant="compact"
                disabled={isUpdating("custom_words")}
              />
              <Button
                onClick={handleAddWord}
                disabled={
                  !normalizedWord ||
                  normalizedWord.length > 50 ||
                  isUpdating("custom_words")
                }
                variant="primary"
                size="md"
              >
                {t("settings.advanced.customWords.add")}
              </Button>
            </div>

            {exactExistingWord && (
              <p className="text-xs text-amber-500 font-medium">
                {t("settings.advanced.customWords.exactMatchHint", {
                  word: exactExistingWord,
                  count: usageMap[exactExistingWord] || 0,
                })}
              </p>
            )}

            {!exactExistingWord && similarExistingWords.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5 text-xs text-text/65">
                <span>
                  {t("settings.advanced.customWords.similarMatchHint")}
                </span>
                {similarExistingWords.map((word) => (
                  <span
                    key={word}
                    className="rounded bg-logo-primary/10 px-1.5 py-0.5 text-logo-primary font-medium"
                  >
                    {word}
                  </span>
                ))}
              </div>
            )}
          </div>
        </SettingContainer>

        {customWords.length === 0 ? (
          <div className="rounded-lg border border-mid-gray/20 px-4 py-6 text-center text-sm text-text/55">
            {t("settings.advanced.customWords.emptyHint")}
          </div>
        ) : (
          <div
            className={`p-3.5 space-y-3 ${
              grouped ? "" : "rounded-lg border border-mid-gray/20"
            }`}
          >
            {/* 顶部统计筛选 + 排序切换栏 */}
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-mid-gray/15 pb-2.5">
              <div className="flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={() => setFilterMode("all")}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    filterMode === "all"
                      ? "bg-logo-primary text-white"
                      : "bg-mid-gray/10 text-text/75 hover:bg-mid-gray/20"
                  }`}
                >
                  {t("settings.advanced.customWords.statsAll", {
                    count: customWords.length,
                  })}
                </button>
                <button
                  type="button"
                  onClick={() => setFilterMode("used")}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    filterMode === "used"
                      ? "bg-logo-primary text-white"
                      : "bg-mid-gray/10 text-text/75 hover:bg-mid-gray/20"
                  }`}
                >
                  {t("settings.advanced.customWords.statsUsed", {
                    count: usedCount,
                  })}
                </button>
                <button
                  type="button"
                  onClick={() => setFilterMode("unused")}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    filterMode === "unused"
                      ? "bg-logo-primary text-white"
                      : "bg-mid-gray/10 text-text/75 hover:bg-mid-gray/20"
                  }`}
                >
                  {t("settings.advanced.customWords.statsUnused", {
                    count: unusedCount,
                  })}
                </button>
              </div>

              <div className="flex flex-wrap items-center gap-1">
                <button
                  type="button"
                  onClick={() =>
                    setSortMode((prev) =>
                      prev === "freq_desc" ? "freq_asc" : "freq_desc",
                    )
                  }
                  className={`inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-xs transition-colors ${
                    sortMode === "freq_desc" || sortMode === "freq_asc"
                      ? "bg-text/15 font-semibold text-text"
                      : "text-text/65 hover:bg-text/5"
                  }`}
                >
                  <span>{t("settings.advanced.customWords.sortFreq")}</span>
                  {sortMode === "freq_asc" ? (
                    <ArrowUp className="h-3.5 w-3.5" />
                  ) : (
                    <ArrowDown className="h-3.5 w-3.5" />
                  )}
                </button>
                <button
                  type="button"
                  onClick={() => setSortMode("az")}
                  className={`rounded-md px-2.5 py-1 text-xs transition-colors ${
                    sortMode === "az"
                      ? "bg-text/15 font-semibold text-text"
                      : "text-text/65 hover:bg-text/5"
                  }`}
                >
                  {t("settings.advanced.customWords.sortAz")}
                </button>
                <button
                  type="button"
                  onClick={() => setSortMode("default")}
                  className={`rounded-md px-2.5 py-1 text-xs transition-colors ${
                    sortMode === "default"
                      ? "bg-text/15 font-semibold text-text"
                      : "text-text/65 hover:bg-text/5"
                  }`}
                >
                  {t("settings.advanced.customWords.sortDefault")}
                </button>
              </div>
            </div>

            {/* 搜索过滤框 + A-Z 首字母快速导航 */}
            <div className="flex flex-wrap items-center gap-2">
              <div className="relative w-48 shrink-0">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text/45" />
                <input
                  type="search"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder={t(
                    "settings.advanced.customWords.searchPlaceholder",
                  )}
                  aria-label={t(
                    "settings.advanced.customWords.searchPlaceholder",
                  )}
                  className="w-full rounded-md border border-mid-gray/25 bg-transparent py-1.5 pl-8 pr-7 text-xs focus:outline-none focus:ring-1 focus:ring-logo-primary"
                />
                {searchQuery && (
                  <button
                    type="button"
                    onClick={() => setSearchQuery("")}
                    aria-label={t("settings.advanced.customWords.clearSearch")}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-text/45 hover:text-text"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>

              {availableLetters.length > 1 && (
                <div className="flex flex-wrap items-center gap-1">
                  {availableLetters.map((letter) => (
                    <button
                      key={letter}
                      type="button"
                      onClick={() =>
                        setActiveLetter((prev) =>
                          prev === letter ? null : letter,
                        )
                      }
                      className={`h-6 min-w-6 rounded px-1.5 text-xs font-semibold transition-colors ${
                        activeLetter === letter
                          ? "bg-logo-primary text-white"
                          : "bg-mid-gray/10 text-text/70 hover:bg-mid-gray/20"
                      }`}
                    >
                      {letter}
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* 热词列表主体 */}
            {filteredAndSortedWords.length === 0 ? (
              <div className="py-6 text-center text-xs text-text/50">
                {t("settings.advanced.customWords.noFilterMatch")}
              </div>
            ) : sortMode === "az" ? (
              <div className="space-y-2.5">
                {groupedByLetter.map((group) => (
                  <div
                    key={group.letter}
                    className="flex items-start gap-2.5 border-b border-mid-gray/10 pb-2.5 last:border-b-0 last:pb-0"
                  >
                    <span className="mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded bg-logo-primary/10 text-xs font-bold text-logo-primary">
                      {group.letter}
                    </span>
                    <div className="flex flex-1 flex-wrap gap-1.5">
                      {group.words.map((word) => renderWordPill(word))}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {filteredAndSortedWords.map((word) => renderWordPill(word))}
              </div>
            )}
          </div>
        )}
      </div>
    );
  },
);
