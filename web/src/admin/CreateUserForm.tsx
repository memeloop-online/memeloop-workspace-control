import { useMemo, useState, type FormEvent } from "react";
import {
  Button,
  Checkbox,
  Field,
  Input,
  Option,
  Select,
  Text,
} from "@fluentui/react-components";

import type { ApiClient } from "../api";
import { API_KEY_SCOPES, DEFAULT_USER_SCOPES, SYSTEM_ADMIN_SCOPES } from "../apiKeyScopes";
import { useI18n } from "../i18n";
import type { ApiKeyScope, Principal, Role } from "../types";
import { AdminToolbar, SaveButton, useAdminStyles } from "./fluentAdmin";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  onCreated: () => Promise<void> | void;
  onCancel: () => void;
  onError: (message: string) => void;
}

export function CreateUserForm({ api, principal, organizationId, onCreated, onCancel, onError }: Props) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const grantableScopes = useMemo(
    () => API_KEY_SCOPES.filter(({ scope }) => principal.api_key_scopes.includes(scope)),
    [principal.api_key_scopes],
  );
  const [displayName, setDisplayName] = useState("");
  const [token, setToken] = useState("");
  const [systemAdmin, setSystemAdmin] = useState(false);
  const [organizationRole, setOrganizationRole] = useState<Role>("member");
  const [scopes, setScopes] = useState<ApiKeyScope[]>(() => DEFAULT_USER_SCOPES.filter((scope) => grantableScopes.some((item) => item.scope === scope)));
  const [expiresAt, setExpiresAt] = useState(defaultExpiry);
  const [saving, setSaving] = useState(false);

  function toggleScope(scope: ApiKeyScope) {
    setScopes((current) => current.includes(scope) ? current.filter((item) => item !== scope) : [...current, scope]);
  }

  function setAdministrator(value: boolean) {
    setSystemAdmin(value);
    const defaults = value ? SYSTEM_ADMIN_SCOPES : DEFAULT_USER_SCOPES;
    setScopes(defaults.filter((scope) => grantableScopes.some((item) => item.scope === scope)));
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    const expires = Math.floor(new Date(expiresAt).getTime() / 1_000);
    if (!Number.isFinite(expires) || scopes.length === 0) return;
    setSaving(true);
    try {
      await api.createUser({
        display_name: displayName.trim(),
        token,
        system_admin: systemAdmin,
        scopes,
        expires_at: expires,
        organization_id: organizationId,
        organization_role: organizationRole === "organization_admin" ? "organization_admin" : "member",
      });
      setToken("");
      await onCreated();
    } catch (error) {
      onError(error instanceof Error ? error.message : t("requestFailed"));
    } finally {
      setSaving(false);
    }
  }

  return <form className={styles.stack} onSubmit={(event) => void submit(event)}>
    <div className={styles.formGrid}>
      <Field label={t("displayName")} required>
        <Input value={displayName} onChange={(event) => setDisplayName(event.target.value)} />
      </Field>
      <Field label={t("initialTokenPrompt")} required>
        <Input type="password" minLength={32} maxLength={512} autoComplete="new-password" value={token} onChange={(event) => setToken(event.target.value)} />
      </Field>
      <Field label={t("apiKeyExpires")} required>
        <Input type="datetime-local" min={localDateTime(new Date())} max={localDateTime(new Date(Date.now() + 365 * 86_400_000))} value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} />
      </Field>
      <Field label={t("role")}>
        <Select value={organizationRole} onChange={(event) => setOrganizationRole(event.target.value as Role)}>
          <Option value="member">{t("roleMember")}</Option>
          <Option value="organization_admin">{t("roleOrganizationAdmin")}</Option>
        </Select>
      </Field>
    </div>
    <Checkbox checked={systemAdmin} onChange={(_, data) => setAdministrator(Boolean(data.checked))} label={t("systemAdmin")} />
    <div className={styles.stack}>
      <Text weight="semibold">{t("apiKeyPermissions")}</Text>
      <div className={styles.formGrid}>
        {grantableScopes.map(({ scope, label }) => <Checkbox key={scope} checked={scopes.includes(scope)} onChange={() => toggleScope(scope)} label={t(label)} />)}
      </div>
    </div>
    <AdminToolbar action={<div className={styles.actions}>
      <Button type="button" appearance="secondary" disabled={saving} onClick={onCancel}>{t("cancel")}</Button>
      <SaveButton type="submit" disabled={saving || !displayName.trim() || token.length < 32 || !expiresAt || scopes.length === 0}>{saving ? t("saving") : t("createUser")}</SaveButton>
    </div>}>
      <Text size={300}>{t("apiKeyExpires")}: {expiresAt}</Text>
    </AdminToolbar>
  </form>;
}

function localDateTime(value: Date): string {
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}

function defaultExpiry(): string {
  return localDateTime(new Date(Date.now() + 30 * 86_400_000));
}
