import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type { ApiKeySummary, CreatedApiKey, Organization, Principal, UserProfile } from "./types";
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
  const [createdKey, setCreatedKey] = useState<CreatedApiKey | null>(null);
  const [saving, setSaving] = useState(false);
  const [profileSaved, setProfileSaved] = useState(false);
  const [creatingKey, setCreatingKey] = useState(false);

  useEffect(() => {
    let active = true;
    Promise.all([api.profile(), api.apiKeys()])
      .then(([nextProfile, nextKeys]) => {
        if (!active) return;
        setProfile(nextProfile);
        setAvatarDraft(customAvatarUrl(nextProfile.avatar_url));
        setKeys(nextKeys);
        onProfileChanged(nextProfile);
      })
      .catch((error) => onError(message(error, t("requestFailed"))));
    return () => { active = false; };
  }, [api]);

  async function saveProfile(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setProfileSaved(false);
    try {
      const saved = await api.updateProfile({ display_name: profile.display_name.trim(), avatar_url: avatarDraft.trim() || null });
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
      const created = await api.createApiKey(keyName.trim());
      setCreatedKey(created);
      setKeyName("");
      setKeys(await api.apiKeys());
    } catch (error) {
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
      onError(message(error, t("requestFailed")));
    }
  }

  return <section className="panel-stack settings-page">
    <div className="section-heading"><div><p className="eyebrow">SETTINGS</p><h2>{t("settingsTitle")}</h2></div></div>
    <div className="settings-grid">
      <section className="settings-card">
        <div className="settings-card-heading"><UserAvatar displayName={profile.display_name} userId={principal.user_id} avatarUrl={profile.avatar_url} size="large" /><div><h3>{t("profileSettings")}</h3><p>{t("profileSettingsHelp")}</p></div></div>
        <form className="settings-form" onSubmit={saveProfile}>
          <label>{t("displayName")}<input required minLength={1} maxLength={100} value={profile.display_name} onChange={(event) => { setProfileSaved(false); setProfile({ ...profile, display_name: event.target.value }); }} /></label>
          <label>{t("avatarUrl")}<input type="url" inputMode="url" value={avatarDraft} onChange={(event) => { setProfileSaved(false); setAvatarDraft(event.target.value); setProfile({ ...profile, avatar_url: event.target.value || null }); }} placeholder="https://…" /></label>
          <p className="field-help">{t("avatarUrlHelp")}</p>
          <button className="button primary" disabled={saving || !profile.display_name.trim()}>{saving ? t("saving") : t("saveProfile")}</button>
          {profileSaved && <p className="success-inline" role="status">{t("profileSaved")}</p>}
        </form>
      </section>

      <section className="settings-card">
        <h3>{t("organizationSettings")}</h3>
        <p>{t("organizationSwitchHelp")}</p>
        <label className="settings-form">{t("currentOrganization")}<select value={organizationId} onChange={(event) => onOrganizationChange(event.target.value)}><option value="">{t("chooseOrganization")}</option>{organizations.map((organization) => <option key={organization.id} value={organization.id}>{organization.name}</option>)}</select></label>
      </section>

      <section className="settings-card wide">
        <div className="card-heading"><div><h3>{t("apiKeys")}</h3><p>{t("apiKeysHelp")}</p></div></div>
        {createdKey && <div className="new-api-key" role="status"><strong>{t("apiKeyCreated")}</strong><p>{t("apiKeyCreatedHelp")}</p><div><code>{createdKey.token}</code><button className="button" onClick={() => void navigator.clipboard.writeText(createdKey.token)}>{t("copy")}</button></div><button className="text-button" onClick={() => setCreatedKey(null)}>{t("hideApiKey")}</button></div>}
        <form className="api-key-create" onSubmit={createKey}><label>{t("apiKeyName")}<input maxLength={80} value={keyName} onChange={(event) => setKeyName(event.target.value)} placeholder={t("apiKeyNamePlaceholder")} /></label><button className="button primary" disabled={creatingKey || !keyName.trim()}>{creatingKey ? t("saving") : t("createApiKey")}</button></form>
        {keys.length === 0 ? <p>{t("noApiKeys")}</p> : <div className="api-key-list">{keys.map((key) => <article key={key.id}><div><strong>{key.name}</strong><code>{key.prefix}</code></div><dl><dt>{t("createdAt")}</dt><dd>{formatTime(key.created_at, locale)}</dd><dt>{t("lastUsedAt")}</dt><dd>{key.last_used_at ? formatTime(key.last_used_at, locale) : t("never")}</dd></dl><button className="button danger" onClick={() => void revokeKey(key)}>{t("revokeApiKey")}</button></article>)}</div>}
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

function customAvatarUrl(value: string | null | undefined): string {
  return value?.startsWith("data:image/svg+xml") ? "" : value ?? "";
}
