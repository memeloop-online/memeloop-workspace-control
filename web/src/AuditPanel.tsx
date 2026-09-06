import { useCallback, useEffect, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { AuditRow } from "./audit/AuditRow";
import { useI18n } from "./i18n";
import type { AuditRecord } from "./types";

interface AuditFilters {
  action: string;
  actor: string;
  workspace: string;
  q: string;
}

const EMPTY_FILTERS: AuditFilters = { action: "", actor: "", workspace: "", q: "" };

export function AuditPanel({ api, organizationId, systemAdmin, onError }: { api: ApiClient; organizationId: string; systemAdmin: boolean; onError: (message: string) => void }) {
  const { locale, t } = useI18n();
  const [records, setRecords] = useState<AuditRecord[]>([]);
  const [draft, setDraft] = useState<AuditFilters>(EMPTY_FILTERS);
  const [filters, setFilters] = useState<AuditFilters>(EMPTY_FILTERS);
  const [limit, setLimit] = useState(25);
  const [offset, setOffset] = useState(0);
  const [offsetHistory, setOffsetHistory] = useState<number[]>([]);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [scope, setScope] = useState<"organization" | "all">(systemAdmin ? "all" : "organization");

  const load = useCallback(async (next: number, applied: AuditFilters, pageLimit: number) => {
    setLoading(true);
    try {
      const page = await api.audit(scope === "all" ? undefined : organizationId, { ...applied, limit: pageLimit, offset: next });
      setRecords(page.items);
      setOffset(next);
      setNextOffset(page.next_offset);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setLoading(false);
    }
  }, [api, organizationId, onError, scope, t]);

  useEffect(() => {
    setOffsetHistory([]);
    void load(0, filters, limit);
  }, [filters, limit, load]);

  function applyFilters(event: FormEvent) {
    event.preventDefault();
    setFilters(draft);
  }

  function clearFilters() {
    setDraft(EMPTY_FILTERS);
    setFilters(EMPTY_FILTERS);
  }

  function nextPage() {
    if (nextOffset === null) return;
    setOffsetHistory((history) => [...history, offset]);
    void load(nextOffset, filters, limit);
  }

  function previousPage() {
    const previous = offsetHistory.at(-1);
    if (previous === undefined) return;
    setOffsetHistory((history) => history.slice(0, -1));
    void load(previous, filters, limit);
  }

  function changeLimit(value: number) {
    setLimit(value);
  }

  return <section className="panel-stack audit-page">
    <div className="section-heading"><div><p className="eyebrow">AUDIT</p><h2>{t("auditTitle")}</h2></div></div>
    <form className={`audit-filters${systemAdmin ? " with-scope" : ""}`} onSubmit={applyFilters}>
      <label>{t("auditSearch")}<input type="search" value={draft.q} onChange={(event) => setDraft({ ...draft, q: event.target.value })} placeholder={t("auditSearchHint")} /></label>
      <label>{t("auditAction")}<input value={draft.action} onChange={(event) => setDraft({ ...draft, action: event.target.value })} placeholder="workspace.create" /></label>
      <label>{t("auditActor")}<input value={draft.actor} onChange={(event) => setDraft({ ...draft, actor: event.target.value })} placeholder={t("auditActorHint")} /></label>
      <label>{t("auditWorkspace")}<input value={draft.workspace} onChange={(event) => setDraft({ ...draft, workspace: event.target.value })} placeholder={t("auditWorkspaceHint")} /></label>
      {systemAdmin && <label>{t("auditLogScope")}<select value={scope} onChange={(event) => setScope(event.target.value as "organization" | "all")}><option value="all">{t("auditAllScopes")}</option><option value="organization" disabled={!organizationId}>{t("auditCurrentOrganizationScope")}</option></select></label>}
      <div className="audit-filter-actions"><button className="button primary" disabled={loading}>{t("applyFilters")}</button><button className="button" type="button" onClick={clearFilters}>{t("clearFilters")}</button></div>
    </form>
    <section className="audit-card" aria-busy={loading}>
      {records.length ? <div className="audit-table-frame"><table className="audit-table"><caption className="visually-hidden">{t("auditTableCaption")}</caption><thead><tr><th scope="col">{t("auditAction")}</th><th scope="col">{t("auditActor")}</th><th scope="col">{t("auditScopeObject")}</th><th scope="col">{t("auditTime")}</th></tr></thead><tbody>{records.map((record) => <AuditRow key={record.id} record={record} locale={locale} />)}</tbody></table></div> : <p className="empty compact">{loading ? t("loading") : t("noAudit")}</p>}
      <footer className="audit-pagination"><label>{t("rowsPerPage")}<select value={limit} onChange={(event) => changeLimit(Number(event.target.value))}><option value="10">10</option><option value="25">25</option><option value="50">50</option><option value="100">100</option></select></label><span>{t("page")} {offsetHistory.length + 1}</span><button className="button" disabled={loading || offsetHistory.length === 0} onClick={previousPage}>{t("previousPage")}</button><button className="button" disabled={loading || nextOffset === null} onClick={nextPage}>{t("nextPage")}</button></footer>
    </section>
  </section>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
