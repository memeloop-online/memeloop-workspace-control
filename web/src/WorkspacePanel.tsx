import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { CredentialReferencePicker } from "./forms/CredentialReferencePicker";
import { useI18n } from "./i18n";
import { hasApiKeyScope } from "./permissions";
import { WorkspaceCard } from "./WorkspaceCard";
import { reserveWebShellWindow } from "./workspaceShell";
import { previousWorkspaceCursor } from "./workspacePaging";
import type {
  CreateWorkspace,
  Principal,
  Resources,
  StoredInjection,
  WorkspaceResponse,
  WorkspaceRuntime,
  WorkspaceTemplate,
} from "./types";

const EMPTY_WORKSPACE_PAGE: WorkspaceResponse[] = [];
const WORKSPACE_PAGE_SIZE = 50;

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
  const { locale, t } = useI18n();
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
  const [workspaceSearch, setWorkspaceSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [pagedWorkspaces, setPagedWorkspaces] = useState<WorkspaceResponse[]>([]);
  const [nextWorkspaceCursor, setNextWorkspaceCursor] = useState<string | null>(null);
  const [workspaceCursorHistory, setWorkspaceCursorHistory] = useState<(string | null)[]>([null]);
  const [workspacePageNumber, setWorkspacePageNumber] = useState(1);
  const [pageLoading, setPageLoading] = useState(false);
  const [loadedWorkspaceScope, setLoadedWorkspaceScope] = useState<string | null>(null);
  const [runtimeLoadFailed, setRuntimeLoadFailed] = useState(false);
  const [runtimeRetryToken, setRuntimeRetryToken] = useState(0);
  const pageRequestGeneration = useRef(0);
  const pageRequestActive = useRef(false);
  const currentWorkspaceCursor = useRef<string | null>(null);
  const nextWorkspaceCursorRef = useRef<string | null>(null);
  const currentWorkspaceIdsRef = useRef<string[]>([]);
  const workspaceListRef = useRef<HTMLDivElement | null>(null);
  const runtimeRequestGeneration = useRef(0);
  const runtimeKey = pagedWorkspaces.map((item) => `${item.workspace.id}:${item.workspace.state}`).join(",");
  const workspaceScope = `${props.organizationId}\u0000${debouncedSearch.trim()}`;
  const workspaceScopeRef = useRef(workspaceScope);
  workspaceScopeRef.current = workspaceScope;
  const showingCurrentWorkspaceScope = loadedWorkspaceScope === workspaceScope;
  const currentPagedWorkspaces = showingCurrentWorkspaceScope ? pagedWorkspaces : EMPTY_WORKSPACE_PAGE;
  const canCreateWorkspace = hasApiKeyScope(props.principal, "create_workspace");
  const canConnectWorkspace = hasApiKeyScope(props.principal, "connect_workspace");
  const canChangeWorkspaceState = hasApiKeyScope(props.principal, "change_workspace_state");
  const canDeleteWorkspace = hasApiKeyScope(props.principal, "delete_workspace");
  currentWorkspaceIdsRef.current = currentPagedWorkspaces.map((item) => item.workspace.id);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedSearch(workspaceSearch), 250);
    return () => window.clearTimeout(timer);
  }, [workspaceSearch]);

  useEffect(() => {
    const generation = ++pageRequestGeneration.current;
    pageRequestActive.current = true;
    setPageLoading(true);
    setPagedWorkspaces([]);
    setNextWorkspaceCursor(null);
    nextWorkspaceCursorRef.current = null;
    currentWorkspaceCursor.current = null;
    setWorkspaceCursorHistory([null]);
    setWorkspacePageNumber(1);
    setLoadedWorkspaceScope(null);
    setRuntime({});
    setRuntimeLoadFailed(false);
    props.api.workspacesPage(props.organizationId, { limit: WORKSPACE_PAGE_SIZE, search: debouncedSearch.trim() || undefined })
      .then((page) => {
        if (pageRequestGeneration.current !== generation || workspaceScopeRef.current !== workspaceScope) return;
        setPagedWorkspaces(page.items);
        setNextWorkspaceCursor(page.next_cursor);
        nextWorkspaceCursorRef.current = page.next_cursor;
        setLoadedWorkspaceScope(workspaceScope);
      })
      .catch((error) => {
        if (pageRequestGeneration.current === generation && workspaceScopeRef.current === workspaceScope) props.onError(message(error));
      })
      .finally(() => {
        if (pageRequestGeneration.current === generation && workspaceScopeRef.current === workspaceScope) {
          pageRequestActive.current = false;
          setPageLoading(false);
        }
      });
  }, [props.api, props.organizationId, debouncedSearch, props.onError]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "visible" || !showingCurrentWorkspaceScope || pageRequestActive.current) return;
      void loadWorkspacePage(currentWorkspaceCursor.current, workspaceScopeRef.current, { preserveRuntime: true });
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [props.api, props.organizationId, debouncedSearch, showingCurrentWorkspaceScope]);

  useEffect(() => {
    let active = true;
    const generation = ++runtimeRequestGeneration.current;
    const refreshRuntime = async () => {
      if (document.visibilityState !== "visible" || !showingCurrentWorkspaceScope || currentPagedWorkspaces.length === 0) return;
      try {
        const batches = workspaceIdBatches(currentPagedWorkspaces.map((item) => item.workspace.id));
        const responses = await Promise.all(batches.map((workspaceIds) => props.api.workspaceRuntimes(props.organizationId, workspaceIds)));
        if (!active || runtimeRequestGeneration.current !== generation) return;
        const entries = responses.flat();
        setRuntime((current) => Object.fromEntries(entries.map((entry) => [
            entry.workspace_id,
            { ...entry.runtime, events: current[entry.workspace_id]?.events ?? [] },
          ])));
        setRuntimeLoadFailed(false);
      } catch {
        if (active && runtimeRequestGeneration.current === generation) setRuntimeLoadFailed(true);
      }
    };
    void refreshRuntime();
    const timer = window.setInterval(() => void refreshRuntime(), 30000);
    return () => { active = false; window.clearInterval(timer); };
  }, [props.api, props.organizationId, runtimeKey, showingCurrentWorkspaceScope, runtimeRetryToken]);

  useEffect(() => {
    let active = true;
    props.api
      .templates(props.organizationId)
      .then((items) => active && setTemplates(items.filter((item) => item.enabled)))
      .catch((error) => props.onError(message(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId, props.onError]);

  useEffect(() => {
    let active = true;
    Promise.all([
      props.api.injections("organization", props.organizationId),
      props.api.injections("user", props.principal.user_id),
    ])
      .then(([organization, user]) => {
        if (!active) return;
        setOrganizationInjections(organization);
        setUserInjections(user);
        setExplicitInjectionRefs(false);
        setOrganizationRefs([]);
        setUserRefs([]);
      })
      .catch((error) => props.onError(message(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId, props.principal.user_id, props.onError]);

  const totals = useMemo(
    () =>
        currentPagedWorkspaces.reduce(
        (sum, item) => ({
          cpu: sum.cpu + item.workspace.resources.cpu_millis,
          memory: sum.memory + item.workspace.resources.memory_mib,
          disk: sum.disk + item.workspace.resources.disk_gib,
          gpu: sum.gpu + item.workspace.resources.gpu_count,
        }),
        { cpu: 0, memory: 0, disk: 0, gpu: 0 },
      ),
    [currentPagedWorkspaces],
  );
  const selectedTemplate = templates.find((template) => template.id === templateId);
  const filteredWorkspaces = useMemo(() => {
    const query = debouncedSearch.trim().toLocaleLowerCase();
    if (!query) return currentPagedWorkspaces;
    return currentPagedWorkspaces.filter(({ workspace }) => [
      workspace.name,
      workspace.short_id,
      workspace.state,
      workspace.image,
      workspace.workspace_user,
    ].some((value) => value.toLocaleLowerCase().includes(query)));
  }, [currentPagedWorkspaces, debouncedSearch]);
  const visibleWorkspaces = filteredWorkspaces;

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!templateId) {
      props.onError(t("chooseTemplateError"));
      return;
    }
    if (selectedTemplate?.cluster_access && !confirm(t("workspaceHighRiskConfirm"))) return;
    const command: CreateWorkspace = {
      organization_id: props.organizationId,
      owner_id: props.principal.user_id,
      name,
      template_id: templateId,
      resources: resourceDraft && selectedTemplate && !sameResources(resourceDraft, selectedTemplate.resources)
        ? resourceDraft
        : null,
      organization_injection_refs: explicitInjectionRefs
        ? organizationInjections
            .filter((item) => item.locked || organizationRefs.includes(item.key))
            .map((item) => item.key)
        : null,
      user_injection_refs: explicitInjectionRefs ? userRefs : null,
    };
    setSubmitting(true);
    try {
      await props.api.createWorkspace(command);
      setName("");
      setTemplateId("");
      setResourceDraft(null);
      setShowCreate(false);
      await props.onRefresh();
      await resetWorkspacePage();
    } catch (error) {
      props.onError(message(error));
    } finally {
      setSubmitting(false);
    }
  }

  function chooseTemplate(id: string) {
    setTemplateId(id);
    const template = templates.find((item) => item.id === id);
    setResourceDraft(template ? { ...template.resources } : null);
  }

  function updateResource(key: keyof Resources, value: string) {
    const parsed = Number(value);
    setResourceDraft((current) => current ? {
      ...current,
      [key]: Number.isFinite(parsed) ? Math.max(0, Math.trunc(parsed)) : 0,
    } : current);
  }

  function setReferenceMode(explicit: boolean) {
    setExplicitInjectionRefs(explicit);
    if (explicit) {
      setOrganizationRefs([]);
      setUserRefs([]);
    }
  }

  async function action(
    id: string,
    value: "start" | "stop" | "restart" | "delete",
  ) {
    if (value === "delete" && !confirm(t("deleteWorkspaceConfirm"))) {
      return;
    }
    try {
      await props.api.workspaceAction(id, value);
      await props.onRefresh();
      await loadWorkspacePage(currentWorkspaceCursor.current, workspaceScopeRef.current);
    } catch (error) {
      props.onError(message(error));
    }
  }

  async function openShell(workspaceId: string) {
    const target = reserveWebShellWindow();
    try {
      const ticket = await props.api.issueWebShellTicket(workspaceId);
      if (target) target.location.href = ticket.web_shell_url;
      else window.location.href = ticket.web_shell_url;
    } catch (error) {
      target?.close();
      props.onError(message(error));
    }
  }

  async function refreshWorkspaceRuntime(workspaceId: string) {
    const scope = workspaceScope;
    try {
      const observation = await props.api.workspaceRuntime(workspaceId);
      if (workspaceScopeRef.current !== scope || !currentWorkspaceIdsRef.current.includes(workspaceId)) return;
      setRuntime((current) => ({ ...current, [workspaceId]: observation }));
      setRuntimeLoadFailed(false);
    } catch (error) {
      if (workspaceScopeRef.current === scope) {
        setRuntimeLoadFailed(true);
        props.onError(message(error));
      }
    }
  }

  async function loadWorkspacePage(cursor: string | null, scope: string, options: { resetScroll?: boolean; preserveRuntime?: boolean } = {}) {
    if (pageRequestActive.current || workspaceScopeRef.current !== scope) return false;
    const generation = ++pageRequestGeneration.current;
    pageRequestActive.current = true;
    setPageLoading(true);
    if (!options.preserveRuntime) setRuntime({});
    try {
      const page = await props.api.workspacesPage(props.organizationId, { limit: WORKSPACE_PAGE_SIZE, cursor: cursor ?? undefined, search: debouncedSearch.trim() || undefined });
      if (pageRequestGeneration.current !== generation || scope !== workspaceScopeRef.current) return false;
      currentWorkspaceCursor.current = cursor;
      nextWorkspaceCursorRef.current = page.next_cursor;
      setPagedWorkspaces(page.items);
      setLoadedWorkspaceScope(scope);
      setNextWorkspaceCursor(page.next_cursor);
      if (options.resetScroll && workspaceListRef.current) workspaceListRef.current.scrollTop = 0;
      return true;
    } catch (error) {
      if (pageRequestGeneration.current === generation && scope === workspaceScopeRef.current) props.onError(message(error));
      return false;
    } finally {
      if (pageRequestGeneration.current === generation && scope === workspaceScopeRef.current) {
        pageRequestActive.current = false;
        setPageLoading(false);
      }
    }
  }

  async function resetWorkspacePage() {
    if (workspaceScopeRef.current !== workspaceScope) return;
    setWorkspaceCursorHistory([null]);
    setWorkspacePageNumber(1);
    currentWorkspaceCursor.current = null;
    await loadWorkspacePage(null, workspaceScope, { resetScroll: true });
  }

  async function loadNextWorkspacePage() {
    if (!nextWorkspaceCursorRef.current || pageLoading || pageRequestActive.current || !showingCurrentWorkspaceScope) return;
    const cursor = nextWorkspaceCursorRef.current;
    if (await loadWorkspacePage(cursor, workspaceScope, { resetScroll: true })) {
      setWorkspaceCursorHistory((history) => [...history, cursor]);
      setWorkspacePageNumber((page) => page + 1);
    }
  }

  async function loadPreviousWorkspacePage() {
    if (workspacePageNumber <= 1 || pageLoading || pageRequestActive.current || !showingCurrentWorkspaceScope) return;
    const cursor = previousWorkspaceCursor(workspaceCursorHistory, workspacePageNumber);
    if (await loadWorkspacePage(cursor, workspaceScope, { resetScroll: true })) {
      setWorkspaceCursorHistory((history) => history.slice(0, -1));
      setWorkspacePageNumber((page) => page - 1);
    }
  }

  return (
    <section className="panel-stack">
      <div className="stat-grid">
        <Stat label={t("workspaceLoadedCount")} value={String(currentPagedWorkspaces.length)} hint={t("workspaceLoadedCountHint")} />
        <Stat label="CPU" value={`${totals.cpu / 1000} ${t("cores")}`} hint={t("requestedTotal")} />
        <Stat label={t("memory")} value={`${formatGiB(totals.memory)} GiB`} hint={t("requestedTotal")} />
        <Stat label={t("persistentDisk")} value={`${totals.disk} GiB`} hint={`${totals.gpu} GPU`} />
      </div>

      <div className="section-heading">
        <div>
          <p className="eyebrow">{t("workspaces")}</p>
          <h2>{t("workspaces")}</h2>
        </div>
        {canCreateWorkspace && <button className="button primary" onClick={() => setShowCreate((value) => !value)}>
          {showCreate ? t("collapse") : t("newWorkspace")}
        </button>}
      </div>

      {canCreateWorkspace && showCreate && (
        <form className="create-card" onSubmit={create}>
          <label>{t("name")}<input required value={name} onChange={(e) => setName(e.target.value)} /></label>
          <label><FieldTitle label={t("template")} help={t("templatePersistenceHelp")} /><select required value={templateId} onChange={(e) => chooseTemplate(e.target.value)}><option value="">{t("chooseTemplate")}</option>{templates.map((template) => <option key={template.id} value={template.id}>{template.name}</option>)}</select></label>
          {selectedTemplate && <dl className="template-summary wide"><div><dt>{t("image")}</dt><dd><code>{selectedTemplate.image}</code></dd></div><div><dt><FieldTitle label={t("accessMode")} help={selectedTemplate.access_mode === "internal" ? t("internalHelp") : t("publicHelp")} /></dt><dd>{selectedTemplate.access_mode === "internal" ? t("internal") : t("public")}</dd></div><div><dt>{t("resources")}</dt><dd>{selectedTemplate.resources.cpu_millis}m CPU · {selectedTemplate.resources.memory_mib} MiB · {selectedTemplate.resources.disk_gib} GiB · {selectedTemplate.resources.gpu_count} GPU</dd></div><div><dt>{t("workspaceUser")}</dt><dd><code>{selectedTemplate.workspace_user} · {selectedTemplate.workspace_home}</code></dd></div></dl>}
          {selectedTemplate && resourceDraft && <fieldset className="workspace-resource-editor wide"><legend><FieldTitle label={t("workspaceResources")} help={t("workspaceResourcesHelp")} /></legend><div className="workspace-resource-fields"><label>{t("cpuLimitMillis")}<input type="number" min={selectedTemplate.pod_requests.cpu_millis} step="100" required value={resourceDraft.cpu_millis} onChange={(event) => updateResource("cpu_millis", event.target.value)} /></label><label>{t("memoryLimitMib")}<input type="number" min={selectedTemplate.pod_requests.memory_mib} step="256" required value={resourceDraft.memory_mib} onChange={(event) => updateResource("memory_mib", event.target.value)} /></label><label>{t("diskSizeGib")}<input type="number" min="1" step="1" required value={resourceDraft.disk_gib} onChange={(event) => updateResource("disk_gib", event.target.value)} /></label><label>{t("gpuCount")}<input type="number" min="0" step="1" required value={resourceDraft.gpu_count} onChange={(event) => updateResource("gpu_count", event.target.value)} /></label></div></fieldset>}
          <label>{t("injectionReferences")}<select value={explicitInjectionRefs ? "selected" : "all"} onChange={(e) => setReferenceMode(e.target.value === "selected")}><option value="all">{t("allMatching")}</option><option value="selected">{t("selectedReferences")}</option></select></label>
          {explicitInjectionRefs && (
            <CredentialReferencePicker organizationItems={organizationInjections} userItems={userInjections} organizationSelected={organizationRefs} userSelected={userRefs} onOrganizationSelected={setOrganizationRefs} onUserSelected={setUserRefs} />
          )}
          <div className="form-actions"><button className="button primary" disabled={submitting || !templateId}>{submitting ? t("creating") : t("submitCreate")}</button></div>
        </form>
      )}

      <label className="workspace-search">{t("searchWorkspaces")}<input type="search" value={workspaceSearch} onChange={(event) => setWorkspaceSearch(event.target.value)} placeholder={t("searchWorkspacesHint")} /></label>

      {runtimeLoadFailed && currentPagedWorkspaces.length > 0 && <div className="empty" role="status">
        {t("runtimeDataUnavailable")} <button onClick={() => setRuntimeRetryToken((token) => token + 1)}>{t("retryRuntime")}</button>
      </div>}

      <div className="workspace-pagination" aria-label={t("workspacePagination")}>
        <button className="button" type="button" disabled={workspacePageNumber <= 1 || pageLoading} onClick={() => void loadPreviousWorkspacePage()}>{t("previousPage")}</button>
        <span role="status">{t("workspacePageStatus")} {workspacePageNumber} · {currentPagedWorkspaces.length}</span>
        <button className="button" type="button" disabled={!nextWorkspaceCursor || pageLoading} onClick={() => void loadNextWorkspacePage()}>{t("nextPage")}</button>
      </div>

      <div ref={workspaceListRef} className="workspace-list" aria-busy={props.busy || pageLoading} style={{ maxHeight: "min(70vh, 820px)", overflowY: "auto", paddingRight: "6px" }}>
        {currentPagedWorkspaces.length === 0 && pageLoading && <div className="empty" role="status">{t("loadingWorkspaces")}</div>}
        {currentPagedWorkspaces.length === 0 && !pageLoading && showingCurrentWorkspaceScope && <div className="empty">{t("noWorkspaces")}</div>}
        {currentPagedWorkspaces.length > 0 && filteredWorkspaces.length === 0 && <div className="empty">{t("noMatchingWorkspaces")}</div>}
        {visibleWorkspaces.map((item) => <WorkspaceCard key={item.workspace.id} api={props.api} item={item} runtime={runtime[item.workspace.id]} onAction={action} onOpenShell={openShell} onRequestRuntime={refreshWorkspaceRuntime} onError={props.onError} canConnect={canConnectWorkspace} canChangeState={canChangeWorkspaceState} canDelete={canDeleteWorkspace} />)}
      </div>
    </section>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint: string }) {
  return <div className="stat-card"><span>{label}</span><strong>{value}</strong><small>{hint}</small></div>;
}

function formatGiB(mib: number) {
  return (mib / 1024).toFixed(mib % 1024 === 0 ? 0 : 1);
}

function workspaceIdBatches(workspaceIds: string[]): string[][] {
  const batches: string[][] = [];
  for (let index = 0; index < workspaceIds.length; index += 100) batches.push(workspaceIds.slice(index, index + 100));
  return batches;
}

function sameResources(left: Resources, right: Resources) {
  return left.cpu_millis === right.cpu_millis
    && left.memory_mib === right.memory_mib
    && left.disk_gib === right.disk_gib
    && left.gpu_count === right.gpu_count;
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "操作失败";
}

function FieldTitle({ label, help }: { label: string; help: string }) {
  return <span className="field-title"><span>{label}</span><span className="help-tip" title={help} aria-label={help} tabIndex={0}>?</span></span>;
}
