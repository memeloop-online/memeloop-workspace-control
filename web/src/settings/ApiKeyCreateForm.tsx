import type { FormEvent } from "react";

import type { ApiKeyScope, WorkspaceTemplate } from "../types";
import type { MessageKey } from "../i18n";
import { ApiKeyScopePicker, type GrantableApiKeyScope } from "./ApiKeyScopePicker";
import { ApiKeyTemplatePicker } from "./ApiKeyTemplatePicker";

interface Props {
  name: string;
  scopes: ApiKeyScope[];
  expiresAt: string;
  grantableScopes: readonly GrantableApiKeyScope[];
  templates: readonly WorkspaceTemplate[];
  templateRestriction: boolean;
  allowedTemplateIds: readonly string[];
  templateRestrictionDisabled?: boolean;
  creating: boolean;
  onNameChange: (value: string) => void;
  onScopesChange: (value: ApiKeyScope[]) => void;
  onExpiresAtChange: (value: string) => void;
  onTemplateRestrictionChange: (value: boolean) => void;
  onAllowedTemplateIdsChange: (value: string[]) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  translate: (key: MessageKey) => string;
}

export function ApiKeyCreateForm({
  name,
  scopes,
  expiresAt,
  grantableScopes,
  templates,
  templateRestriction,
  allowedTemplateIds,
  templateRestrictionDisabled = false,
  creating,
  onNameChange,
  onScopesChange,
  onExpiresAtChange,
  onTemplateRestrictionChange,
  onAllowedTemplateIdsChange,
  onSubmit,
  translate,
}: Props) {
  return <form className="api-key-create-form" onSubmit={onSubmit}>
    <div className="api-key-create-heading">
      <div>
        <h4>{translate("createApiKey")}</h4>
        <p>{translate("apiKeysHelp")}</p>
      </div>
    </div>
    <div className="api-key-create-fields">
      <label>
        <span>{translate("apiKeyName")}</span>
        <input
          maxLength={80}
          placeholder={translate("apiKeyNamePlaceholder")}
          required
          value={name}
          onChange={(event) => onNameChange(event.target.value)}
        />
      </label>
      <label>
        <span>{translate("apiKeyExpires")}</span>
        <input
          required
          type="datetime-local"
          min={localDateTime(new Date())}
          max={localDateTime(new Date(Date.now() + 365 * 86_400_000))}
          value={expiresAt}
          onChange={(event) => onExpiresAtChange(event.target.value)}
        />
      </label>
    </div>
    <ApiKeyScopePicker
      legend={`${translate("apiKeyPermissions")} · ${translate("apiKeyPermissionsHelp")}`}
      scopes={grantableScopes}
      selected={scopes}
      translate={translate}
      onChange={onScopesChange}
    />
    <ApiKeyTemplatePicker
      disabled={creating}
      restrictionDisabled={templateRestrictionDisabled}
      restricted={templateRestriction}
      selected={allowedTemplateIds}
      templates={templates}
      translate={translate}
      onRestrictedChange={onTemplateRestrictionChange}
      onSelectedChange={onAllowedTemplateIdsChange}
    />
    <div className="api-key-create-actions">
      <button className="button primary" disabled={creating || !name.trim() || !expiresAt || scopes.length === 0}>
        {creating ? translate("saving") : translate("createApiKey")}
      </button>
    </div>
  </form>;
}

function localDateTime(value: Date): string {
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}
