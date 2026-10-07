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
import { AdminPanel } from "./OperationsPanel";
import { Page } from "./design-system/Page";
import { useI18n } from "./i18n";
import type { Locale, MessageKey } from "./i18n";
import { matchingFields, settingsFields, type SettingsField, type SimpleSectionId } from "./settings/fields";
import { visibleSettingsSections, type SettingsCategory } from "./settings/schema";
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
  onOrganizationsChanged: (preferredOrganizationId?: string) => Promise<void>;
  theme: "light" | "dark";
  onThemeChange: (theme: "light" | "dark") => void;
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
  navigation: { display: "flex", gap: tokens.spacingHorizontalS, flexWrap: "wrap" },
  settingsLayout: { display: "grid", gap: tokens.spacingVerticalXL },
  search: { width: "100%", maxWidth: "560px" },
});

export function SettingsPanel({
  api,
  principal,
  organizations,
  organizationId,
  onOrganizationChange,
  onProfileChanged,
  onError,
  onOrganizationsChanged,
  theme,
  onThemeChange,
}: Props) {
  const { t, locale, setLocale } = useI18n();
  const classes = useStyles();
  const [profile, setProfile] = useState<UserProfile>({
    display_name: principal.display_name,
    avatar_url: principal.avatar_url ?? null,
  });
  const [avatarDraft, setAvatarDraft] = useState(customAvatarUrl(principal.avatar_url));
  const [saving, setSaving] = useState(false);
  const [profileSaved, setProfileSaved] = useState(false);
  const [category, setCategory] = useState<SettingsCategory | "all">("all");
  const [search, setSearch] = useState("");
  const sections = visibleSettingsSections(principal, organizationId, category, search, t);
  const visible = new Set(sections.map((section) => section.id));
  const adminSections = sections.filter((section) => !["profile", "appearance", "organization"].includes(section.id)).map((section) => section.id);
  const fieldsFor = (section: SimpleSectionId) => {
    const matched = matchingFields(section, search, t);
    return matched.length > 0 ? matched : settingsFields.filter((field) => field.section === section);
  };

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
      <div className={classes.settingsLayout}>
      <Field label={t("settingsSearch")}><Input className={classes.search} data-testid="settings-search" type="search" value={search} onChange={(_, data) => setSearch(data.value)} placeholder={t("settingsSearchPlaceholder")} /></Field>
      <nav className={classes.navigation} aria-label={t("settingsCategories")}>
        {(["all", "personal", "organization", "system"] as const).filter((item) => item === "all" || visibleSettingsSections(principal, organizationId, item, "", t).length > 0).map((item) => <Button key={item} aria-pressed={category === item} appearance={category === item ? "primary" : "secondary"} onClick={() => setCategory(item)}>{t(categoryLabels[item])}</Button>)}
      </nav>
      {sections.length === 0 && <Body1 role="status">{t("settingsNoResults")}</Body1>}
      <div className={classes.cards}>
        {visible.has("profile") && <section data-testid="settings-section-profile" className={classes.halfCard}><ProfileCard
          className=""
          cardClassName={classes.card}
          avatarDraft={avatarDraft}
          principal={principal}
          profile={profile}
          profileSaved={profileSaved}
          fields={fieldsFor("profile")}
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
        /></section>}
        {visible.has("appearance") && <section data-testid="settings-section-appearance" className={classes.halfCard}><Card appearance="outline" className={classes.card}><CardHeader header={<Subtitle1>{t("appearanceSettings")}</Subtitle1>} />{fieldsFor("appearance").map((field) => <SchemaField key={field.key} definition={field} value={field.key === "theme" ? theme : locale} onChange={(value) => field.key === "theme" ? onThemeChange(value as "light" | "dark") : setLocale(value as Locale)} />)}</Card></section>}
        {visible.has("organization") && <section data-testid="settings-section-organization" className={classes.halfCard}><OrganizationCard
          className=""
          cardClassName={classes.card}
          organizationId={organizationId}
          organizations={organizations}
          fields={fieldsFor("organization")}
          onOrganizationChange={onOrganizationChange}
        /></section>}
      </div>
      {adminSections.length > 0 && <AdminPanel api={api} principal={principal} organizationId={organizationId} onError={onError} onOrganizationsChanged={onOrganizationsChanged} visibleSections={adminSections} embedded />}
      </div>
    </Page>;
}

