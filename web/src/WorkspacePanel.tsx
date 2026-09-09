import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import { hasApiKeyScope } from "./permissions";
import { WorkspaceCard } from "./WorkspaceCard";
import { previousWorkspaceCursor } from "./workspacePaging";
import { reserveWebShellWindow } from "./workspaceShell";
import { aggregateRuntimeUsage } from "./workspaceMetrics";
import { WorkspaceCreationForm } from "./workspaces/WorkspaceCreationForm";
import { WorkspaceStats } from "./workspaces/WorkspaceStats";
import "./workspace-ui.css";
import type { CreateWorkspace, OrganizationUsageSummary, Principal, Resources, StoredInjection, WorkspaceResponse, WorkspaceRuntime, WorkspaceSummary, WorkspaceTemplate } from "./types";

const PAGE_SIZE = 50;
const EMPTY_WORKSPACES: WorkspaceResponse[] = [];
type Action = "start" | "stop" | "restart" | "delete";
type PendingAction = { item: WorkspaceResponse; action: Action } | null;
type WorkspacePageResult = { items: WorkspaceResponse[]; next_cursor: string | null; summary?: WorkspaceSummary };

interface Props {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  workspaces: WorkspaceResponse[];
  busy: boolean;
  onRefresh: () => Promise<void>;
  onError: (message: string) => void;
}

