import { useEffect, useState } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import { TemplateEditor } from "./TemplateEditor";
import type { AuditRecord, ImagePolicy, Principal, Resources, Role, ScalingStatus, UserSummary, WebhookSubscription, WorkspaceResponse, WorkspaceTemplate } from "./types";

interface ResourceDraft {
  cpu_millis: string;
  memory_mib: string;
  gpu_count: string;
  disk_gib: string;
}

const DEFAULT_QUOTA: Resources = { cpu_millis: 4000, memory_mib: 8192, gpu_count: 0, disk_gib: 100 };

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
  const [editingQuota, setEditingQuota] = useState(false);
  const [quotaDraft, setQuotaDraft] = useState<ResourceDraft>(() => resourceDraft(DEFAULT_QUOTA));
  const canManageQuota = principal.system_admin || principal.memberships.some((membership) => membership.organization_id === organizationId && membership.role === "organization_admin");
  const states = workspaces.reduce<Record<string, number>>((sum, item) => ({ ...sum, [item.workspace.state]: (sum[item.workspace.state] ?? 0) + 1 }), {});

  async function refresh() {
    try {
      const common = [api.audit(organizationId), api.quota(organizationId), api.templates(organizationId), api.webhooks(organizationId)] as const;
      const [records, currentQuota, visibleTemplates, subscriptions] = await Promise.all(common);
      setAudit(records); setQuota(currentQuota); setQuotaDraft(resourceDraft(currentQuota ?? DEFAULT_QUOTA)); setTemplates(visibleTemplates); setWebhooks(subscriptions);
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
    if (!memberUser || !confirm(t("revokeMemberConfirm"))) return;
    try { await api.removeMembership(organizationId, memberUser); setMemberUser(""); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function saveQuota() {
    try { await api.setQuota(organizationId, parseResourceDraft(quotaDraft)); setEditingQuota(false); await refresh(); } catch (error) { onError(error instanceof InvalidResourceDraft ? t("invalidTemplateNumber") : message(error)); }
  }

  async function editUserQuota() {
    if (!memberUser) return;
    try {
      const current = await api.userQuota(memberUser);
      const cpu = prompt(t("userCpuQuotaPrompt"), String(current?.cpu_millis ?? 4000));
      const memory = prompt(t("userMemoryQuotaPrompt"), String(current?.memory_mib ?? 8192));
      const gpu = prompt(t("userGpuQuotaPrompt"), String(current?.gpu_count ?? 0));
      const disk = prompt(t("userDiskQuotaPrompt"), String(current?.disk_gib ?? 100));
      if ([cpu, memory, gpu, disk].some((value) => value === null)) return;
      await api.setUserQuota(memberUser, { cpu_millis: Number(cpu), memory_mib: Number(memory), gpu_count: Number(gpu), disk_gib: Number(disk) });
      await refresh();
    } catch (error) { onError(message(error)); }
  }

  async function newWebhook() {
    const url = prompt(t("webhookUrlPrompt")); const secret = prompt(t("webhookSecretPrompt"));
    if (!url || !secret) return;
    try { await api.createWebhook({ organization_id: organizationId, url, event_prefix: "workspace.", signing_secret: secret }); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function newUser() {
    const name = prompt(t("userDisplayNamePrompt")); const token = prompt(t("initialTokenPrompt"));
    if (!name || !token) return;
    try { await api.createUser(name, token); await refresh(); } catch (error) { onError(message(error)); }
  }

  return <section className="panel-stack">
    <div className="section-heading"><div><p className="eyebrow">OPERATIONS</p><h2>{t("operationsTitle")}</h2></div><a className="button" href="/api/v1/openapi.json" target="_blank">OpenAPI</a></div>
    <div className="system-grid">
      <div className="system-card"><h3>{t("identityQuota")}</h3><dl><dt>{t("user")}</dt><dd>{principal.display_name}</dd><dt>{t("systemAdmin")}</dt><dd>{principal.system_admin ? t("enabled") : t("disabled")}</dd><dt>{t("orgQuota")}</dt><dd>{quota ? `${quota.cpu_millis}m / ${quota.memory_mib}Mi / ${quota.disk_gib}Gi / ${quota.gpu_count} GPU` : t("notEnabled")}</dd></dl>{editingQuota ? <div className="quota-editor"><label>CPU (m)<input type="number" inputMode="numeric" min="100" step="100" value={quotaDraft.cpu_millis} onChange={(event) => setQuotaDraft({ ...quotaDraft, cpu_millis: event.target.value })} /></label><label>{t("memory")} (MiB)<input type="number" inputMode="numeric" min="128" step="128" value={quotaDraft.memory_mib} onChange={(event) => setQuotaDraft({ ...quotaDraft, memory_mib: event.target.value })} /></label><label>GPU<input type="number" inputMode="numeric" min="0" step="1" value={quotaDraft.gpu_count} onChange={(event) => setQuotaDraft({ ...quotaDraft, gpu_count: event.target.value })} /></label><label>{t("disk")} (GiB)<input type="number" inputMode="numeric" min="1" step="1" value={quotaDraft.disk_gib} onChange={(event) => setQuotaDraft({ ...quotaDraft, disk_gib: event.target.value })} /></label><div className="quota-actions"><button className="button primary" onClick={() => void saveQuota()}>{t("saveQuota")}</button><button className="button" onClick={() => setEditingQuota(false)}>{t("cancel")}</button></div></div> : <><button className="button" disabled={!canManageQuota} title={canManageQuota ? undefined : t("noQuotaPermission")} onClick={() => setEditingQuota(true)}>{t("editQuota")}</button>{!canManageQuota && <p className="security-note">{t("noQuotaPermission")}</p>}</>}</div>
      <div className="system-card"><h3>{t("workspaceState")}</h3><div className="state-bars">{Object.entries(states).map(([state, count]) => <div key={state}><span>{state}</span><strong>{count}</strong></div>)}</div></div>
      {scaling && <div className="system-card"><h3>{t("scaling")}</h3><dl><dt>{t("database")}</dt><dd>{scaling.database_mode}</dd><dt>{t("replicas")}</dt><dd>{scaling.configured_replicas}</dd><dt>{t("jobs")}</dt><dd>{scaling.jobs.pending} pending · {scaling.jobs.running} running</dd><dt>{t("schema")}</dt><dd>v{scaling.schema_version}</dd></dl></div>}
      <div className="system-card wide"><h3>{t("audit")}</h3>{audit.length ? <div className="audit-list"><div className="audit-head"><span>{t("auditAction")}</span><span>{t("auditActor")}</span><span>{t("auditWorkspace")}</span><span>{t("auditTime")}</span></div>{audit.slice(0, 20).map((record) => <div className="audit-row" key={record.id}><code>{record.action}</code><span>{record.actor_display_name ?? (record.actor_user_id ? t("unknownActor") : t("systemActor"))}</span><span>{record.workspace_name ? `${record.workspace_name}${record.workspace_short_id ? ` · ${record.workspace_short_id}` : ""}` : record.workspace_id ? record.workspace_id.slice(0, 8) : t("organization")}</span><time dateTime={new Date(record.created_at * 1000).toISOString()}>{new Date(record.created_at * 1000).toLocaleString(locale)}</time></div>)}</div> : <p>{t("noAudit")}</p>}</div>
      {principal.system_admin && <><div className="system-card"><h3>{t("usersRoles")}</h3><button className="button" onClick={() => void newUser()}>{t("createUser")}</button><label>{t("member")}<select value={memberUser} onChange={(event) => setMemberUser(event.target.value)}><option value="">{t("chooseUser")}</option>{users.map((user) => <option key={user.id} value={user.id}>{user.display_name}</option>)}</select></label><label>{t("role")}<select value={memberRole} onChange={(event) => setMemberRole(event.target.value as Role)}><option value="member">member</option><option value="organization_admin">organization_admin</option></select></label><button className="button" disabled={!memberUser} onClick={() => void grantMember()}>{t("saveMembership")}</button><button className="button" disabled={!memberUser} onClick={() => void editUserQuota()}>{t("editUserQuota")}</button><button className="button danger" disabled={!memberUser} onClick={() => void revokeMember()}>{t("revokeMembership")}</button></div><div className="system-card"><h3>{t("imageAllowlist")} · Contract v1</h3><label>{t("ociImage")}<input value={image} onChange={(event) => setImage(event.target.value)} placeholder="registry/image@sha256:…" /></label><button className="button" disabled={!image.trim()} onClick={() => void allowImage()}>{t("allowImage")}</button><div className="state-bars">{images.map((item) => <div key={item.image}><code>{item.image}</code><strong>{item.enabled ? t("enabled") : t("disabled")}</strong></div>)}</div></div><div className="system-card"><h3>{t("createOrganization")}</h3><label>{t("name")}<input value={organizationName} onChange={(event) => setOrganizationName(event.target.value)} /></label><button className="button" disabled={!organizationName.trim()} onClick={() => void createOrganization()}>{t("createOrganization")}</button></div></>}
      <div className="system-card wide template-system-card"><TemplateEditor api={api} organizationId={organizationId} templates={templates} canGrantClusterAccess={principal.system_admin} onRefresh={refresh} onError={onError} /></div>
      <div className="system-card wide"><h3>{t("webhook")}</h3><button className="button" onClick={() => void newWebhook()}>{t("addWebhook")}</button>{webhooks.length ? <div className="state-bars">{webhooks.map((hook) => <div key={hook.id}><span>{hook.event_prefix}</span><code>{hook.url}</code></div>)}</div> : <p>{t("noWebhooks")}</p>}</div>
    </div>
  </section>;
}

function message(error: unknown) { return error instanceof Error ? error.message : "请求失败"; }

class InvalidResourceDraft extends Error {}

function resourceDraft(resources: Resources): ResourceDraft {
  return {
    cpu_millis: String(resources.cpu_millis),
    memory_mib: String(resources.memory_mib),
    gpu_count: String(resources.gpu_count),
    disk_gib: String(resources.disk_gib),
  };
}

function parseResourceDraft(draft: ResourceDraft): Resources {
  return {
    cpu_millis: parseResourceValue(draft.cpu_millis, 100, 100),
    memory_mib: parseResourceValue(draft.memory_mib, 128, 128),
    gpu_count: parseResourceValue(draft.gpu_count, 0, 1),
    disk_gib: parseResourceValue(draft.disk_gib, 1, 1),
  };
}

function parseResourceValue(value: string, min: number, step: number) {
  const parsed = Number(value);
  if (!value || !Number.isSafeInteger(parsed) || parsed < min || (parsed - min) % step !== 0) throw new InvalidResourceDraft();
  return parsed;
}
