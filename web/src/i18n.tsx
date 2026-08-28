import i18next from "i18next";
import { useCallback, useEffect, useMemo } from "react";
import type { ReactNode } from "react";
import { I18nextProvider, initReactI18next, useTranslation } from "react-i18next";
import en from "./locales/en.json";
import ru from "./locales/ru.json";
import zhCN from "./locales/zh-CN.json";

export type Locale = "zh-CN" | "en" | "ru";
export type MessageKey = keyof typeof zhCN;

const supportedLocales: readonly Locale[] = ["zh-CN", "en", "ru"];

function normalizeLocale(value: string | null | undefined): Locale {
  if (value && supportedLocales.includes(value as Locale)) return value as Locale;
  const normalized = value?.toLowerCase() ?? "";
  if (normalized.startsWith("zh")) return "zh-CN";
  if (normalized.startsWith("ru")) return "ru";
  return "en";
}

const initialLocale = normalizeLocale(
  localStorage.getItem("mwc.locale") || navigator.language,
);

void i18next.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    en: { translation: en },
    ru: { translation: ru },
  },
  lng: initialLocale,
  fallbackLng: "en",
  supportedLngs: supportedLocales,
  load: "currentOnly",
  interpolation: { escapeValue: false },
  initAsync: false,
});

type I18nValue = {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: MessageKey) => string;
};

export function I18nProvider({ children }: { children: ReactNode }) {
  return <I18nextProvider i18n={i18next}>{children}</I18nextProvider>;
}

export function useI18n(): I18nValue {
  const { t: translate, i18n } = useTranslation();
  const locale = normalizeLocale(i18n.resolvedLanguage || i18n.language);
  useEffect(() => {
    localStorage.setItem("mwc.locale", locale);
    document.documentElement.lang = locale;
  }, [locale]);
  const setLocale = useCallback((next: Locale) => {
    void i18n.changeLanguage(next);
  }, [i18n]);
  return useMemo(() => ({
    locale,
    setLocale,
    t: (key: MessageKey) => translate(key),
  }), [locale, setLocale, translate]);
}
