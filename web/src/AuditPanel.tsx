import {
  Button,
  DataGrid,
  DataGridBody,
  DataGridHeader,
  DataGridHeaderCell,
  DataGridRow,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Option,
  Select,
  Spinner,
  Toolbar,
  ToolbarButton,
  createTableColumn,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import type { FormEvent, ReactNode } from "react";
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

const useStyles = makeStyles({
  page: { display: "grid", gap: tokens.spacingVerticalL, minWidth: 0 },
  heading: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: tokens.spacingHorizontalM, flexWrap: "wrap" },
  title: { margin: 0, fontSize: tokens.fontSizeBase600, fontWeight: tokens.fontWeightSemibold },
  filters: {
    display: "grid",
    gridTemplateColumns: "minmax(220px, 2fr) repeat(3, minmax(150px, 1fr))",
    gap: tokens.spacingHorizontalM,
    alignItems: "end",
    padding: tokens.spacingVerticalM,
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    backgroundColor: tokens.colorNeutralBackground1,
    "@media (max-width: 900px)": { gridTemplateColumns: "repeat(2, minmax(0, 1fr))" },
    "@media (max-width: 620px)": { gridTemplateColumns: "1fr" },
  },
  filterActions: { display: "flex", gap: tokens.spacingHorizontalS, flexWrap: "wrap", alignItems: "center" },
  gridCard: {
    minWidth: 0,
    overflowX: "auto",
    padding: tokens.spacingVerticalM,
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    backgroundColor: tokens.colorNeutralBackground1,
  },
  grid: { minWidth: "760px" },
  loading: { display: "flex", alignItems: "center", justifyContent: "center", minHeight: "180px", gap: tokens.spacingHorizontalS },
  empty: { display: "grid", placeItems: "center", minHeight: "180px", color: tokens.colorNeutralForeground2 },
  pagination: { display: "flex", alignItems: "center", justifyContent: "flex-end", gap: tokens.spacingHorizontalS, flexWrap: "wrap", marginTop: tokens.spacingVerticalM },
  pageText: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  detailList: { display: "grid", gridTemplateColumns: "minmax(120px, .35fr) minmax(0, 1fr)", gap: tokens.spacingVerticalS, margin: 0 },
  detailTerm: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  detailDescription: { minWidth: 0, margin: 0, overflowWrap: "anywhere", fontSize: tokens.fontSizeBase300 },
});

