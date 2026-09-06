import { useI18n } from "../i18n";
import type { AuditRecord } from "../types";
import { auditActionLabel, auditStateLabel, hasKnownAuditAction } from "./auditPresentation";

export function AuditRow({ record, locale }: { record: AuditRecord; locale: string }) {
  const { t } = useI18n();
  const stateKey = auditStateLabel(record);
  const knownAction = hasKnownAuditAction(record.action);
  return <tr>
    <td data-label={t("auditAction")}><div className="audit-action"><strong>{t(auditActionLabel(record.action))}</strong><code className={knownAction ? undefined : "audit-action-unmapped"}>{record.action}</code>{stateKey && <span className="audit-state-badge" data-state={stateKey}>{t(stateKey)}</span>}</div></td>
    <td data-label={t("auditActor")}><AuditActor record={record} /></td>
    <td data-label={t("auditScopeObject")}><AuditTarget record={record} /></td>
    <td data-label={t("auditTime")}><AuditTime record={record} locale={locale} /></td>
  </tr>;
}

function AuditActor({ record }: { record: AuditRecord }) {
  const { t } = useI18n();
  if (!record.actor_user_id) return <span className="audit-system-badge">{t("systemActor")}</span>;
  return <div className="audit-actor"><strong>{record.actor_display_name ?? t("deletedActor")}</strong><code title={`${t("auditTechnicalId")}: ${record.actor_user_id}`}>{shortId(record.actor_user_id)}</code></div>;
}

function AuditTarget({ record }: { record: AuditRecord }) {
  const { t } = useI18n();
  if (record.workspace_id) {
    return <div className="audit-target"><span className="audit-scope-badge">{t("scopeWorkspace")}</span><strong>{record.workspace_name ?? t("auditDeletedWorkspace")}</strong>{record.workspace_short_id && <code>{record.workspace_short_id}</code>}<code title={`${t("auditTechnicalId")}: ${record.workspace_id}`}>{shortId(record.workspace_id)}</code></div>;
  }
  if (record.organization_id) {
    return <div className="audit-target"><span className="audit-scope-badge">{t("scopeOrganization")}</span><strong>{t("currentOrganization")}</strong><code title={`${t("auditTechnicalId")}: ${record.organization_id}`}>{shortId(record.organization_id)}</code></div>;
  }
  return <div className="audit-target"><span className="audit-scope-badge">{t("auditGlobalScope")}</span><strong>{t("auditPlatformObject")}</strong></div>;
}

function AuditTime({ record, locale }: { record: AuditRecord; locale: string }) {
  const { t } = useI18n();
  return <div className="audit-time"><time dateTime={new Date(record.created_at * 1_000).toISOString()}>{new Date(record.created_at * 1_000).toLocaleString(locale)}</time>{Object.keys(record.metadata).length > 0 && <details><summary>{t("auditDetails")}</summary><pre>{JSON.stringify(record.metadata, null, 2)}</pre></details>}</div>;
}

function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 8)}…` : value;
}
