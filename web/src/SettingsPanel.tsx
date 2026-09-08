import { useEffect, useState } from "react";
import type { FormEvent } from "react";

import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import { ApiKeySection } from "./settings/ApiKeySection";
import type { Organization, Principal, UserProfile } from "./types";
import { UserAvatar } from "./UserAvatar";
import "./settings-ui.css";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizations: Organization[];
  organizationId: string;
  onOrganizationChange: (organizationId: string) => void;
  onProfileChanged: (profile: UserProfile) => void;
  onError: (message: string) => void;
}

export function SettingsPanel({
  api,
  principal,
  organizations,
  organizationId,
  onOrganizationChange,
  onProfileChanged,
  onError,
}: Props) {
  const { t } = useI18n();
  const [profile, setProfile] = useState<UserProfile>({
    display_name: principal.display_name,
    avatar_url: principal.avatar_url ?? null,
  });
  const [avatarDraft, setAvatarDraft] = useState(customAvatarUrl(principal.avatar_url));
  const [saving, setSaving] = useState(false);
  const [profileSaved, setProfileSaved] = useState(false);

  useEffect(() => {
    let active = true;
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
    return () => { active = false; };
    // The shell currently supplies onProfileChanged inline. Profile loading is
    // scoped to an authenticated API client, not to that render callback.
    // Keeping this dependency narrow prevents a profile request on every
    // parent render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api]);

  async function saveProfile(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setProfileSaved(false);
    try {
      const saved = await api.updateProfile({
        display_name: profile.display_name.trim(),
        avatar_url: avatarDraft || null,
      });
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

  return <section className="panel-stack settings-page">
    <div className="section-heading"><div><p className="eyebrow">SETTINGS</p><h2>{t("settingsTitle")}</h2></div></div>
    <div className="settings-grid settings-grid-refined">
      <ProfileCard
        avatarDraft={avatarDraft}
        principal={principal}
        profile={profile}
        profileSaved={profileSaved}
        saving={saving}
        onAvatarChange={(value) => {
          setProfileSaved(false);
          setAvatarDraft(value ?? "");
          setProfile((current) => ({ ...current, avatar_url: value }));
        }}
        onDisplayNameChange={(displayName) => {
          setProfileSaved(false);
          setProfile((current) => ({ ...current, display_name: displayName }));
        }}
        onSave={(event) => void saveProfile(event)}
      />
      <OrganizationCard
        organizationId={organizationId}
        organizations={organizations}
        onOrganizationChange={onOrganizationChange}
      />
      <ApiKeySection api={api} organizationId={organizationId} principal={principal} onError={onError} />
    </div>
  </section>;
}

interface ProfileCardProps {
  avatarDraft: string;
  principal: Principal;
  profile: UserProfile;
  profileSaved: boolean;
  saving: boolean;
  onAvatarChange: (value: string | null) => void;
  onDisplayNameChange: (value: string) => void;
  onSave: (event: FormEvent<HTMLFormElement>) => void;
}

function ProfileCard({
  avatarDraft,
  principal,
  profile,
  profileSaved,
  saving,
  onAvatarChange,
  onDisplayNameChange,
  onSave,
}: ProfileCardProps) {
  const { t } = useI18n();
  return <section className="settings-card settings-profile-card">
    <div className="settings-card-heading">
      <UserAvatar displayName={profile.display_name} userId={principal.user_id} avatarUrl={profile.avatar_url} size="large" />
      <div><h3>{t("profileSettings")}</h3><p>{t("profileSettingsHelp")}</p></div>
    </div>
    <form className="settings-form" onSubmit={onSave}>
      <label>
        <span>{t("displayName")}</span>
        <input
          required
          minLength={1}
          maxLength={80}
          value={profile.display_name}
          onChange={(event) => onDisplayNameChange(event.target.value)}
        />
      </label>
      <div className="avatar-editor">
        <UserAvatar
          displayName={profile.display_name}
          userId={principal.user_id}
          avatarUrl={avatarDraft || null}
          size="large"
          disabled={saving}
          onChange={onAvatarChange}
        />
      </div>
      <button className="button primary" disabled={saving || !profile.display_name.trim()}>{saving ? t("saving") : t("saveProfile")}</button>
      {profileSaved && <p className="success-inline" role="status">{t("profileSaved")}</p>}
    </form>
  </section>;
}

function OrganizationCard({
  organizationId,
  organizations,
  onOrganizationChange,
}: Pick<Props, "organizationId" | "organizations" | "onOrganizationChange">) {
  const { t } = useI18n();
  return <section className="settings-card settings-organization-card">
    <h3>{t("organizationSettings")}</h3>
    <p>{t("organizationSwitchHelp")}</p>
    <label className="settings-form">
      <span>{t("currentOrganization")}</span>
      <select
        value={organizationId}
        onChange={(event) => {
          if (event.target.value) onOrganizationChange(event.target.value);
        }}
      >
        {!organizationId && <option value="" disabled>{t("chooseOrganization")}</option>}
        {organizations.map((organization) => <option key={organization.id} value={organization.id}>{organization.name}</option>)}
      </select>
    </label>
  </section>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function customAvatarUrl(value: string | null | undefined): string {
  return value && /^(data:image\/(png|jpeg|webp);base64,)/i.test(value) ? value : "";
}