export function WorkspacePanel(props: Props) {
  const { t } = useI18n();
  const [showCreate, setShowCreate] = useState(false);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [templateId, setTemplateId] = useState("");
  const [name, setName] = useState("");
  const [resourceDraft, setResourceDraft] = useState<Resources | null>(null);
  const [organizationInjections, setOrganizationInjections] = useState<StoredInjection[]>([]);
  const [userInjections, setUserInjections] = useState<StoredInjection[]>([]);
  const [explicitInjectionRefs, setExplicitInjectionRefs] = useState(false);
  const [organizationRefs, setOrganizationRefs] = useState<string[]>([]);
  const [userRefs, setUserRefs] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [runtime, setRuntime] = useState<Record<string, WorkspaceRuntime>>({});
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [items, setItems] = useState<WorkspaceResponse[]>([]);
  const [summary, setSummary] = useState<WorkspaceSummary | null>(null);
  const [quota, setQuota] = useState<Resources | null>(null);
  const [usageSummary, setUsageSummary] = useState<OrganizationUsageSummary | null>(null);
  const [usageSummaryOrganization, setUsageSummaryOrganization] = useState<string | null>(null);
  const [usageSummaryStale, setUsageSummaryStale] = useState(false);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [history, setHistory] = useState<(string | null)[]>([null]);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [loadedScope, setLoadedScope] = useState<string | null>(null);
  const [runtimeFailed, setRuntimeFailed] = useState(false);
  const [runtimeRetry, setRuntimeRetry] = useState(0);
  const [pendingAction, setPendingAction] = useState<PendingAction>(null);
  const [pendingCreate, setPendingCreate] = useState<CreateWorkspace | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const generation = useRef(0);
  const requestActive = useRef(false);
  const cursorRef = useRef<string | null>(null);
  const nextCursorRef = useRef<string | null>(null);
  const activeIdsRef = useRef<string[]>([]);
  const listAnchorRef = useRef<HTMLDivElement>(null);
  const runtimeGeneration = useRef(0);
  const usageGeneration = useRef(0);
  const usageRequestActive = useRef(false);
  const scope = `${props.organizationId}\u0000${debouncedSearch.trim()}`;
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const scopeIsCurrent = loadedScope === scope;
  const workspaces = scopeIsCurrent ? items : EMPTY_WORKSPACES;
  const runtimeKey = workspaces.map(({ workspace }) => `${workspace.id}:${workspace.state}`).join(",");
  activeIdsRef.current = workspaces.map(({ workspace }) => workspace.id);
  const selectedTemplate = templates.find((template) => template.id === templateId);
  const canCreate = hasApiKeyScope(props.principal, "create_workspace");
  const canConnect = hasApiKeyScope(props.principal, "connect_workspace");
  const canChangeState = hasApiKeyScope(props.principal, "change_workspace_state");
  const canDelete = hasApiKeyScope(props.principal, "delete_workspace");

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedSearch(search), 250);
    return () => window.clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    const nextGeneration = ++generation.current;
    requestActive.current = true;
    setLoading(true); setItems([]); setSummary(null); setRuntime({}); setNextCursor(null); setHistory([null]); setPage(1); setLoadedScope(null); setRuntimeFailed(false);
    void fetchPage(null, scope, nextGeneration).finally(() => finishRequest(nextGeneration, scope));
  }, [props.api, props.organizationId, debouncedSearch]);

  useEffect(() => {
    let active = true;
    void props.api.quota(props.organizationId).then((value) => active && setQuota(value)).catch(() => active && setQuota(null));
    return () => { active = false; };
  }, [props.api, props.organizationId]);

  useEffect(() => {
    const requestGeneration = ++usageGeneration.current;
    usageRequestActive.current = false;
    setUsageSummary(null); setUsageSummaryOrganization(null); setUsageSummaryStale(false);
    const refresh = async () => {
      if (usageRequestActive.current || document.visibilityState !== "visible") return;
      usageRequestActive.current = true;
      try {
        const value = await props.api.usageSummary(props.organizationId);
        if (usageGeneration.current === requestGeneration) { setUsageSummary(value); setUsageSummaryOrganization(props.organizationId); setUsageSummaryStale(false); }
      } catch {
        // Keep the last organization observation visible; an absent metric remains unknown, never zero.
        if (usageGeneration.current === requestGeneration) setUsageSummaryStale(true);
      } finally {
        if (usageGeneration.current === requestGeneration) usageRequestActive.current = false;
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 30_000);
    return () => { ++usageGeneration.current; window.clearInterval(timer); };
  }, [props.api, props.organizationId]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible" && scopeIsCurrent && !requestActive.current) void loadPage(cursorRef.current, scope, true);
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [scopeIsCurrent]);

  useEffect(() => {
    let active = true;
    const nextGeneration = ++runtimeGeneration.current;
    const refresh = async () => {
      if (document.visibilityState !== "visible" || !scopeIsCurrent || workspaces.length === 0) return;
      try {
        const batches = workspaceIdBatches(workspaces.map(({ workspace }) => workspace.id));
        const responses = await Promise.all(batches.map((workspaceIds) => props.api.workspaceRuntimes(props.organizationId, workspaceIds)));
        if (!active || runtimeGeneration.current !== nextGeneration) return;
        setRuntime((current) => Object.fromEntries(responses.flat().map((entry) => [entry.workspace_id, { ...entry.runtime, events: current[entry.workspace_id]?.events ?? [] }])));
        setRuntimeFailed(false);
      } catch { if (active && runtimeGeneration.current === nextGeneration) setRuntimeFailed(true); }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 30_000);
    return () => { active = false; window.clearInterval(timer); };
  }, [props.api, props.organizationId, runtimeKey, scopeIsCurrent, runtimeRetry]);

  useEffect(() => {
    let active = true;
    void props.api.templates(props.organizationId).then((value) => active && setTemplates(value.filter(({ enabled }) => enabled))).catch((error) => props.onError(errorMessage(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId, props.onError]);

  useEffect(() => {
    let active = true;
    void Promise.all([props.api.injections("organization", props.organizationId), props.api.injections("user", props.principal.user_id)])
      .then(([organization, user]) => {
        if (!active) return;
        setOrganizationInjections(organization); setUserInjections(user); setExplicitInjectionRefs(false); setOrganizationRefs([]); setUserRefs([]);
      }).catch((error) => props.onError(errorMessage(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId, props.principal.user_id, props.onError]);

  async function fetchPage(cursor: string | null, requestScope: string, requestGeneration: number) {
    try {
      const result = await props.api.workspacesPage(props.organizationId, { limit: PAGE_SIZE, cursor: cursor ?? undefined, search: debouncedSearch.trim() || undefined }) as WorkspacePageResult;
      if (generation.current !== requestGeneration || scopeRef.current !== requestScope) return false;
      cursorRef.current = cursor; nextCursorRef.current = result.next_cursor;
      setItems(result.items); setSummary(result.summary ?? null); setNextCursor(result.next_cursor); setLoadedScope(requestScope);
      return true;
    } catch (error) {
      if (generation.current === requestGeneration && scopeRef.current === requestScope) props.onError(errorMessage(error));
      return false;
    }
  }

  function finishRequest(requestGeneration: number, requestScope: string, scroll = false) {
    if (generation.current !== requestGeneration || scopeRef.current !== requestScope) return;
    requestActive.current = false; setLoading(false);
    if (scroll) listAnchorRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  async function loadPage(cursor: string | null, requestScope: string, preserveRuntime = false, scroll = false) {
    if (requestActive.current || scopeRef.current !== requestScope) return false;
    const nextGeneration = ++generation.current;
    requestActive.current = true; setLoading(true);
    if (!preserveRuntime) setRuntime({});
    try { return await fetchPage(cursor, requestScope, nextGeneration); }
    finally { finishRequest(nextGeneration, requestScope, scroll); }
  }

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!templateId) return props.onError(t("chooseTemplateError"));
    const command: CreateWorkspace = {
      organization_id: props.organizationId, owner_id: props.principal.user_id, name, template_id: templateId,
      resources: resourceDraft && selectedTemplate && !sameResources(resourceDraft, selectedTemplate.resources) ? resourceDraft : null,
      organization_injection_refs: explicitInjectionRefs ? organizationInjections.filter((item) => item.locked || organizationRefs.includes(item.key)).map((item) => item.key) : null,
      user_injection_refs: explicitInjectionRefs ? userRefs : null,
    };
    if (selectedTemplate?.cluster_access) {
      setPendingCreate(command);
      return;
    }
    await submitCreate(command);
  }

  async function submitCreate(command: CreateWorkspace) {
    setSubmitting(true);
    try {
      await props.api.createWorkspace(command);
      setName(""); setTemplateId(""); setResourceDraft(null); setShowCreate(false); setPendingCreate(null);
      await props.onRefresh(); await resetPage();
    } catch (error) { props.onError(errorMessage(error)); }
    finally { setSubmitting(false); }
  }

  function chooseTemplate(id: string) {
    setTemplateId(id);
    const template = templates.find((item) => item.id === id);
    setResourceDraft(template ? { ...template.resources } : null);
  }

  function updateResource(key: keyof Resources, value: string) {
    const parsed = Number(value);
    setResourceDraft((current) => current ? { ...current, [key]: Number.isFinite(parsed) ? Math.max(0, Math.trunc(parsed)) : 0 } : current);
  }

  function setReferenceMode(explicit: boolean) {
    setExplicitInjectionRefs(explicit);
    if (explicit) { setOrganizationRefs([]); setUserRefs([]); }
  }

  function requestAction(item: WorkspaceResponse, action: Action) {
    if (action === "start" || ((action === "stop" || action === "restart") && currentCpu(runtime[item.workspace.id]) === 0)) return void executeAction(item, action);
    setPendingAction({ item, action });
  }

  async function executeAction(item: WorkspaceResponse, action: Action) {
    setActionBusy(true);
    try {
      await props.api.workspaceAction(item.workspace.id, action);
      await props.onRefresh();
      await loadPage(cursorRef.current, scopeRef.current);
      setPendingAction(null);
    } catch (error) { props.onError(errorMessage(error)); }
    finally { setActionBusy(false); }
  }

  async function openShell(workspaceId: string) {
    const target = reserveWebShellWindow();
    try {
      const ticket = await props.api.issueWebShellTicket(workspaceId);
      if (target) target.location.href = ticket.web_shell_url; else window.location.href = ticket.web_shell_url;
    } catch (error) { target?.close(); props.onError(errorMessage(error)); }
  }

  async function refreshRuntime(workspaceId: string) {
    try {
      const observation = await props.api.workspaceRuntime(workspaceId);
      if (scopeRef.current !== scope || !activeIdsRef.current.includes(workspaceId)) return;
      setRuntime((current) => ({ ...current, [workspaceId]: observation })); setRuntimeFailed(false);
    } catch (error) { if (scopeRef.current === scope) { setRuntimeFailed(true); props.onError(errorMessage(error)); } }
  }

  async function resetPage() {
    setHistory([null]); setPage(1); cursorRef.current = null;
    await loadPage(null, scope, false, true);
  }

  async function nextPage() {
    const cursor = nextCursorRef.current;
    if (!cursor || loading || requestActive.current || !scopeIsCurrent) return;
    if (await loadPage(cursor, scope, false, true)) { setHistory((current) => [...current, cursor]); setPage((current) => current + 1); }
  }

  async function previousPage() {
    if (page <= 1 || loading || requestActive.current || !scopeIsCurrent) return;
    const cursor = previousWorkspaceCursor(history, page);
    if (await loadPage(cursor, scope, false, true)) { setHistory((current) => current.slice(0, -1)); setPage((current) => current - 1); }
  }

  const actionDescription = pendingAction?.action === "delete"
    ? t("deleteWorkspaceConfirm")
    : pendingAction?.action === "stop"
      ? t("stopWorkspaceConfirm")
      : pendingAction?.action === "restart"
        ? t("restartWorkspaceConfirm")
        : "";
  return <section className="panel-stack workspace-panel">
    <WorkspaceStats summary={usageSummaryOrganization === props.organizationId ? usageSummary : null} quota={quota} stale={usageSummaryStale} />
    <div className="section-heading"><h2>{t("workspaces")}</h2>{canCreate && <button className="button primary" onClick={() => setShowCreate((current) => !current)}>{showCreate ? t("collapse") : t("newWorkspace")}</button>}</div>
    {canCreate && showCreate && <WorkspaceCreationForm name={name} templateId={templateId} templates={templates} resourceDraft={resourceDraft} explicitInjectionRefs={explicitInjectionRefs} organizationInjections={organizationInjections} userInjections={userInjections} organizationRefs={organizationRefs} userRefs={userRefs} submitting={submitting} onNameChange={setName} onTemplateChange={chooseTemplate} onResourceChange={updateResource} onReferenceModeChange={setReferenceMode} onOrganizationRefsChange={setOrganizationRefs} onUserRefsChange={setUserRefs} onSubmit={create} />}
    <label className="workspace-search">{t("searchWorkspaces")}<input type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchWorkspacesHint")} /></label>
    {runtimeFailed && workspaces.length > 0 && <div className="empty workspace-runtime-notice" role="status">{t("runtimeDataUnavailable")} <button onClick={() => setRuntimeRetry((value) => value + 1)}>{t("retryRuntime")}</button></div>}
    <div className="workspace-pagination" aria-label={t("workspacePagination")}><button className="button" type="button" disabled={page <= 1 || loading} onClick={() => void previousPage()}>{t("previousPage")}</button><span role="status">{t("workspacePageStatus")} {page}</span><button className="button" type="button" disabled={!nextCursor || loading} onClick={() => void nextPage()}>{t("nextPage")}</button></div>
    <div ref={listAnchorRef} className="workspace-list" aria-busy={props.busy || loading}>
      {workspaces.length === 0 && loading && <div className="empty" role="status">{t("loadingWorkspaces")}</div>}
      {workspaces.length === 0 && !loading && scopeIsCurrent && <div className="empty">{t("noWorkspaces")}</div>}
      {workspaces.map((item) => <WorkspaceCard key={item.workspace.id} api={props.api} item={item} runtime={runtime[item.workspace.id]} onAction={requestAction} onOpenShell={openShell} onRequestRuntime={refreshRuntime} onError={props.onError} canConnect={canConnect} canChangeState={canChangeState} canDelete={canDelete} />)}
    </div>
    <ConfirmDialog open={pendingAction !== null} title={pendingAction ? t(pendingAction.action) : ""} description={actionDescription} confirmLabel={pendingAction ? t(pendingAction.action) : ""} cancelLabel={t("cancel")} busy={actionBusy} danger={pendingAction?.action === "delete"} details={pendingAction && <code>{pendingAction.item.workspace.name}</code>} onClose={() => !actionBusy && setPendingAction(null)} onConfirm={() => pendingAction && void executeAction(pendingAction.item, pendingAction.action)} />
    <ConfirmDialog open={pendingCreate !== null} title={t("submitCreate")} description={t("workspaceHighRiskConfirm")} confirmLabel={t("submitCreate")} cancelLabel={t("cancel")} busy={submitting} danger details={pendingCreate && <code>{pendingCreate.name}</code>} onClose={() => !submitting && setPendingCreate(null)} onConfirm={() => pendingCreate && void submitCreate(pendingCreate)} />
  </section>;
}

function currentCpu(runtime: WorkspaceRuntime | undefined) { return runtime ? aggregateRuntimeUsage(runtime).cpuMillis : null; }
function workspaceIdBatches(ids: string[]) { return Array.from({ length: Math.ceil(ids.length / 100) }, (_, index) => ids.slice(index * 100, index * 100 + 100)); }
function sameResources(left: Resources, right: Resources) { return left.cpu_millis === right.cpu_millis && left.memory_mib === right.memory_mib && left.disk_gib === right.disk_gib && left.gpu_count === right.gpu_count; }
function errorMessage(error: unknown) { return error instanceof Error ? error.message : "操作失败"; }
