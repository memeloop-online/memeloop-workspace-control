import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { CredentialReferencePicker } from "./forms/CredentialReferencePicker";
import { useI18n } from "./i18n";
import { WorkspaceCard } from "./WorkspaceCard";
import type {
  CreateWorkspace,
  Principal,
  StoredInjection,
  WorkspaceResponse,
  WorkspaceRuntime,
  WorkspaceTemplate,
} from "./types";

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
  const [organizationInjections, setOrganizationInjections] = useState<StoredInjection[]>([]);
  const [userInjections, setUserInjections] = useState<StoredInjection[]>([]);
  const [explicitInjectionRefs, setExplicitInjectionRefs] = useState(false);
  const [organizationRefs, setOrganizationRefs] = useState<string[]>([]);
  const [userRefs, setUserRefs] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [runtime, setRuntime] = useState<Record<string, WorkspaceRuntime>>({});
  const [workspaceSearch, setWorkspaceSearch] = useState("");
  const runtimeKey = props.workspaces.map((item) => `${item.workspace.id}:${item.workspace.state}`).join(",");

  useEffect(() => {
    let active = true;
    const refreshRuntime = async () => {
      if (document.visibilityState !== "visible" || props.workspaces.length === 0) return;
      try {
        const entries = await props.api.workspaceRuntimes(props.organizationId);
        if (!active) return;
        setRuntime((current) => Object.fromEntries(entries.map((entry) => [
          entry.workspace_id,
          { ...entry.runtime, events: current[entry.workspace_id]?.events ?? [] },
        ])));
      } catch {
        // Runtime observations are supplemental. The workspace lifecycle list
        // remains usable when metrics.k8s.io or Kubernetes is temporarily down.
      }
    };
    void refreshRuntime();
    const timer = window.setInterval(() => void refreshRuntime(), 30000);
    return () => { active = false; window.clearInterval(timer); };
  }, [props.api, props.organizationId, runtimeKey]);

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
      props.workspaces.reduce(
        (sum, item) => ({
          cpu: sum.cpu + item.workspace.resources.cpu_millis,
          memory: sum.memory + item.workspace.resources.memory_mib,
          disk: sum.disk + item.workspace.resources.disk_gib,
          gpu: sum.gpu + item.workspace.resources.gpu_count,
        }),
        { cpu: 0, memory: 0, disk: 0, gpu: 0 },
      ),
    [props.workspaces],
  );
  const selectedTemplate = templates.find((template) => template.id === templateId);
  const filteredWorkspaces = useMemo(() => {
    const query = workspaceSearch.trim().toLocaleLowerCase();
    if (!query) return props.workspaces;
    return props.workspaces.filter(({ workspace }) => [
      workspace.name,
      workspace.short_id,
      workspace.state,
      workspace.image,
      workspace.workspace_user,
    ].some((value) => value.toLocaleLowerCase().includes(query)));
  }, [props.workspaces, workspaceSearch]);

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
      setShowCreate(false);
      await props.onRefresh();
    } catch (error) {
      props.onError(message(error));
    } finally {
      setSubmitting(false);
    }
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
    } catch (error) {
      props.onError(message(error));
    }
  }

  async function openShell(workspaceId: string) {
    // Open synchronously so browser popup protection permits the terminal tab. Passing the
    // `noopener` feature makes Chromium return null even when it created a tab, which leaves an
    // unreachable blank tab and forces the fallback navigation in this page. Clear opener before
    // awaiting the ticket instead.
    const target = window.open("about:blank", "_blank");
    if (target) target.opener = null;
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
    try {
      const observation = await props.api.workspaceRuntime(workspaceId);
      setRuntime((current) => ({ ...current, [workspaceId]: observation }));
    } catch (error) {
      props.onError(message(error));
    }
  }

  return (
    <section className="panel-stack">
      <div className="stat-grid">
        <Stat label={t("workspaceCount")} value={String(props.workspaces.length)} hint={t("currentOrganization")} />
        <Stat label="CPU" value={`${totals.cpu / 1000} ${t("cores")}`} hint={t("requestedTotal")} />
        <Stat label={t("memory")} value={`${formatGiB(totals.memory)} GiB`} hint={t("requestedTotal")} />
        <Stat label={t("persistentDisk")} value={`${totals.disk} GiB`} hint={`${totals.gpu} GPU`} />
      </div>

      <div className="section-heading">
        <div>
          <p className="eyebrow">{t("workspaces")}</p>
          <h2>{t("workspaces")}</h2>
        </div>
        <button className="button primary" onClick={() => setShowCreate((value) => !value)}>
          {showCreate ? t("collapse") : t("newWorkspace")}
        </button>
      </div>

      {showCreate && (
        <form className="create-card" onSubmit={create}>
          <label>{t("name")}<input required value={name} onChange={(e) => setName(e.target.value)} /></label>
          <label><FieldTitle label={t("template")} help={t("templatePersistenceHelp")} /><select required value={templateId} onChange={(e) => setTemplateId(e.target.value)}><option value="">{t("chooseTemplate")}</option>{templates.map((template) => <option key={template.id} value={template.id}>{template.name}</option>)}</select></label>
          {selectedTemplate && <dl className="template-summary wide"><div><dt>{t("image")}</dt><dd><code>{selectedTemplate.image}</code></dd></div><div><dt><FieldTitle label={t("accessMode")} help={selectedTemplate.access_mode === "internal" ? t("internalHelp") : t("publicHelp")} /></dt><dd>{selectedTemplate.access_mode === "internal" ? t("internal") : t("public")}</dd></div><div><dt>{t("resources")}</dt><dd>{selectedTemplate.resources.cpu_millis}m CPU · {selectedTemplate.resources.memory_mib} MiB · {selectedTemplate.resources.disk_gib} GiB · {selectedTemplate.resources.gpu_count} GPU</dd></div><div><dt>{t("workspaceUser")}</dt><dd><code>{selectedTemplate.workspace_user} · {selectedTemplate.workspace_home}</code></dd></div></dl>}
          <label>{t("injectionReferences")}<select value={explicitInjectionRefs ? "selected" : "all"} onChange={(e) => setReferenceMode(e.target.value === "selected")}><option value="all">{t("allMatching")}</option><option value="selected">{t("selectedReferences")}</option></select></label>
          {explicitInjectionRefs && (
            <CredentialReferencePicker organizationItems={organizationInjections} userItems={userInjections} organizationSelected={organizationRefs} userSelected={userRefs} onOrganizationSelected={setOrganizationRefs} onUserSelected={setUserRefs} />
          )}
          <div className="form-actions"><button className="button primary" disabled={submitting || !templateId}>{submitting ? t("creating") : t("submitCreate")}</button></div>
        </form>
      )}

      <label className="workspace-search">{t("searchWorkspaces")}<input type="search" value={workspaceSearch} onChange={(event) => setWorkspaceSearch(event.target.value)} placeholder={t("searchWorkspacesHint")} /></label>

      <div className="workspace-list" aria-busy={props.busy}>
        {props.workspaces.length === 0 && <div className="empty">{t("noWorkspaces")}</div>}
        {props.workspaces.length > 0 && filteredWorkspaces.length === 0 && <div className="empty">{t("noMatchingWorkspaces")}</div>}
        {filteredWorkspaces.map((item) => <WorkspaceCard key={item.workspace.id} item={item} runtime={runtime[item.workspace.id]} onAction={action} onOpenShell={openShell} onRequestRuntime={refreshWorkspaceRuntime} />)}
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

function message(error: unknown) {
  return error instanceof Error ? error.message : "操作失败";
}

function FieldTitle({ label, help }: { label: string; help: string }) {
  return <span className="field-title"><span>{label}</span><span className="help-tip" title={help} aria-label={help} tabIndex={0}>?</span></span>;
}
