import type { ApiKeySummary } from "../types";
import type { MessageKey } from "../i18n";
import { API_KEY_SCOPES } from "../apiKeyScopes";

interface Props {
  keys: readonly ApiKeySummary[];
  locale: string;
  onRevoke: (key: ApiKeySummary) => void;
  translate: (key: MessageKey) => string;
}

export function ApiKeyList({ keys, locale, onRevoke, translate }: Props) {
  if (keys.length === 0) return <p className="api-key-empty">{translate("noApiKeys")}</p>;

  return <div className="api-key-list" aria-label={translate("apiKeys")}>
    {keys.map((key) => <article className="api-key-list-item" key={key.id}>
      <div className="api-key-list-identity">
        <strong title={key.name}>{key.name}</strong>
        <code>{key.prefix}</code>
        <span>{formatScopes(key, translate) ?? translate("apiKeyScopesUnavailable")}</span>
      </div>
      <dl className="api-key-facts">
        <div><dt>{translate("createdAt")}</dt><dd>{formatTime(key.created_at, locale)}</dd></div>
        <div><dt>{translate("lastUsedAt")}</dt><dd>{key.last_used_at ? formatTime(key.last_used_at, locale) : translate("never")}</dd></div>
        <div><dt>{translate("apiKeyExpires")}</dt><dd>{formatExpiry(key, locale, translate)}</dd></div>
        <div><dt>{translate("apiKeyTemplates")}</dt><dd>{formatTemplateRestriction(key, translate)}</dd></div>
      </dl>
      <button className="button danger api-key-revoke" type="button" onClick={() => onRevoke(key)}>{translate("revokeApiKey")}</button>
    </article>)}
  </div>;
}

function formatTemplateRestriction(key: ApiKeySummary, translate: (key: MessageKey) => string): string {
  if (key.allowed_template_ids === null) return translate("allTemplates");
  if (key.allowed_template_ids.length === 0) return translate("noTemplates");
  return key.allowed_template_ids.join(" · ");
}

function formatTime(value: number, locale: string) {
  return new Date(value * 1_000).toLocaleString(locale);
}

function formatScopes(key: ApiKeySummary, translate: (key: MessageKey) => string): string | null {
  if (!Array.isArray(key.scopes) || key.scopes.length === 0) return null;
  const labels = key.scopes.map((scope) => {
    const definition = API_KEY_SCOPES.find((item) => item.scope === scope);
    return definition ? translate(definition.label) : translate("scopeUnknown");
  });
  return labels.join(" · ");
}

function formatExpiry(key: ApiKeySummary, locale: string, translate: (key: MessageKey) => string): string {
  if (typeof key.expires_at === "number") return formatTime(key.expires_at, locale);
  if (key.expires_at === null) return translate("never");
  return translate("apiKeyExpiryUnavailable");
}