const categoryLabels = { all: "settingsCategoryAll", personal: "settingsCategoryPersonal", organization: "settingsCategoryOrganization", system: "settingsCategorySystem" } as const satisfies Record<SettingsCategory | "all", MessageKey>;

function SchemaField({ definition, value, onChange, organizations, avatar }: { definition: SettingsField; value: string; onChange: (value: string) => void; organizations?: Organization[]; avatar?: { principal: Principal; displayName: string; disabled: boolean } }) {
  const { t } = useI18n();
  if (definition.type === "avatar" && avatar) return <Field label={t(definition.label)} hint={definition.description ? t(definition.description) : undefined}><UserAvatar displayName={avatar.displayName} userId={avatar.principal.user_id} avatarUrl={value || null} size="large" disabled={avatar.disabled} onChange={(next) => onChange(next ?? "")} /></Field>;
  if (definition.type === "string-enum") {
    const choices = organizations ? organizations.map((organization) => ({ value: organization.id, label: organization.name })) : (definition.options ?? []).map((option) => ({ value: option.value, label: t(option.label) }));
    return <Field label={t(definition.label)} hint={definition.description ? t(definition.description) : undefined} required={definition.required}><Dropdown value={choices.find((choice) => choice.value === value)?.label ?? ""} selectedOptions={value ? [value] : []} disabled={choices.length === 0} onOptionSelect={(_, data) => { if (data.optionValue) onChange(data.optionValue); }}>{choices.map((choice) => <Option key={choice.value} value={choice.value} text={choice.label}>{choice.label}</Option>)}</Dropdown></Field>;
  }
  return <Field label={t(definition.label)} hint={definition.description ? t(definition.description) : undefined} required={definition.required}><Input required={definition.required} minLength={definition.required ? 1 : undefined} maxLength={definition.maxLength} value={value} onChange={(_, data) => onChange(data.value)} /></Field>;
}

interface ProfileCardProps {
  className: string;
  cardClassName: string;
  avatarDraft: string;
  principal: Principal;
  profile: UserProfile;
  profileSaved: boolean;
  fields: readonly SettingsField[];
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
  fields,
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
      {fields.map((field) => <SchemaField key={field.key} definition={field} value={field.key === "avatar_url" ? avatarDraft : profile.display_name} onChange={field.key === "avatar_url" ? onAvatarChange : onDisplayNameChange} avatar={{ principal, displayName: profile.display_name, disabled: saving }} />)}
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
  fields,
}: Pick<Props, "organizationId" | "organizations" | "onOrganizationChange"> & { className: string; cardClassName: string; fields: readonly SettingsField[] }) {
  const { t } = useI18n();
  const classes = useStyles();
  const selectedOrganization = organizations.find((organization) => organization.id === organizationId);
  return <Card appearance="outline" className={`${className} ${cardClassName}`}>
    <CardHeader
      header={<Subtitle1>{t("organizationSettings")}</Subtitle1>}
      description={<Body1 className={classes.cardDescription}>{t("organizationSwitchHelp")}</Body1>}
    />
    {fields.map((field) => <SchemaField key={field.key} definition={field} value={selectedOrganization?.id ?? ""} organizations={organizations} onChange={onOrganizationChange} />)}
  </Card>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function customAvatarUrl(value: string | null | undefined): string {
  return value && /^(data:image\/(png|jpeg|webp);base64,)/i.test(value) ? value : "";
}
