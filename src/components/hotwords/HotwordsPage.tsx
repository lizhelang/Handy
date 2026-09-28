import { useTranslation } from "react-i18next";
import { CustomWords } from "../settings/CustomWords";

export function HotwordsPage() {
  const { t } = useTranslation();
  return (
    <section
      aria-label={t("sidebar.hotwords")}
      className="w-full min-w-0 max-w-3xl mx-auto space-y-4"
    >
      <h1 className="text-lg font-semibold">{t("sidebar.hotwords")}</h1>
      <CustomWords descriptionMode="inline" />
    </section>
  );
}
