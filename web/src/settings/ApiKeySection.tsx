import { useEffect, useMemo, useState, type FormEvent } from "react";

import type { ApiClient } from "../api";
import { API_KEY_SCOPES } from "../apiKeyScopes";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useI18n } from "../i18n";
import type { ApiKeyScope, ApiKeySummary, CreatedApiKey, Principal, WorkspaceTemplate } from "../types";
import { ApiKeyCreateForm } from "./ApiKeyCreateForm";
import { ApiKeyList } from "./ApiKeyList";
import { normalizeTemplateSelection, templatesAllowedForPrincipal } from "./apiKeyTemplatePickerModel";

interface Props {
  api: ApiClient;
  organizationId: string;
  principal: Principal;
  onError: (message: string) => void;
}

export function ApiKeySection({ api, organizationId, principal, onError }: Props) {
  const { locale, t } = useI18n();
  const [keys, setKeys] = useState<ApiKeySummary[]>([]);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<ApiKeyScope[]>(["read_workspace"]);
  const [expiresAt, setExpiresAt] = useState(defaultExpiry);
  const [createdKey, setCreatedKey] = useState<CreatedApiKey | null>(null);
  const [access, setAccess] = useState<"loading" | "available" | "unavailable">("loading");
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<ApiKeySummary | null>(null);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [templateRestriction, setTemplateRestriction] = useState(() => principal.allowed_template_ids !== null);
  const [allowedTemplateIds, setAllowedTemplateIds] = useState<string[]>([]);
  const [revokeBusy, setRevokeBusy] = useState(false);
  const selectableTemplates = useMemo(
    () => templatesAllowedForPrincipal(templates, principal.allowed_template_ids),
    [principal.allowed_template_ids, templates],
  );
  const templateRestrictionDisabled = principal.allowed_template_ids !== null;
  const grantableScopes = useMemo(
    () => API_KEY_SCOPES.filter(({ scope }) => principal.api_key_scopes.includes(scope)),
    [principal.api_key_scopes],
  );

  useEffect(() => {
    setScopes((current) => {
      const allowed = current.filter((scope) => grantableScopes.some((item) => item.scope === scope));
      return allowed.length > 0 ? allowed : grantableScopes[0] ? [grantableScopes[0].scope] : [];
    });
  }, [grantableScopes]);

  useEffect(() => {
    let active = true;
    setAccess("loading");
    setKeys([]);
    setCreatedKey(null);
    void api.apiKeys()
      .then((nextKeys) => {
        if (!active) return;
        setKeys(nextKeys);
        setAccess("available");
      })
      .catch((error) => {
        if (!active) return;
        if (isForbidden(error)) {
          setAccess("unavailable");
          return;
        }
        onError(message(error, t("requestFailed")));
        setAccess("available");
      });
    return () => { active = false; };
  }, [api, onError, t]);

  useEffect(() => {
    // The organization-scoped template list must never leak into the next
    // organization while its request is in flight.
    setTemplates([]);
    setTemplateRestriction((current) => templateRestrictionDisabled || current);
    setAllowedTemplateIds([]);
    if (!organizationId) return;
    let active = true;
    void api.templates(organizationId).then((items) => { if (active) setTemplates(items); })
      .catch((error) => { if (active) onError(message(error, t("requestFailed"))); });
    return () => { active = false; };
  }, [api, organizationId, onError, t, templateRestrictionDisabled]);

  useEffect(() => {
    setAllowedTemplateIds((current) => normalizeTemplateSelection(current, selectableTemplates));
  }, [selectableTemplates]);

  async function createKey(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) return;
    setCreating(true);
    try {
      const restrictedForRequest = templateRestriction || templateRestrictionDisabled;
      const created = await api.createApiKey({
        name: name.trim(),
        scopes,
        expires_at: Math.floor(new Date(expiresAt).getTime() / 1_000),
        allowed_template_ids: restrictedForRequest
          ? normalizeTemplateSelection(allowedTemplateIds, selectableTemplates)
          : null,
      });
      setCreatedKey(created);
      setName("");
      setScopes(grantableScopes[0] ? [grantableScopes[0].scope] : []);
      setExpiresAt(defaultExpiry());
      setTemplateRestriction(templateRestrictionDisabled);
      setAllowedTemplateIds([]);
      setKeys(await api.apiKeys());
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setCreating(false);
    }
  }

  async function confirmRevoke() {
    if (!revoking) return;
    setRevokeBusy(true);
    try {
      await api.deleteApiKey(revoking.id);
      setKeys((current) => current.filter((item) => item.id !== revoking.id));
      if (createdKey?.id === revoking.id) setCreatedKey(null);
      setRevoking(null);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setRevokeBusy(false);
    }
  }

  return <section className="settings-card settings-api-key-card">
    <div className="settings-api-key-heading">
      <div><h3>{t("apiKeys")}</h3><p>{t("apiKeysHelp")}</p></div>
    </div>
    {access === "unavailable" ? <p className="settings-unavailable" role="status">{t("apiKeysUnavailable")}</p>
      : access === "loading" ? <p className="settings-loading" role="status">{t("loading")}</p>
        : <>
          {createdKey && <CreatedApiKeyNotice value={createdKey} onHide={() => setCreatedKey(null)} />}
          <ApiKeyCreateForm
            creating={creating}
            expiresAt={expiresAt}
            grantableScopes={grantableScopes}
            name={name}
            scopes={scopes}
            templates={selectableTemplates}
            templateRestriction={templateRestriction}
            templateRestrictionDisabled={templateRestrictionDisabled}
            allowedTemplateIds={allowedTemplateIds}
            translate={t}
            onExpiresAtChange={setExpiresAt}
            onNameChange={setName}
            onScopesChange={setScopes}
            onTemplateRestrictionChange={(restricted) => setTemplateRestriction(templateRestrictionDisabled || restricted)}
            onAllowedTemplateIdsChange={setAllowedTemplateIds}
            onSubmit={(event) => void createKey(event)}
          />
          <ApiKeyList keys={keys} locale={locale} translate={t} onRevoke={setRevoking} />
        </>}
    <ConfirmDialog
      busy={revokeBusy}
      cancelLabel={t("cancel")}
      confirmLabel={t("revokeApiKey")}
      danger
      description={revoking ? `${t("revokeApiKeyConfirm")} ${revoking.name}?` : t("revokeApiKeyConfirm")}
      details={revoking ? <code>{revoking.prefix}</code> : undefined}
      open={Boolean(revoking)}
      title={t("revokeApiKey")}
      onClose={() => setRevoking(null)}
      onConfirm={() => void confirmRevoke()}
    />
  </section>;
}

function CreatedApiKeyNotice({ value, onHide }: { value: CreatedApiKey; onHide: () => void }) {
  const { t } = useI18n();
  return <div className="new-api-key" role="status">
    <strong>{t("apiKeyCreated")}</strong>
    <p>{t("apiKeyCreatedHelp")}</p>
    <div><code>{value.token}</code><button className="button" type="button" onClick={() => void navigator.clipboard.writeText(value.token)}>{t("copy")}</button></div>
    <button className="text-button" type="button" onClick={onHide}>{t("hideApiKey")}</button>
  </div>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function isForbidden(error: unknown): boolean {
  if (!error) return false;
  const status = typeof error === "object" && error !== null && "status" in error ? (error as { status?: unknown }).status : undefined;
  if (status === 403) return true;
  return error instanceof Error && /\b403\b|forbidden|not allowed/i.test(error.message);
}

function defaultExpiry(): string {
  const value = new Date(Date.now() + 30 * 86_400_000);
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}
