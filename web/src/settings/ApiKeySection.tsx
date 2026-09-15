import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  Body2,
  Button,
  Card,
  CardHeader,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Input,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

import type { ApiClient } from "../api";
import { API_KEY_SCOPES } from "../apiKeyScopes";
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

const useStyles = makeStyles({
  card: {
    display: "grid",
    gap: tokens.spacingVerticalL,
    padding: tokens.spacingHorizontalXL,
    minWidth: 0,
    "@media (max-width: 640px)": {
      padding: tokens.spacingHorizontalL,
    },
  },
  header: {
    minWidth: 0,
  },
  description: {
    color: tokens.colorNeutralForeground2,
  },
  loading: {
    display: "grid",
    placeItems: "center",
    minHeight: "140px",
  },
  created: {
    display: "grid",
    gap: tokens.spacingVerticalS,
  },
  createdValue: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr) auto",
    gap: tokens.spacingHorizontalS,
    alignItems: "center",
    "@media (max-width: 640px)": {
      gridTemplateColumns: "1fr",
    },
  },
  createdInput: {
    minWidth: 0,
  },
  dialogDetails: {
    marginBlockStart: tokens.spacingVerticalM,
  },
});

export function ApiKeySection({ api, organizationId, principal, onError }: Props) {
  const { locale, t } = useI18n();
  const classes = useStyles();
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
    setTemplates([]);
    setTemplateRestriction((current) => templateRestrictionDisabled || current);
    setAllowedTemplateIds([]);
    if (!organizationId) return;
    let active = true;
    void api.templates(organizationId)
      .then((items) => { if (active) setTemplates(items); })
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

  return <Card className={classes.card}>
    <CardHeader
      className={classes.header}
      header={<Subtitle1 id="api-keys-title">{t("apiKeys")}</Subtitle1>}
      description={<Body2 className={classes.description}>{t("apiKeysHelp")}</Body2>}
    />
    {access === "unavailable" ? <MessageBar intent="warning"><MessageBarBody>{t("apiKeysUnavailable")}</MessageBarBody></MessageBar>
      : access === "loading" ? <div className={classes.loading} role="status"><Spinner label={t("loading")} /></div>
        : <>
          {createdKey && <CreatedApiKeyNotice value={createdKey} onHide={() => setCreatedKey(null)} classes={classes} />}
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
    <Dialog open={Boolean(revoking)} onOpenChange={(_, data) => { if (!data.open && !revokeBusy) setRevoking(null); }}>
      <DialogSurface>
        <DialogBody>
          <DialogTitle>{t("revokeApiKey")}</DialogTitle>
          <DialogContent>
            {t("revokeApiKeyConfirm")}
            {revoking && <div className={classes.dialogDetails}><strong>{revoking.name}</strong><br /><code>{revoking.prefix}</code></div>}
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" disabled={revokeBusy} onClick={() => setRevoking(null)}>{t("cancel")}</Button>
            <Button appearance="primary" disabled={revokeBusy} onClick={() => void confirmRevoke()}>
              {revokeBusy ? <Spinner size="tiny" /> : t("revokeApiKey")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  </Card>;
}

function CreatedApiKeyNotice({ value, onHide, classes }: { value: CreatedApiKey; onHide: () => void; classes: ReturnType<typeof useStyles> }) {
  const { t } = useI18n();
  return <MessageBar className={classes.created} intent="success">
    <MessageBarBody>
      <strong>{t("apiKeyCreated")}</strong>
      <p>{t("apiKeyCreatedHelp")}</p>
      <div className={classes.createdValue}>
        <Input className={classes.createdInput} value={value.token} readOnly type="password" />
        <Button appearance="secondary" onClick={() => void navigator.clipboard.writeText(value.token)}>{t("copy")}</Button>
      </div>
      <Button appearance="subtle" onClick={onHide}>{t("hideApiKey")}</Button>
    </MessageBarBody>
  </MessageBar>;
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
