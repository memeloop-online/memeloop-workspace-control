import type { MessageKey } from "../i18n";

export type SimpleSectionId = "profile" | "appearance" | "organization";
export interface SettingsField {
  key: "display_name" | "avatar_url" | "theme" | "locale" | "organization_id";
  section: SimpleSectionId;
  type: "string" | "string-enum" | "avatar";
  label: MessageKey;
  description?: MessageKey;
  required?: boolean;
  maxLength?: number;
  options?: readonly { value: string; label: MessageKey }[];
}

export const settingsFields: readonly SettingsField[] = [
  { key: "display_name", section: "profile", type: "string", label: "displayName", required: true, maxLength: 80 },
  { key: "avatar_url", section: "profile", type: "avatar", label: "avatarUrl", description: "avatarUrlHelp" },
  { key: "theme", section: "appearance", type: "string-enum", label: "settingsTheme", options: [{ value: "light", label: "themeLight" }, { value: "dark", label: "themeDark" }] },
  { key: "locale", section: "appearance", type: "string-enum", label: "language", options: [{ value: "zh-CN", label: "languageChinese" }, { value: "en", label: "languageEnglish" }, { value: "ru", label: "languageRussian" }] },
  { key: "organization_id", section: "organization", type: "string-enum", label: "currentOrganization", description: "organizationSwitchHelp" },
];

export function matchingFields(section: SimpleSectionId, query: string, t: (key: MessageKey) => string): SettingsField[] {
  const fields = settingsFields.filter((field) => field.section === section);
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return fields;
  return fields.filter((field) => terms.every((term) => [field.key, t(field.label), field.description ? t(field.description) : "", ...(field.options ?? []).flatMap((option) => [option.value, t(option.label)])].join(" ").toLocaleLowerCase().includes(term)));
}
