import { Badge, Button, DataGridCell, Text, makeStyles, tokens } from "@fluentui/react-components";
import { useI18n } from "../i18n";
import type { ReactNode } from "react";
import type { AuditRecord } from "../types";
import { auditActionLabel, auditStateLabel, hasKnownAuditAction } from "./auditPresentation";

const useStyles = makeStyles({
  cell: { minWidth: 0 },
  stack: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  primary: { overflowWrap: "anywhere" },
  technical: { color: tokens.colorNeutralForeground3, fontFamily: tokens.fontFamilyMonospace, fontSize: tokens.fontSizeBase100, overflowWrap: "anywhere" },
  subtle: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  state: { width: "fit-content" },
  detailButton: { width: "fit-content", justifySelf: "start", paddingInline: tokens.spacingHorizontalXS },
});

function Action({ record }: { record: AuditRecord }) {
  const styles = useStyles();
  const { t } = useI18n();
  const stateKey = auditStateLabel(record);
  return <div className={styles.stack}>
    <Text className={styles.primary} weight="semibold">{t(auditActionLabel(record.action))}</Text>
    {!hasKnownAuditAction(record.action) && <code className={styles.technical} title={record.action}>{record.action}</code>}
    {stateKey && <Badge className={styles.state} appearance="tint" color={stateKey === "stateFailed" ? "danger" : stateKey === "disabled" ? "informative" : "success"}>{t(stateKey)}</Badge>}
  </div>;
}

function Actor({ record }: { record: AuditRecord }) {
  const styles = useStyles();
  const { t } = useI18n();
  const name = record.actor_display_name?.trim() || (record.actor_user_id ? t("deletedActor") : t("systemActor"));
  return <div className={styles.stack}>
    <Text className={styles.primary} weight="semibold">{name}</Text>
    {record.actor_user_id && <code className={styles.technical} title={`${t("auditTechnicalId")}: ${record.actor_user_id}`}>{shortId(record.actor_user_id)}</code>}
  </div>;
}

function Target({ record }: { record: AuditRecord }) {
  const styles = useStyles();
  const { t } = useI18n();
  if (record.workspace_id) {
    return <div className={styles.stack}>
      <Text className={styles.primary} weight="semibold">{record.workspace_name ?? t("auditDeletedWorkspace")}</Text>
      {record.workspace_short_id && <code className={styles.technical}>{record.workspace_short_id}</code>}
      <code className={styles.technical} title={`${t("auditTechnicalId")}: ${record.workspace_id}`}>{shortId(record.workspace_id)}</code>
    </div>;
  }
  if (record.organization_id) {
    return <div className={styles.stack}>
      <Text className={styles.primary} weight="semibold">{t("currentOrganization")}</Text>
      <code className={styles.technical} title={`${t("auditTechnicalId")}: ${record.organization_id}`}>{shortId(record.organization_id)}</code>
    </div>;
  }
  return <Text className={styles.subtle}>{t("auditPlatformObject")}</Text>;
}

function Time({ record, locale, onDetails }: { record: AuditRecord; locale: string; onDetails: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  return <div className={styles.stack}>
    <time className={styles.primary} dateTime={new Date(record.created_at * 1_000).toISOString()}>{new Date(record.created_at * 1_000).toLocaleString(locale)}</time>
    {Object.keys(record.metadata).length > 0 && <Button className={styles.detailButton} appearance="subtle" size="small" onClick={onDetails}>{t("auditDetails")}</Button>}
  </div>;
}

function Cell({ children }: { children: ReactNode }) {
  const styles = useStyles();
  return <DataGridCell className={styles.cell}>{children}</DataGridCell>;
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}…` : value;
}

export const AuditRow = { Action, Actor, Target, Time, Cell };
