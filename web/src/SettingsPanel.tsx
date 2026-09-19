import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import {
  Body1,
  Button,
  Card,
  CardHeader,
  Dropdown,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Option,
  Subtitle1,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

import type { ApiClient } from "./api";
import { Page } from "./design-system/Page";
import { useI18n } from "./i18n";
import type { Organization, Principal, UserProfile } from "./types";
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

const useStyles = makeStyles({
  cards: {
    display: "grid",
    gridTemplateColumns: "repeat(12, minmax(0, 1fr))",
    gap: tokens.spacingHorizontalL,
    alignItems: "stretch",
    "@media (max-width: 900px)": {
      gridTemplateColumns: "1fr",
    },
  },
  halfCard: {
    gridColumn: "span 6",
    minWidth: 0,
    "@media (max-width: 900px)": {
      gridColumn: "auto",
    },
  },
  fullCard: {
    gridColumn: "1 / -1",
    minWidth: 0,
  },
  card: {
    display: "grid",
    alignContent: "start",
    gap: tokens.spacingVerticalL,
    padding: tokens.spacingHorizontalXL,
    minWidth: 0,
    "@media (max-width: 640px)": {
      padding: tokens.spacingHorizontalL,
    },
  },
  cardHeader: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    minWidth: 0,
  },
  cardHeaderText: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
    minWidth: 0,
  },
  cardDescription: {
    color: tokens.colorNeutralForeground2,
  },
  form: {
    display: "grid",
    gap: tokens.spacingVerticalL,
  },
  actions: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    flexWrap: "wrap",
  },
  status: {
    minWidth: 0,
  },
});

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
  const classes = useStyles();
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
    // Profile loading is scoped to this API client. The shell callback is
    // intentionally excluded so parent renders do not repeat the request.
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

  return <Page title={t("settingsTitle")}>
      <div className={classes.cards}>
        <ProfileCard
          className={classes.halfCard}
          cardClassName={classes.card}
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
          className={classes.halfCard}
          cardClassName={classes.card}
          organizationId={organizationId}
          organizations={organizations}
          onOrganizationChange={onOrganizationChange}
        />
      </div>
    </Page>;
}

interface ProfileCardProps {
  className: string;
  cardClassName: string;
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
  className,
  cardClassName,
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
  const classes = useStyles();
  return <Card appearance="outline" className={`${className} ${cardClassName}`}>
    <CardHeader
      image={<UserAvatar displayName={profile.display_name} userId={principal.user_id} avatarUrl={profile.avatar_url} size="large" />}
      header={<Subtitle1>{t("profileSettings")}</Subtitle1>}
      description={<Body1 className={classes.cardDescription}>{t("displayName")}</Body1>}
    />
    <form className={classes.form} onSubmit={onSave}>
      <Field label={t("displayName")} required>
        <Input
          required
          minLength={1}
          maxLength={80}
          value={profile.display_name}
          onChange={(event) => onDisplayNameChange(event.target.value)}
        />
      </Field>
      <UserAvatar
        displayName={profile.display_name}
        userId={principal.user_id}
        avatarUrl={avatarDraft || null}
        size="large"
        disabled={saving}
        onChange={onAvatarChange}
      />
      <div className={classes.actions}>
        <Button appearance="primary" type="submit" disabled={saving || !profile.display_name.trim()}>
          {saving ? t("saving") : t("saveProfile")}
        </Button>
        {profileSaved && <MessageBar className={classes.status} intent="success">
          <MessageBarBody>{t("profileSaved")}</MessageBarBody>
        </MessageBar>}
      </div>
    </form>
  </Card>;
}

function OrganizationCard({
  className,
  cardClassName,
  organizationId,
  organizations,
  onOrganizationChange,
}: Pick<Props, "organizationId" | "organizations" | "onOrganizationChange"> & { className: string; cardClassName: string }) {
  const { t } = useI18n();
  const classes = useStyles();
  const selectedOrganization = organizations.find((organization) => organization.id === organizationId);
  return <Card appearance="outline" className={`${className} ${cardClassName}`}>
    <CardHeader
      header={<Subtitle1>{t("organizationSettings")}</Subtitle1>}
      description={<Body1 className={classes.cardDescription}>{t("organizationSwitchHelp")}</Body1>}
    />
    <Field label={t("currentOrganization")} required>
      <Dropdown
        value={selectedOrganization?.name ?? ""}
        selectedOptions={organizationId ? [organizationId] : []}
        disabled={organizations.length === 0}
        onOptionSelect={(_, data) => {
          if (data.optionValue) onOrganizationChange(data.optionValue);
        }}
      >
        {organizations.map((organization) => <Option key={organization.id} value={organization.id} text={organization.name}>
          {organization.name}
        </Option>)}
      </Dropdown>
    </Field>
  </Card>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function customAvatarUrl(value: string | null | undefined): string {
  return value && /^(data:image\/(png|jpeg|webp);base64,)/i.test(value) ? value : "";
}