export function AuditPanel({ api, organizationId, systemAdmin, onError }: { api: ApiClient; organizationId: string; systemAdmin: boolean; onError: (message: string) => void }) {
  const styles = useStyles();
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
  const [details, setDetails] = useState<AuditRecord | null>(null);

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
    setFilters({ ...draft });
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

  const columns = useMemo(() => [
    createTableColumn<AuditRecord>({ columnId: "action", renderHeaderCell: () => t("auditAction"), renderCell: () => null }),
    createTableColumn<AuditRecord>({ columnId: "actor", renderHeaderCell: () => t("auditActor"), renderCell: () => null }),
    createTableColumn<AuditRecord>({ columnId: "target", renderHeaderCell: () => t("auditScopeObject"), renderCell: () => null }),
    createTableColumn<AuditRecord>({ columnId: "time", renderHeaderCell: () => t("auditTime"), renderCell: () => null }),
  ], [t]);

  const rowCells = useMemo<readonly ((record: AuditRecord) => ReactNode)[]>(() => [
    (record) => <AuditRow.Action record={record} />,
    (record) => <AuditRow.Actor record={record} />,
    (record) => <AuditRow.Target record={record} />,
    (record) => <AuditRow.Time record={record} locale={locale} onDetails={() => setDetails(record)} />,
  ], [locale]);

  return <section className={styles.page} aria-labelledby="audit-page-title">
      <div className={styles.heading}><h2 id="audit-page-title" className={styles.title}>{t("auditTitle")}</h2></div>
      <form className={styles.filters} onSubmit={applyFilters}>
        <Field label={t("auditSearch")}>
          <Input type="search" value={draft.q} onChange={(_, data) => setDraft({ ...draft, q: data.value })} placeholder={t("auditSearchHint")} />
        </Field>
        <Field label={t("auditAction")}>
          <Input value={draft.action} onChange={(_, data) => setDraft({ ...draft, action: data.value })} placeholder="workspace.create" />
        </Field>
        <Field label={t("auditActor")}>
          <Input value={draft.actor} onChange={(_, data) => setDraft({ ...draft, actor: data.value })} placeholder={t("auditActorHint")} />
        </Field>
        <Field label={t("auditWorkspace")}>
          <Input value={draft.workspace} onChange={(_, data) => setDraft({ ...draft, workspace: data.value })} placeholder={t("auditWorkspaceHint")} />
        </Field>
        {systemAdmin && <Field label={t("auditLogScope")}>
          <Select value={scope} onChange={(_, data) => setScope(data.value as "organization" | "all")}>
            <Option value="all" text={t("auditAllScopes")}>{t("auditAllScopes")}</Option>
            <Option value="organization" text={t("auditCurrentOrganizationScope")}>{t("auditCurrentOrganizationScope")}</Option>
          </Select>
        </Field>}
        <div className={styles.filterActions}>
          <Button appearance="primary" type="submit" disabled={loading}>{t("applyFilters")}</Button>
          <Button type="button" onClick={clearFilters}>{t("clearFilters")}</Button>
        </div>
      </form>
      {loading && records.length === 0 && <div className={styles.gridCard}><div className={styles.loading} role="status"><Spinner size="tiny" />{t("loading")}</div></div>}
      {!loading && records.length === 0 && <div className={styles.gridCard}><div className={styles.empty}>{t("noAudit")}</div></div>}
      {records.length > 0 && <div className={styles.gridCard} aria-busy={loading}>
        {loading && <MessageBar intent="info"><MessageBarBody>{t("loading")}</MessageBarBody></MessageBar>}
        <DataGrid className={styles.grid} items={records} columns={columns} getRowId={(record) => record.id} aria-label={t("auditTableCaption")}>
          <DataGridHeader>
            <DataGridRow>{({ renderHeaderCell }) => <DataGridHeaderCell>{renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow>
          </DataGridHeader>
          <DataGridBody<AuditRecord>>{({ item, rowId }) => <DataGridRow<AuditRecord> key={rowId}>{() => rowCells.map((renderCell, index) => <AuditRow.Cell key={index}>{renderCell(item)}</AuditRow.Cell>)}</DataGridRow>}</DataGridBody>
        </DataGrid>
        <div className={styles.pagination}>
          <Field label={t("rowsPerPage")} orientation="horizontal">
            <Select value={String(limit)} onChange={(_, data) => setLimit(Number(data.value))}>
              {[10, 25, 50, 100].map((value) => <Option key={value} value={String(value)} text={String(value)}>{value}</Option>)}
            </Select>
          </Field>
          <span className={styles.pageText}>{t("page")} {offsetHistory.length + 1}</span>
          <Toolbar aria-label={t("auditTitle")}>
            <ToolbarButton onClick={previousPage} disabled={loading || offsetHistory.length === 0}>{t("previousPage")}</ToolbarButton>
            <ToolbarButton onClick={nextPage} disabled={loading || nextOffset === null}>{t("nextPage")}</ToolbarButton>
          </Toolbar>
        </div>
      </div>}
      <AuditDetailsDialog record={details} locale={locale} onClose={() => setDetails(null)} />
    </section>;
}

function AuditDetailsDialog({ record, locale, onClose }: { record: AuditRecord | null; locale: string; onClose: () => void }) {
  const { t } = useI18n();
  const styles = useStyles();
  const actor = record?.actor_display_name?.trim() || (record?.actor_user_id ? t("deletedActor") : t("systemActor"));
  return <Dialog open={record !== null} onOpenChange={(_, data) => !data.open && onClose()}>
    <DialogSurface>
      <DialogBody>
        <DialogTitle>{t("auditDetails")}</DialogTitle>
        {record && <DialogContent>
          <dl className={styles.detailList}>
            <dt className={styles.detailTerm}>{t("auditAction")}</dt><dd className={styles.detailDescription}><code>{record.action}</code></dd>
            <dt className={styles.detailTerm}>{t("auditActor")}</dt><dd className={styles.detailDescription}>{actor}</dd>
            <dt className={styles.detailTerm}>{t("auditTime")}</dt><dd className={styles.detailDescription}>{new Date(record.created_at * 1_000).toLocaleString(locale)}</dd>
            <dt className={styles.detailTerm}>{t("auditTechnicalId")}</dt><dd className={styles.detailDescription}><code>{record.id}</code></dd>
          </dl>
          {Object.keys(record.metadata).length > 0 && <dl className={styles.detailList}>
            {Object.entries(record.metadata).map(([key, value]) => (
              <Fragment key={key}>
                <dt className={styles.detailTerm}><code>{key}</code></dt>
                <dd className={styles.detailDescription}>{metadataText(value)}</dd>
              </Fragment>
            ))}
          </dl>}
        </DialogContent>}
        <DialogActions><Button appearance="primary" onClick={onClose}>{t("close")}</Button></DialogActions>
      </DialogBody>
    </DialogSurface>
  </Dialog>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function metadataText(value: unknown): string {
  if (value === null) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}
