import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { API_KEY_SCOPES, API_KEY_SCOPE_LABELS } from "./apiKeyScopes";
import { useI18n, type MessageKey } from "./i18n";
import type { ApiKeyScope, ApiKeySummary, CreatedApiKey, Organization, Principal, UserProfile } from "./types";
import { UserAvatar } from "./UserAvatar";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizations: Organization[];
  organizationId: string;
  onOrganizationChange: (organizationId: string) => void;
  onProfileChanged: (profile: UserProfile) => void;
  onError: (message: string) => void;
}

export function SettingsPanel({ api, principal, organizations, organizationId, onOrganizationChange, onProfileChanged, onError }: Props) {
  const { locale, t } = useI18n();
  const [profile, setProfile] = useState<UserProfile>({ display_name: principal.display_name, avatar_url: principal.avatar_url ?? null });
  const [avatarDraft, setAvatarDraft] = useState(customAvatarUrl(principal.avatar_url));
  const [keys, setKeys] = useState<ApiKeySummary[]>([]);
  const [keyName, setKeyName] = useState("");
  const [keyScopes, setKeyScopes] = useState<ApiKeyScope[]>(["read_workspace"]);
  const [keyExpires, setKeyExpires] = useState(() => defaultExpiry());
  const [createdKey, setCreatedKey] = useState<CreatedApiKey | null>(null);
  const [apiKeyAccess, setApiKeyAccess] = useState<"loading" | "available" | "unavailable">("loading");
  const [saving, setSaving] = useState(false);
  const [profileSaved, setProfileSaved] = useState(false);
  const [creatingKey, setCreatingKey] = useState(false);
  const grantableScopes = useMemo(
    () => principal.api_key_scopes.includes("*")
      ? API_KEY_SCOPES
      : API_KEY_SCOPES.filter(({ scope }) => principal.api_key_scopes.includes(scope)),
    [principal.api_key_scopes],
  );

  useEffect(() => {
    setKeyScopes((current) => {
      const allowed = current.filter((scope) => grantableScopes.some((item) => item.scope === scope));
      return allowed.length > 0 ? allowed : grantableScopes[0] ? [grantableScopes[0].scope] : [];
    });
  }, [grantableScopes]);

  useEffect(() => {
    let active = true;
    setApiKeyAccess("loading");
    setKeys([]);
    setCreatedKey(null);

    // Profile access is independent of API-key management. In particular, a
    // read-only token may receive 403 for the key endpoint but must still be
    // able to load and edit its own profile.
    void api.profile()
      .then((nextProfile) => {
        if (!active) return;
        setProfile(nextProfile);
        setAvatarDraft(customAvatarUrl(nextProfile.avatar_url));
        onProfileChanged(nextProfile);
      })
      .catch((error) => {
        if (active) onError(message(error, t("requestFailed")));
      });

    void api.apiKeys()
      .then((nextKeys) => {
        if (!active) return;
        setKeys(nextKeys);
        setApiKeyAccess("available");
      })
      .catch((error) => {
        if (!active) return;
        if (isForbidden(error)) {
          setApiKeyAccess("unavailable");
          return;
        }
        onError(message(error, t("requestFailed")));
        setApiKeyAccess("available");
      });
    return () => { active = false; };
  }, [api]);

  async function saveProfile(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setProfileSaved(false);
    try {
      const saved = await api.updateProfile({ display_name: profile.display_name.trim(), avatar_url: avatarDraft || null });
      setProfile(saved);
      setAvatarDraft(customAvatarUrl(saved.avatar_url));
      onProfileChanged(saved);
      setProfileSaved(true);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function createKey(event: FormEvent) {
    event.preventDefault();
    if (!keyName.trim()) return;
    setCreatingKey(true);
    try {
      // Kept structurally compatible while the API client evolves its typed input.
      const created = await api.createApiKey({ name: keyName.trim(), scopes: keyScopes, expires_at: Math.floor(new Date(keyExpires).getTime() / 1000) });
      setCreatedKey(created);
      setKeyName("");
      setKeyScopes(grantableScopes[0] ? [grantableScopes[0].scope] : []);
      setKeyExpires(defaultExpiry());
      setKeys(await api.apiKeys());
    } catch (error) {
      if (isForbidden(error)) {
        setApiKeyAccess("unavailable");
        return;
      }
      onError(message(error, t("requestFailed")));
    } finally {
      setCreatingKey(false);
    }
  }

  async function revokeKey(key: ApiKeySummary) {
    if (!confirm(`${t("revokeApiKeyConfirm")} ${key.name}?`)) return;
    try {
      await api.deleteApiKey(key.id);
      setKeys((current) => current.filter((item) => item.id !== key.id));
      if (createdKey?.id === key.id) setCreatedKey(null);
    } catch (error) {
      if (isForbidden(error)) {
        setApiKeyAccess("unavailable");
        return;
      }
      onError(message(error, t("requestFailed")));
    }
  }

  return <section className="panel-stack settings-page">
    <div className="section-heading"><div><p className="eyebrow">SETTINGS</p><h2>{t("settingsTitle")}</h2></div></div>
    <div className="settings-grid">
      <section className="settings-card">
        <div className="settings-card-heading"><UserAvatar displayName={profile.display_name} userId={principal.user_id} avatarUrl={profile.avatar_url} size="large" /><div><h3>{t("profileSettings")}</h3><p>{t("profileSettingsHelp")}</p></div></div>
        <form className="settings-form" onSubmit={saveProfile}>
          <label>{t("displayName")}<input required minLength={1} maxLength={80} value={profile.display_name} onChange={(event) => { setProfileSaved(false); setProfile({ ...profile, display_name: event.target.value }); }} /></label>
          <div className="avatar-editor"><UserAvatar displayName={profile.display_name} userId={principal.user_id} avatarUrl={avatarDraft || null} size="large" onChange={(value) => { setProfileSaved(false); setAvatarDraft(value ?? ""); setProfile({ ...profile, avatar_url: value }); }} disabled={saving} /></div>
          <button className="button primary" disabled={saving || !profile.display_name.trim()}>{saving ? t("saving") : t("saveProfile")}</button>
          {profileSaved && <p className="success-inline" role="status">{t("profileSaved")}</p>}
        </form>
      </section>

      <section className="settings-card">
        <h3>{t("organizationSettings")}</h3>
        <p>{t("organizationSwitchHelp")}</p>
        <label className="settings-form">{t("currentOrganization")}<select value={organizationId} onChange={(event) => { if (event.target.value) onOrganizationChange(event.target.value); }}><option value="" disabled>{t("chooseOrganization")}</option>{organizations.map((organization) => <option key={organization.id} value={organization.id}>{organization.name}</option>)}</select></label>
      </section>

      <section className="settings-card wide">
        <div className="card-heading"><div><h3>{t("apiKeys")}</h3><p>{t("apiKeysHelp")}</p></div></div>
        {apiKeyAccess === "unavailable" ? <p className="settings-unavailable" role="status">{t("apiKeysUnavailable")}</p> : apiKeyAccess === "loading" ? <p role="status">{t("loading")}</p> : <>
          {createdKey && <div className="new-api-key" role="status"><strong>{t("apiKeyCreated")}</strong><p>{t("apiKeyCreatedHelp")}</p><div><code>{createdKey.token}</code><button className="button" onClick={() => void navigator.clipboard.writeText(createdKey.token)}>{t("copy")}</button></div><button className="text-button" onClick={() => setCreatedKey(null)}>{t("hideApiKey")}</button></div>}
          <form className="api-key-create" onSubmit={createKey}><label>{t("apiKeyName")}<input maxLength={80} value={keyName} onChange={(event) => setKeyName(event.target.value)} placeholder={t("apiKeyNamePlaceholder")} /></label><fieldset><legend>{t("apiKeyPermissions")}</legend>{grantableScopes.map(({ scope, label }) => <label key={scope}><input type="checkbox" checked={keyScopes.includes(scope)} onChange={() => setKeyScopes((current) => current.includes(scope) ? current.filter((item) => item !== scope) : [...current, scope])} />{t(label)}</label>)}</fieldset><label>{t("apiKeyExpires")}<input required type="datetime-local" min={localDateTime(new Date())} max={localDateTime(new Date(Date.now() + 365 * 86_400_000))} value={keyExpires} onChange={(event) => setKeyExpires(event.target.value)} /></label><button className="button primary" disabled={creatingKey || !keyName.trim() || !keyExpires || keyScopes.length === 0}>{creatingKey ? t("saving") : t("createApiKey")}</button></form>
          {keys.length === 0 ? <p>{t("noApiKeys")}</p> : <div className="api-key-list">{keys.map((key) => <article key={key.id}><div><strong>{key.name}</strong><code>{key.prefix}</code><small>{formatScopes(key, t) ?? t("apiKeyScopesUnavailable")}</small></div><dl><dt>{t("createdAt")}</dt><dd>{formatTime(key.created_at, locale)}</dd><dt>{t("lastUsedAt")}</dt><dd>{key.last_used_at ? formatTime(key.last_used_at, locale) : t("never")}</dd><dt>{t("apiKeyExpires")}</dt><dd>{formatExpiry(key, locale, t)}</dd></dl><button className="button danger" onClick={() => void revokeKey(key)}>{t("revokeApiKey")}</button></article>)}</div>}
        </>}
      </section>
    </div>
  </section>;
}

function formatTime(value: number, locale: string) {
  return new Date(value * 1_000).toLocaleString(locale);
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function isForbidden(error: unknown): boolean {
  if (!error) return false;
  const status = typeof error === "object" && error !== null && "status" in error ? (error as { status?: unknown }).status : undefined;
  if (status === 403) return true;
  if (!(error instanceof Error)) return false;
  return /\b403\b|forbidden|not allowed/i.test(error.message);
}

function formatScopes(key: ApiKeySummary, t: (key: MessageKey) => string): string | null {
  const scopes = (key as ApiKeySummary & { scopes?: unknown }).scopes;
  if (!Array.isArray(scopes) || scopes.length === 0) return null;
  const labels = scopes.map((scope) => {
    if (typeof scope !== "string") return null;
    return API_KEY_SCOPE_LABELS[scope] ? t(API_KEY_SCOPE_LABELS[scope]) : t("scopeUnknown");
  }).filter((label): label is string => Boolean(label));
  return labels.length > 0 ? labels.join(" · ") : null;
}

function formatExpiry(key: ApiKeySummary, locale: string, t: (key: MessageKey) => string): string {
  const expiresAt = (key as ApiKeySummary & { expires_at?: unknown }).expires_at;
  if (typeof expiresAt === "number") return formatTime(expiresAt, locale);
  if (expiresAt === null) return t("never");
  return t("apiKeyExpiryUnavailable");
}

function customAvatarUrl(value: string | null | undefined): string {
  // Profile avatars are deliberately local data URLs; never put an arbitrary
  // network URL back into the editor or send it back to the API.
  return value && /^(data:image\/(png|jpeg|webp);base64,)/i.test(value) ? value : "";
}

function localDateTime(value: Date): string {
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}

function defaultExpiry(): string {
  return localDateTime(new Date(Date.now() + 30 * 86_400_000));
}
