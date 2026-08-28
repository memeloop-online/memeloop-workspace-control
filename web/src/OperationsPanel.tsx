import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import { RUNTIME_PROFILES, isHighRiskRuntimeProfile, runtimeProfileLabel } from "./runtimeProfiles";
import type { AccessMode, AuditRecord, ImagePolicy, Principal, Resources, Role, RuntimeProfile, ScalingStatus, UserSummary, WebhookSubscription, WorkspaceResponse, WorkspaceTemplate } from "./types";

export function OperationsPanel({ api, principal, organizationId, workspaces, onError }: { api: ApiClient; principal: Principal; organizationId: string; workspaces: WorkspaceResponse[]; onError: (message: string) => void }) {
  const { locale, t } = useI18n();
  const [audit, setAudit] = useState<AuditRecord[]>([]);
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [images, setImages] = useState<ImagePolicy[]>([]);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [webhooks, setWebhooks] = useState<WebhookSubscription[]>([]);
  const [scaling, setScaling] = useState<ScalingStatus | null>(null);
  const [quota, setQuota] = useState<Resources | null>(null);
  const [image, setImage] = useState("");
  const [organizationName, setOrganizationName] = useState("");
  const [memberUser, setMemberUser] = useState("");
  const [memberRole, setMemberRole] = useState<Role>("member");
  const [showTemplateForm, setShowTemplateForm] = useState(false);
  const [templateName, setTemplateName] = useState("");
  const [templateImage, setTemplateImage] = useState("");
  const [templateProfile, setTemplateProfile] = useState<RuntimeProfile | "">("");
  const [templateAccessMode, setTemplateAccessMode] = useState<AccessMode>("internal");
  const [templateCpu, setTemplateCpu] = useState(2000);
  const [templateMemory, setTemplateMemory] = useState(4096);
  const [templateGpu, setTemplateGpu] = useState(0);
  const [templateDisk, setTemplateDisk] = useState(50);
  const [creatingTemplate, setCreatingTemplate] = useState(false);
  const [editingQuota, setEditingQuota] = useState(false);
  const [quotaDraft, setQuotaDraft] = useState<Resources>({ cpu_millis: 4000, memory_mib: 8192, gpu_count: 0, disk_gib: 100 });
  const canManageQuota = principal.system_admin || principal.memberships.some((membership) => membership.organization_id === organizationId && membership.role === "organization_admin");
  const states = workspaces.reduce<Record<string, number>>((sum, item) => ({ ...sum, [item.workspace.state]: (sum[item.workspace.state] ?? 0) + 1 }), {});

  async function refresh() {
    try {
      const common = [api.audit(organizationId), api.quota(organizationId), api.templates(organizationId), api.webhooks(organizationId)] as const;
      const [records, currentQuota, visibleTemplates, subscriptions] = await Promise.all(common);
      setAudit(records); setQuota(currentQuota); setQuotaDraft(currentQuota ?? { cpu_millis: 4000, memory_mib: 8192, gpu_count: 0, disk_gib: 100 }); setTemplates(visibleTemplates); setWebhooks(subscriptions);
      if (principal.system_admin) {
        const [allUsers, allImages, status] = await Promise.all([api.users(), api.images(), api.scaling()]);
        setUsers(allUsers); setImages(allImages); setScaling(status);
      }
    } catch (error) { onError(message(error)); }
  }

  useEffect(() => { void refresh(); }, [api, organizationId]);

  async function allowImage() {
    try { await api.putImage(image.trim()); setImage(""); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function createOrganization() {
    try { await api.createOrganization(organizationName.trim()); setOrganizationName(""); window.location.reload(); } catch (error) { onError(message(error)); }
  }

  async function grantMember() {
    try { await api.setMembership(organizationId, memberUser, memberRole); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function revokeMember() {
    if (!memberUser || !confirm(locale === "zh-CN" ? "撤销该用户的组织成员关系和新 SSH 连接权限？" : "Revoke this user's membership and new SSH access?")) return;
    try { await api.removeMembership(organizationId, memberUser); setMemberUser(""); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function saveQuota() {
    try { await api.setQuota(organizationId, quotaDraft); setEditingQuota(false); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function editUserQuota() {
    if (!memberUser) return;
    try {
      const current = await api.userQuota(memberUser);
      const cpu = prompt(locale === "zh-CN" ? "用户 CPU 总额度（millicores）" : "User CPU quota (millicores)", String(current?.cpu_millis ?? 4000));
      const memory = prompt(locale === "zh-CN" ? "用户内存总额度（MiB）" : "User memory quota (MiB)", String(current?.memory_mib ?? 8192));
      const gpu = prompt(locale === "zh-CN" ? "用户 GPU 总额度" : "User GPU quota", String(current?.gpu_count ?? 0));
      const disk = prompt(locale === "zh-CN" ? "用户磁盘总额度（GiB）" : "User disk quota (GiB)", String(current?.disk_gib ?? 100));
      if ([cpu, memory, gpu, disk].some((value) => value === null)) return;
      await api.setUserQuota(memberUser, { cpu_millis: Number(cpu), memory_mib: Number(memory), gpu_count: Number(gpu), disk_gib: Number(disk) });
      await refresh();
    } catch (error) { onError(message(error)); }
  }

  async function newTemplate(event: FormEvent) {
    event.preventDefault();
    if (!templateProfile) {
      onError(locale === "zh-CN" ? "必须选择受控运行时配置" : "Choose a controlled runtime profile");
      return;
    }
    if (
      isHighRiskRuntimeProfile(templateProfile)
      && !confirm(locale === "zh-CN" ? "该模板将使用集群管理员运行时配置，可能拥有集群级权限。确认创建？" : "This template may grant cluster-level privileges. Create it?")
    ) return;
    setCreatingTemplate(true);
    try {
      await api.createTemplate({
        organization_id: organizationId,
        name: templateName.trim(),
        image: templateImage.trim(),
        runtime_profile: templateProfile,
        access_mode: templateAccessMode,
        resources: {
          cpu_millis: templateCpu,
          memory_mib: templateMemory,
          gpu_count: templateGpu,
          disk_gib: templateDisk,
        },
      });
      setTemplateName("");
      setTemplateImage("");
      setTemplateProfile("");
      setShowTemplateForm(false);
      await refresh();
    } catch (error) {
      onError(message(error));
    } finally {
      setCreatingTemplate(false);
    }
  }

  async function newWebhook() {
    const url = prompt(locale === "zh-CN" ? "公开 HTTPS Webhook URL" : "Public HTTPS webhook URL"); const secret = prompt(locale === "zh-CN" ? "签名密钥（至少 32 字节）" : "Signing credential (at least 32 bytes)");
    if (!url || !secret) return;
    try { await api.createWebhook({ organization_id: organizationId, url, event_prefix: "workspace.", signing_secret: secret }); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function toggleTemplate(template: WorkspaceTemplate) {
    if (template.enabled && !confirm(locale === "zh-CN" ? `停用模板“${template.name}”？现有工作区不会被删除。` : `Disable template “${template.name}”? Existing workspaces are kept.`)) return;
    try {
      await api.setTemplateEnabled(template.id, !template.enabled);
      await refresh();
    } catch (error) { onError(message(error)); }
  }

  async function newUser() {
    const name = prompt(locale === "zh-CN" ? "用户显示名" : "User display name"); const token = prompt(locale === "zh-CN" ? "初始 API Token（至少 32 字节，不会回显）" : "Initial API token (at least 32 bytes; write-only)");
    if (!name || !token) return;
    try { await api.createUser(name, token); await refresh(); } catch (error) { onError(message(error)); }
  }

  return <section className="panel-stack">
    <div className="section-heading"><div><p className="eyebrow">OPERATIONS</p><h2>{t("operationsTitle")}</h2></div><a className="button" href="/api/v1/openapi.json" target="_blank">OpenAPI</a></div>
    <div className="system-grid">
      <div className="system-card"><h3>{t("identityQuota")}</h3><dl><dt>{t("user")}</dt><dd>{principal.display_name}</dd><dt>{t("systemAdmin")}</dt><dd>{principal.system_admin ? t("enabled") : t("disabled")}</dd><dt>{t("orgQuota")}</dt><dd>{quota ? `${quota.cpu_millis}m / ${quota.memory_mib}Mi / ${quota.disk_gib}Gi / ${quota.gpu_count} GPU` : t("notEnabled")}</dd></dl>{editingQuota ? <div className="quota-editor"><label>CPU (m)<input type="number" min="100" value={quotaDraft.cpu_millis} onChange={(event) => setQuotaDraft({ ...quotaDraft, cpu_millis: Number(event.target.value) })} /></label><label>{t("memory")} (MiB)<input type="number" min="128" value={quotaDraft.memory_mib} onChange={(event) => setQuotaDraft({ ...quotaDraft, memory_mib: Number(event.target.value) })} /></label><label>GPU<input type="number" min="0" value={quotaDraft.gpu_count} onChange={(event) => setQuotaDraft({ ...quotaDraft, gpu_count: Number(event.target.value) })} /></label><label>{t("disk")} (GiB)<input type="number" min="1" value={quotaDraft.disk_gib} onChange={(event) => setQuotaDraft({ ...quotaDraft, disk_gib: Number(event.target.value) })} /></label><div className="quota-actions"><button className="button primary" onClick={() => void saveQuota()}>{t("saveQuota")}</button><button className="button" onClick={() => setEditingQuota(false)}>{t("cancel")}</button></div></div> : <><button className="button" disabled={!canManageQuota} title={canManageQuota ? undefined : t("noQuotaPermission")} onClick={() => setEditingQuota(true)}>{t("editQuota")}</button>{!canManageQuota && <p className="security-note">{t("noQuotaPermission")}</p>}</>}</div>
      <div className="system-card"><h3>{t("workspaceState")}</h3><div className="state-bars">{Object.entries(states).map(([state, count]) => <div key={state}><span>{state}</span><strong>{count}</strong></div>)}</div></div>
      {scaling && <div className="system-card"><h3>{t("scaling")}</h3><dl><dt>{t("database")}</dt><dd>{scaling.database_mode}</dd><dt>{t("replicas")}</dt><dd>{scaling.configured_replicas}</dd><dt>{t("jobs")}</dt><dd>{scaling.jobs.pending} pending · {scaling.jobs.running} running</dd><dt>{t("schema")}</dt><dd>v{scaling.schema_version}</dd></dl></div>}
      <div className="system-card wide"><h3>{t("audit")}</h3>{audit.length ? <div className="state-bars">{audit.slice(0, 20).map((record) => <div key={record.id}><span>{record.action}<small> · {new Date(record.created_at * 1000).toLocaleString()}</small></span><code>{record.workspace_id?.slice(0, 8) ?? "organization"}</code></div>)}</div> : <p>{t("noAudit")}</p>}</div>
      {principal.system_admin && <><div className="system-card"><h3>{t("usersRoles")}</h3><button className="button" onClick={() => void newUser()}>{t("createUser")}</button><label>{t("member")}<select value={memberUser} onChange={(event) => setMemberUser(event.target.value)}><option value="">{t("chooseUser")}</option>{users.map((user) => <option key={user.id} value={user.id}>{user.display_name}</option>)}</select></label><label>{t("role")}<select value={memberRole} onChange={(event) => setMemberRole(event.target.value as Role)}><option value="member">member</option><option value="organization_admin">organization_admin</option></select></label><button className="button" disabled={!memberUser} onClick={() => void grantMember()}>{t("saveMembership")}</button><button className="button" disabled={!memberUser} onClick={() => void editUserQuota()}>{t("editUserQuota")}</button><button className="button danger" disabled={!memberUser} onClick={() => void revokeMember()}>{t("revokeMembership")}</button></div><div className="system-card"><h3>{t("imageAllowlist")} · Contract v1</h3><label>{t("ociImage")}<input value={image} onChange={(event) => setImage(event.target.value)} placeholder="registry/image@sha256:…" /></label><button className="button" disabled={!image.trim()} onClick={() => void allowImage()}>{t("allowImage")}</button><div className="state-bars">{images.map((item) => <div key={item.image}><code>{item.image}</code><strong>{item.enabled ? t("enabled") : t("disabled")}</strong></div>)}</div></div><div className="system-card"><h3>{t("createOrganization")}</h3><label>{t("name")}<input value={organizationName} onChange={(event) => setOrganizationName(event.target.value)} /></label><button className="button" disabled={!organizationName.trim()} onClick={() => void createOrganization()}>{t("createOrganization")}</button></div></>}
      <div className="system-card wide">
        <div className="card-heading">
          <h3>{t("templates")}</h3>
          <button className="button" onClick={() => setShowTemplateForm((current) => !current)}>
            {showTemplateForm ? t("cancel") : t("createTemplate")}
          </button>
        </div>
        {showTemplateForm && (
          <form className="template-form" onSubmit={newTemplate}>
            <label>{t("templateName")}<input required value={templateName} onChange={(event) => setTemplateName(event.target.value)} /></label>
            <label className="wide">{t("allowedOciImage")}<input required value={templateImage} onChange={(event) => setTemplateImage(event.target.value)} placeholder="registry/image@sha256:…" /></label>
            <label>
              {t("runtimeProfile")}
              <select required value={templateProfile} onChange={(event) => setTemplateProfile(event.target.value as RuntimeProfile | "")}>
                <option value="">{t("chooseRuntime")}</option>
                {RUNTIME_PROFILES.map((profile) => <option key={profile.value} value={profile.value}>{locale === "en" ? profile.labelEn : profile.label}</option>)}
              </select>
            </label>
            <label>{t("accessMode")}<select value={templateAccessMode} onChange={(event) => setTemplateAccessMode(event.target.value as AccessMode)}><option value="internal">{t("internal")}</option><option value="public">{t("public")}</option></select></label>
            <label>{t("cpu")}（m）<input required type="number" min="100" value={templateCpu} onChange={(event) => setTemplateCpu(Number(event.target.value))} /></label>
            <label>{t("memory")}（MiB）<input required type="number" min="128" value={templateMemory} onChange={(event) => setTemplateMemory(Number(event.target.value))} /></label>
            <label>{t("gpu")}<input required type="number" min="0" value={templateGpu} onChange={(event) => setTemplateGpu(Number(event.target.value))} /></label>
            <label>{t("disk")}（GiB）<input required type="number" min="1" value={templateDisk} onChange={(event) => setTemplateDisk(Number(event.target.value))} /></label>
            {templateProfile && <p className={isHighRiskRuntimeProfile(templateProfile) ? "risk-note wide" : "profile-note wide"}>{locale === "en" ? RUNTIME_PROFILES.find((profile) => profile.value === templateProfile)?.descriptionEn : RUNTIME_PROFILES.find((profile) => profile.value === templateProfile)?.description}</p>}
            <div className="form-actions wide"><button className="button primary" disabled={creatingTemplate}>{creatingTemplate ? t("creating") : t("saveTemplate")}</button></div>
          </form>
        )}
        {templates.length ? <div className="state-bars template-list">{templates.map((template) => <div key={template.id}><span>{template.name} · {template.access_mode} · {runtimeProfileLabel(template.runtime_profile, locale)} · {template.enabled ? t("enabled") : t("disabled")}</span><code>{template.image}</code>{principal.system_admin && <button className={template.enabled ? "button danger" : "button"} onClick={() => void toggleTemplate(template)}>{template.enabled ? t("disabled") : t("enabled")}</button>}</div>)}</div> : <p>{t("noTemplates")}</p>}
      </div>
      <div className="system-card wide"><h3>{t("webhook")}</h3><button className="button" onClick={() => void newWebhook()}>{t("addWebhook")}</button>{webhooks.length ? <div className="state-bars">{webhooks.map((hook) => <div key={hook.id}><span>{hook.event_prefix}</span><code>{hook.url}</code></div>)}</div> : <p>{t("noWebhooks")}</p>}</div>
    </div>
  </section>;
}

function message(error: unknown) { return error instanceof Error ? error.message : "请求失败"; }
