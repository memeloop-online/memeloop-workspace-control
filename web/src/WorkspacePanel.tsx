import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import { isHighRiskRuntimeProfile, runtimeProfileDescription, runtimeProfileLabel } from "./runtimeProfiles";
import { WorkspaceCard } from "./WorkspaceCard";
import type {
  AccessMode,
  CreateWorkspace,
  Principal,
  RuntimeProfile,
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
  const [image, setImage] = useState("");
  const [runtimeProfile, setRuntimeProfile] = useState<RuntimeProfile | "">("");
  const [accessMode, setAccessMode] = useState<AccessMode>("internal");
  const [cpu, setCpu] = useState(1000);
  const [memory, setMemory] = useState(2048);
  const [gpu, setGpu] = useState(0);
  const [disk, setDisk] = useState(20);
  const [organizationInjections, setOrganizationInjections] = useState<StoredInjection[]>([]);
  const [userInjections, setUserInjections] = useState<StoredInjection[]>([]);
  const [explicitInjectionRefs, setExplicitInjectionRefs] = useState(false);
  const [organizationRefs, setOrganizationRefs] = useState<string[]>([]);
  const [userRefs, setUserRefs] = useState<string[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [runtime, setRuntime] = useState<Record<string, WorkspaceRuntime>>({});
  const runtimeKey = props.workspaces.map((item) => `${item.workspace.id}:${item.workspace.state}`).join(",");

  useEffect(() => {
    let active = true;
    const refreshRuntime = async () => {
      const visible = props.workspaces.filter((item) => !["deleting", "deleted"].includes(item.workspace.state));
      const results = await Promise.allSettled(visible.map(async (item) => [item.workspace.id, await props.api.workspaceRuntime(item.workspace.id)] as const));
      if (!active) return;
      setRuntime((current) => {
        const next = { ...current };
        for (const result of results) if (result.status === "fulfilled") next[result.value[0]] = result.value[1];
        return next;
      });
    };
    void refreshRuntime();
    const timer = window.setInterval(() => void refreshRuntime(), 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [props.api, runtimeKey]);

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

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!templateId || !runtimeProfile) {
      props.onError(locale === "zh-CN" ? "必须选择一个受控工作区模板" : "Choose a controlled workspace template");
      return;
    }
    if (
      isHighRiskRuntimeProfile(runtimeProfile)
      && !confirm(locale === "zh-CN" ? "该工作区将使用集群管理员运行时配置，可能拥有集群级权限。确认创建？" : "This workspace may receive cluster-level privileges. Create it?")
    ) return;
    const command: CreateWorkspace = {
      organization_id: props.organizationId,
      owner_id: props.principal.user_id,
      name,
      template_id: templateId || null,
      organization_injection_refs: explicitInjectionRefs
        ? organizationInjections
            .filter((item) => item.locked || organizationRefs.includes(item.key))
            .map((item) => item.key)
        : null,
      user_injection_refs: explicitInjectionRefs ? userRefs : null,
      image,
      runtime_profile: runtimeProfile,
      access_mode: accessMode,
      resources: {
        cpu_millis: cpu,
        memory_mib: memory,
        gpu_count: gpu,
        disk_gib: disk,
      },
    };
    setSubmitting(true);
    try {
      await props.api.createWorkspace(command);
      setName("");
      setTemplateId("");
      setRuntimeProfile("");
      setImage("");
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

  function toggleReference(
    key: string,
    selected: string[],
    update: (value: string[]) => void,
  ) {
    update(selected.includes(key) ? selected.filter((value) => value !== key) : [...selected, key]);
  }

  function selectTemplate(id: string) {
    setTemplateId(id);
    const template = templates.find((item) => item.id === id);
    if (!template) {
      setImage("");
      setRuntimeProfile("");
      return;
    }
    setImage(template.image);
    setRuntimeProfile(template.runtime_profile);
    setAccessMode(template.access_mode);
    setCpu(template.resources.cpu_millis);
    setMemory(template.resources.memory_mib);
    setGpu(template.resources.gpu_count);
    setDisk(template.resources.disk_gib);
  }

  async function action(
    id: string,
    value: "start" | "stop" | "restart" | "delete",
  ) {
    if (value === "delete" && !confirm(locale === "zh-CN" ? "确定删除这个工作区？PVC 与命名空间会被清理。" : "Delete this workspace? Its PVC and namespace will be removed.")) {
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
    const target = window.open("about:blank", "_blank", "noopener,noreferrer");
    try {
      const ticket = await props.api.issueWebShellTicket(workspaceId);
      if (target) target.location.href = ticket.web_shell_url;
      else window.location.href = ticket.web_shell_url;
    } catch (error) {
      target?.close();
      props.onError(message(error));
    }
  }

  return (
    <section className="panel-stack">
      <div className="stat-grid">
        <Stat label={t("workspaceCount")} value={String(props.workspaces.length)} hint={t("currentOrganization")} />
        <Stat label="CPU" value={`${totals.cpu / 1000} ${locale === "zh-CN" ? "核" : "cores"}`} hint={t("requestedTotal")} />
        <Stat label={t("memory")} value={`${formatGiB(totals.memory)} GiB`} hint={t("requestedTotal")} />
        <Stat label={t("persistentDisk")} value={`${totals.disk} GiB`} hint={`${totals.gpu} GPU`} />
      </div>

      <div className="section-heading">
        <div>
          <p className="eyebrow">WORKSPACES</p>
          <h2>{t("workspaces")}</h2>
        </div>
        <button className="button primary" onClick={() => setShowCreate((value) => !value)}>
          {showCreate ? t("collapse") : t("newWorkspace")}
        </button>
      </div>

      {showCreate && (
        <form className="create-card" onSubmit={create}>
          <label>{t("name")}<input required value={name} onChange={(e) => setName(e.target.value)} /></label>
          <label>{t("template")}<select required value={templateId} onChange={(e) => selectTemplate(e.target.value)}><option value="">{t("chooseTemplate")}</option>{templates.map((template) => <option key={template.id} value={template.id}>{template.name} · {runtimeProfileLabel(template.runtime_profile, locale)}</option>)}</select></label>
          <label>{t("runtimeProfile")}<input readOnly value={runtimeProfile ? runtimeProfileLabel(runtimeProfile, locale) : t("inheritedFromTemplate")} /></label>
          <label className="wide">{t("image")}<input required readOnly value={image} /></label>
          <label>CPU（m）<input type="number" min="100" readOnly value={cpu} /></label>
          <label>{t("memory")}（MiB）<input type="number" min="128" readOnly value={memory} /></label>
          <label>GPU<input type="number" min="0" readOnly value={gpu} /></label>
          <label>{t("disk")}（GiB）<input type="number" min="1" readOnly value={disk} /></label>
          <label>{t("accessMode")}<select disabled value={accessMode}><option value="internal">{t("internal")}</option><option value="public">{t("public")}</option></select></label>
          {runtimeProfile && <p className={isHighRiskRuntimeProfile(runtimeProfile) ? "risk-note wide" : "profile-note wide"}>{runtimeProfileDescription(runtimeProfile, locale)}</p>}
          <label>{t("injectionReferences")}<select value={explicitInjectionRefs ? "selected" : "all"} onChange={(e) => setReferenceMode(e.target.value === "selected")}><option value="all">{t("allMatching")}</option><option value="selected">{t("selectedReferences")}</option></select></label>
          {explicitInjectionRefs && (
            <fieldset className="injection-ref-picker wide">
              <legend>{t("organizationAndUserCredentials")}</legend>
              <div>
                <strong>{t("scopeOrganization")}</strong>
                {organizationInjections.length === 0 && <small>{t("noItems")}</small>}
                {organizationInjections.map((item) => <label key={item.key}><input type="checkbox" checked={item.locked || organizationRefs.includes(item.key)} disabled={item.locked} onChange={() => toggleReference(item.key, organizationRefs, setOrganizationRefs)} /><span>{item.key}{item.locked ? ` · ${t("locked")}` : ""}</span></label>)}
              </div>
              <div>
                <strong>{t("scopeUser")}</strong>
                {userInjections.length === 0 && <small>{t("noItems")}</small>}
                {userInjections.map((item) => <label key={item.key}><input type="checkbox" checked={userRefs.includes(item.key)} onChange={() => toggleReference(item.key, userRefs, setUserRefs)} /><span>{item.key}</span></label>)}
              </div>
              <p>{t("selectedReferenceHelp")}</p>
            </fieldset>
          )}
          <div className="form-actions"><button className="button primary" disabled={submitting || !templateId}>{submitting ? t("creating") : t("submitCreate")}</button></div>
        </form>
      )}

      <div className="workspace-list" aria-busy={props.busy}>
        {props.workspaces.length === 0 && <div className="empty">{t("noWorkspaces")}</div>}
        {props.workspaces.map((item) => <WorkspaceCard key={item.workspace.id} item={item} runtime={runtime[item.workspace.id]} onAction={action} onOpenShell={openShell} />)}
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
