import type { FormEvent } from "react";
import {
  Button,
  Field,
  Input,
  Subtitle2,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

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

const useStyles = makeStyles({
  root: {
    display: "grid",
    gap: tokens.spacingVerticalL,
    padding: tokens.spacingHorizontalL,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground2,
  },
  fields: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr) minmax(220px, .7fr)",
    gap: tokens.spacingHorizontalL,
    "@media (max-width: 640px)": {
      gridTemplateColumns: "1fr",
    },
  },
  actions: {
    display: "flex",
    justifyContent: "flex-end",
  },
});

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
  const classes = useStyles();
  return <form className={classes.root} onSubmit={onSubmit}>
    <Subtitle2>{translate("createApiKey")}</Subtitle2>
    <div className={classes.fields}>
      <Field label={translate("apiKeyName")} required>
        <Input
          maxLength={80}
          placeholder={translate("apiKeyNamePlaceholder")}
          required
          value={name}
          onChange={(event) => onNameChange(event.target.value)}
        />
      </Field>
      <Field label={translate("apiKeyExpires")} required>
        <Input
          required
          type="datetime-local"
          min={localDateTime(new Date())}
          max={localDateTime(new Date(Date.now() + 365 * 86_400_000))}
          value={expiresAt}
          onChange={(event) => onExpiresAtChange(event.target.value)}
        />
      </Field>
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
    <div className={classes.actions}>
      <Button appearance="primary" type="submit" disabled={creating || !name.trim() || !expiresAt || scopes.length === 0}>
        {creating ? translate("saving") : translate("createApiKey")}
      </Button>
    </div>
  </form>;
}

function localDateTime(value: Date): string {
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}
