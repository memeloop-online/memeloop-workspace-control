import { API_KEY_SCOPE_LABELS } from "./apiKeyScopes";
import { getApiKeyStatus } from "./apiKeyStatus";
import type { MessageKey } from "./i18n";
import type { ApiKeySummary } from "./types";

type Translate = (key: MessageKey) => string;

export function formatApiKeyStatus(key: ApiKeySummary, t: Translate): string {
  switch (getApiKeyStatus(key)) {
    case "revoked": return t("apiKeyRevoked");
    case "expired": return t("apiKeyExpired");
    case "active": return t("apiKeyActive");
  }
}

export function formatApiKeyPageStatus(
  locale: string,
  pageNumber: number,
  count: number,
  t: Translate,
): string {
  const countUnit = locale === "ru" ? t(russianApiKeyCountUnit(count)) : t("apiKeyCountUnit");
  return `${t("apiKeyPageStatus")} ${pageNumber}${t("apiKeyPageUnit")} · ${count} ${countUnit}`;
}

export function formatApiKeyScopes(key: ApiKeySummary, t: Translate): string {
  if (key.scopes.length === 0) return t("scopeUnknown");
  return key.scopes.map((scope) => {
    const label = API_KEY_SCOPE_LABELS[scope];
    return label ? t(label) : t("scopeUnknown");
  }).join(" · ");
}

export function formatTime(value: number, locale: string): string {
  return new Date(value * 1_000).toLocaleString(locale);
}

export function formatApiKeyExpiry(value: number | null, locale: string, t: Translate): string {
  if (value === null) return t("never");
  const formatted = formatTime(value, locale);
  return value <= Math.floor(Date.now() / 1_000)
    ? `${formatted} · ${t("apiKeyExpired")}`
    : formatted;
}

function russianApiKeyCountUnit(count: number): MessageKey {
  const lastTwoDigits = count % 100;
  const lastDigit = count % 10;
  if (lastDigit === 1 && lastTwoDigits !== 11) return "apiKeyCountUnitOne";
  if (lastDigit >= 2 && lastDigit <= 4 && (lastTwoDigits < 12 || lastTwoDigits > 14)) {
    return "apiKeyCountUnitFew";
  }
  return "apiKeyCountUnit";
}
