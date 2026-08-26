import { useEffect, useState } from "react";
import type { ApiClient } from "./api";
import type { AuditRecord, ImagePolicy, Principal, Resources, Role, ScalingStatus, UserSummary, WebhookSubscription, WorkspaceResponse, WorkspaceTemplate } from "./types";

export function OperationsPanel({ api, principal, organizationId, workspaces, onError }: { api: ApiClient; principal: Principal; organizationId: string; workspaces: WorkspaceResponse[]; onError: (message: string) => void }) {
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
  const states = workspaces.reduce<Record<string, number>>((sum, item) => ({ ...sum, [item.workspace.state]: (sum[item.workspace.state] ?? 0) + 1 }), {});

  async function refresh() {
    try {
      const common = [api.audit(organizationId), api.quota(organizationId), api.templates(organizationId), api.webhooks(organizationId)] as const;
      const [records, currentQuota, visibleTemplates, subscriptions] = await Promise.all(common);
      setAudit(records); setQuota(currentQuota); setTemplates(visibleTemplates); setWebhooks(subscriptions);
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
    if (!memberUser || !confirm("撤销该用户的组织成员关系和新 SSH 连接权限？")) return;
    try { await api.removeMembership(organizationId, memberUser); setMemberUser(""); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function editQuota() {
    const cpu = prompt("CPU 总额度（millicores）", String(quota?.cpu_millis ?? 4000));
    const memory = prompt("内存总额度（MiB）", String(quota?.memory_mib ?? 8192));
    const gpu = prompt("GPU 总额度", String(quota?.gpu_count ?? 0));
    const disk = prompt("磁盘总额度（GiB）", String(quota?.disk_gib ?? 100));
    if ([cpu, memory, gpu, disk].some((value) => value === null)) return;
    try { await api.setQuota(organizationId, { cpu_millis: Number(cpu), memory_mib: Number(memory), gpu_count: Number(gpu), disk_gib: Number(disk) }); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function editUserQuota() {
    if (!memberUser) return;
    try {
      const current = await api.userQuota(memberUser);
      const cpu = prompt("用户 CPU 总额度（millicores）", String(current?.cpu_millis ?? 4000));
      const memory = prompt("用户内存总额度（MiB）", String(current?.memory_mib ?? 8192));
      const gpu = prompt("用户 GPU 总额度", String(current?.gpu_count ?? 0));
      const disk = prompt("用户磁盘总额度（GiB）", String(current?.disk_gib ?? 100));
      if ([cpu, memory, gpu, disk].some((value) => value === null)) return;
      await api.setUserQuota(memberUser, { cpu_millis: Number(cpu), memory_mib: Number(memory), gpu_count: Number(gpu), disk_gib: Number(disk) });
      await refresh();
    } catch (error) { onError(message(error)); }
  }

  async function newTemplate() {
    const name = prompt("模板名称"); const templateImage = prompt("已允许的 OCI 镜像");
    if (!name || !templateImage) return;
    try { await api.createTemplate({ organization_id: organizationId, name, image: templateImage, access_mode: "internal", resources: { cpu_millis: 2000, memory_mib: 4096, gpu_count: 0, disk_gib: 50 } }); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function newWebhook() {
    const url = prompt("公开 HTTPS Webhook URL"); const secret = prompt("签名密钥（至少 32 字节）");
    if (!url || !secret) return;
    try { await api.createWebhook({ organization_id: organizationId, url, event_prefix: "workspace.", signing_secret: secret }); await refresh(); } catch (error) { onError(message(error)); }
  }

  async function newUser() {
    const name = prompt("用户显示名"); const token = prompt("初始 API Token（至少 32 字节，不会回显）");
    if (!name || !token) return;
    try { await api.createUser(name, token); await refresh(); } catch (error) { onError(message(error)); }
  }

  return <section className="panel-stack">
    <div className="section-heading"><div><p className="eyebrow">OPERATIONS</p><h2>系统与审计</h2></div><a className="button" href="/api/v1/openapi.json" target="_blank">OpenAPI</a></div>
    <div className="system-grid">
      <div className="system-card"><h3>身份与额度</h3><dl><dt>用户</dt><dd>{principal.display_name}</dd><dt>系统管理员</dt><dd>{principal.system_admin ? "是" : "否"}</dd><dt>组织额度</dt><dd>{quota ? `${quota.cpu_millis}m / ${quota.memory_mib}Mi / ${quota.disk_gib}Gi` : "未启用"}</dd></dl><button className="button" onClick={() => void editQuota()}>设置额度</button></div>
      <div className="system-card"><h3>工作区状态</h3><div className="state-bars">{Object.entries(states).map(([state, count]) => <div key={state}><span>{state}</span><strong>{count}</strong></div>)}</div></div>
      {scaling && <div className="system-card"><h3>扩缩容</h3><dl><dt>数据库</dt><dd>{scaling.database_mode}</dd><dt>副本</dt><dd>{scaling.configured_replicas}</dd><dt>任务</dt><dd>{scaling.jobs.pending} pending · {scaling.jobs.running} running</dd><dt>Schema</dt><dd>v{scaling.schema_version}</dd></dl></div>}
      <div className="system-card wide"><h3>审计</h3>{audit.length ? <div className="state-bars">{audit.slice(0, 20).map((record) => <div key={record.id}><span>{record.action}<small> · {new Date(record.created_at * 1000).toLocaleString()}</small></span><code>{record.workspace_id?.slice(0, 8) ?? "organization"}</code></div>)}</div> : <p>暂无组织审计记录。</p>}</div>
      {principal.system_admin && <><div className="system-card"><h3>用户与角色</h3><button className="button" onClick={() => void newUser()}>创建用户</button><label>成员<select value={memberUser} onChange={(event) => setMemberUser(event.target.value)}><option value="">选择用户</option>{users.map((user) => <option key={user.id} value={user.id}>{user.display_name}</option>)}</select></label><label>角色<select value={memberRole} onChange={(event) => setMemberRole(event.target.value as Role)}><option value="member">member</option><option value="organization_admin">organization_admin</option></select></label><button className="button" disabled={!memberUser} onClick={() => void grantMember()}>保存成员关系</button><button className="button" disabled={!memberUser} onClick={() => void editUserQuota()}>设置用户额度</button><button className="button danger" disabled={!memberUser} onClick={() => void revokeMember()}>撤销成员关系</button></div><div className="system-card"><h3>镜像白名单 · Contract v1</h3><label>OCI 镜像<input value={image} onChange={(event) => setImage(event.target.value)} placeholder="registry/image@sha256:…" /></label><button className="button" disabled={!image.trim()} onClick={() => void allowImage()}>允许镜像</button><div className="state-bars">{images.map((item) => <div key={item.image}><code>{item.image}</code><strong>{item.enabled ? "启用" : "停用"}</strong></div>)}</div></div><div className="system-card"><h3>创建组织</h3><label>名称<input value={organizationName} onChange={(event) => setOrganizationName(event.target.value)} /></label><button className="button" disabled={!organizationName.trim()} onClick={() => void createOrganization()}>创建</button></div></>}
      <div className="system-card wide"><h3>模板</h3><button className="button" onClick={() => void newTemplate()}>创建组织模板</button>{templates.length ? <div className="state-bars">{templates.map((template) => <div key={template.id}><span>{template.name} · {template.access_mode}</span><code>{template.image}</code></div>)}</div> : <p>暂无可用模板。</p>}</div>
      <div className="system-card wide"><h3>Webhook</h3><button className="button" onClick={() => void newWebhook()}>添加签名订阅</button>{webhooks.length ? <div className="state-bars">{webhooks.map((hook) => <div key={hook.id}><span>{hook.event_prefix}</span><code>{hook.url}</code></div>)}</div> : <p>暂无 Webhook 订阅。</p>}</div>
    </div>
  </section>;
}

function message(error: unknown) { return error instanceof Error ? error.message : "请求失败"; }
