import {
  Badge,
  Body2,
  Button,
  Card,
  Caption1,
  Text,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

import type { MessageKey } from "../i18n";
import { API_KEY_SCOPES } from "../apiKeyScopes";
import { getApiKeyStatus } from "../apiKeyStatus";
import type { ApiKeySummary } from "../types";

interface Props {
  keys: readonly ApiKeySummary[];
  locale: string;
  onRevoke: (key: ApiKeySummary) => void;
  translate: (key: MessageKey) => string;
}

const useStyles = makeStyles({
  list: {
    display: "grid",
    gap: tokens.spacingVerticalS,
  },
  item: {
    display: "grid",
    gridTemplateColumns: "minmax(180px, 1fr) minmax(0, 1.5fr) auto",
    gap: tokens.spacingHorizontalL,
    alignItems: "center",
    padding: tokens.spacingHorizontalL,
    "@media (max-width: 760px)": {
      gridTemplateColumns: "1fr",
      gap: tokens.spacingVerticalM,
    },
  },
  identity: {
    display: "grid",
    gap: tokens.spacingVerticalXS,
    minWidth: 0,
  },
  name: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  prefix: {
    width: "fit-content",
    maxWidth: "100%",
    padding: `${tokens.spacingVerticalXXS} ${tokens.spacingHorizontalXS}`,
    borderRadius: tokens.borderRadiusSmall,
    backgroundColor: tokens.colorNeutralBackground3,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  facts: {
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: tokens.spacingHorizontalM,
    margin: 0,
    "@media (max-width: 760px)": {
      gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    },
    "@media (max-width: 420px)": {
      gridTemplateColumns: "1fr",
    },
  },
  fact: {
    minWidth: 0,
  },
  factLabel: {
    color: tokens.colorNeutralForeground3,
  },
  factValue: {
    display: "block",
    marginBlockStart: tokens.spacingVerticalXXS,
    overflowWrap: "anywhere",
  },
  empty: {
    padding: tokens.spacingHorizontalL,
    color: tokens.colorNeutralForeground2,
    textAlign: "center",
  },
  revoke: {
    justifySelf: "end",
    "@media (max-width: 760px)": {
      justifySelf: "stretch",
    },
  },
  status: {
    width: "fit-content",
  },
});

export function ApiKeyList({ keys, locale, onRevoke, translate }: Props) {
  const classes = useStyles();
  if (keys.length === 0) return <Card className={classes.empty}>{translate("noApiKeys")}</Card>;

  return <div className={classes.list} aria-label={translate("apiKeys")}>
    {keys.map((key) => {
      const status = getApiKeyStatus(key);
      const statusIntent: "success" | "warning" | "danger" = status === "active" ? "success" : status === "expired" ? "warning" : "danger";
      const statusLabel = status === "active" ? translate("apiKeyActive") : status === "expired" ? translate("apiKeyExpired") : translate("apiKeyRevoked");
      return <Card className={classes.item} key={key.id}>
        <div className={classes.identity}>
          <Body2 className={classes.name} title={key.name}>{key.name}</Body2>
          <Text className={classes.prefix} font="monospace">{key.prefix}</Text>
          <Badge className={classes.status} appearance="tint" color={statusIntent}>{statusLabel}</Badge>
          <Caption1>{formatScopes(key, translate) ?? translate("apiKeyScopesUnavailable")}</Caption1>
        </div>
        <dl className={classes.facts}>
          <div className={classes.fact}><dt className={classes.factLabel}>{translate("createdAt")}</dt><dd className={classes.factValue}>{formatTime(key.created_at, locale)}</dd></div>
          <div className={classes.fact}><dt className={classes.factLabel}>{translate("lastUsedAt")}</dt><dd className={classes.factValue}>{key.last_used_at ? formatTime(key.last_used_at, locale) : translate("never")}</dd></div>
          <div className={classes.fact}><dt className={classes.factLabel}>{translate("apiKeyExpires")}</dt><dd className={classes.factValue}>{formatExpiry(key, locale, translate)}</dd></div>
          <div className={classes.fact}><dt className={classes.factLabel}>{translate("apiKeyTemplates")}</dt><dd className={classes.factValue}>{formatTemplateRestriction(key, translate)}</dd></div>
        </dl>
        <Button className={classes.revoke} appearance="secondary" disabled={status === "revoked"} onClick={() => onRevoke(key)}>
          {translate("revokeApiKey")}
        </Button>
      </Card>;
    })}
  </div>;
}

function formatTemplateRestriction(key: ApiKeySummary, translate: (key: MessageKey) => string): string {
  if (key.allowed_template_ids === null) return translate("allTemplates");
  if (key.allowed_template_ids.length === 0) return translate("noTemplates");
  return key.allowed_template_ids.join(" · ");
}

function formatTime(value: number, locale: string) {
  return new Date(value * 1_000).toLocaleString(locale);
}

function formatScopes(key: ApiKeySummary, translate: (key: MessageKey) => string): string | null {
  if (!Array.isArray(key.scopes) || key.scopes.length === 0) return null;
  const labels = key.scopes.map((scope) => {
    const definition = API_KEY_SCOPES.find((item) => item.scope === scope);
    return definition ? translate(definition.label) : translate("scopeUnknown");
  });
  return labels.join(" · ");
}

function formatExpiry(key: ApiKeySummary, locale: string, translate: (key: MessageKey) => string): string {
  if (typeof key.expires_at === "number") return formatTime(key.expires_at, locale);
  if (key.expires_at === null) return translate("never");
  return translate("apiKeyExpiryUnavailable");
}
