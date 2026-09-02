import { useEffect, useState } from "react";
import { CreateUserForm } from "./admin/CreateUserForm";
import type { ApiClient } from "./api";
import { OrganizationManager } from "./OrganizationManager";
import { canManageOrganization, canManageSystem } from "./permissions";
import { UsersDirectory } from "./UsersDirectory";
import { useI18n } from "./i18n";
import { TemplateEditor } from "./TemplateEditor";
import type {
  ImagePolicy,
  Organization,
  Principal,
  Resources,
  ScalingStatus,
  WebhookSubscription,
  WorkspaceResponse,
  WorkspaceTemplate,
} from "./types";

interface ResourceDraft {
  cpu_millis: string;
  memory_mib: string;
  gpu_count: string;
  disk_gib: string;
}

const DEFAULT_QUOTA: Resources = { cpu_millis: 4000, memory_mib: 8192, gpu_count: 0, disk_gib: 100 };

interface AdminPanelProps {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  workspaces: WorkspaceResponse[];
  onError: (message: string) => void;
  onOrganizationsChanged: (preferredOrganizationId?: string) => Promise<void>;
}

export function AdminPanel({ api, principal, organizationId, workspaces, onError, onOrganizationsChanged }: AdminPanelProps) {
  const { t } = useI18n();
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [images, setImages] = useState<ImagePolicy[]>([]);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [webhooks, setWebhooks] = useState<WebhookSubscription[]>([]);
  const [scaling, setScaling] = useState<ScalingStatus | null>(null);
  const [quota, setQuota] = useState<Resources | null>(null);
  const [image, setImage] = useState("");
  const [organizationName, setOrganizationName] = useState("");
  const [currentOrganizationName, setCurrentOrganizationName] = useState("");
  const [editingQuota, setEditingQuota] = useState(false);
  const [quotaDraft, setQuotaDraft] = useState<ResourceDraft>(() => resourceDraft(DEFAULT_QUOTA));
  const [directoryVersion, setDirectoryVersion] = useState(0);
  const [showCreateUser, setShowCreateUser] = useState(false);
  const canManageQuota = canManageOrganization(principal, organizationId, "manage_organization");
  const canManageMembers = canManageOrganization(principal, organizationId, "manage_members");
  const canManageGlobalState = canManageSystem(principal);
  const currentOrganization = organizations.find((organization) => organization.id === organizationId);
  const states = workspaces.reduce<Record<string, number>>((sum, item) => ({ ...sum, [item.workspace.state]: (sum[item.workspace.state] ?? 0) + 1 }), {});

  async function refresh() {
    try {
      const [managedResources, organizationPage] = await Promise.all([
        canManageQuota
          ? Promise.all([api.quota(organizationId), api.templates(organizationId), api.webhooks(organizationId)])
          : Promise.resolve([null, [], []] as [Resources | null, WorkspaceTemplate[], WebhookSubscription[]]),
        api.organizationsPage({ limit: 200 }),
      ]);
      const [currentQuota, visibleTemplates, subscriptions] = managedResources;
      setQuota(currentQuota);
      setQuotaDraft(resourceDraft(currentQuota ?? DEFAULT_QUOTA));
      setTemplates(visibleTemplates);
      setWebhooks(subscriptions);
      setOrganizations(organizationPage.items);
      if (canManageGlobalState) {
        const [allImages, status] = await Promise.all([api.images(), api.scaling()]);
        setImages(allImages);
        setScaling(status);
      }
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  useEffect(() => { void refresh(); }, [api, organizationId, canManageGlobalState, canManageQuota]);

  useEffect(() => {
    setCurrentOrganizationName(currentOrganization?.name ?? "");
  }, [currentOrganization?.id, currentOrganization?.name]);

  async function allowImage() {
    try {
      await api.putImage(image.trim());
      setImage("");
      await refresh();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function createOrganization() {
    try {
      const organization = await api.createOrganization(organizationName.trim());
      setOrganizationName("");
      await onOrganizationsChanged(organization.id);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function saveOrganization() {
    const nextName = currentOrganizationName.trim();
    if (!nextName || nextName === currentOrganization?.name) return;
    try {
      await api.updateOrganization(organizationId, nextName);
      await onOrganizationsChanged(organizationId);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function deleteOrganization() {
    if (!confirm(t("deleteOrganizationConfirm"))) return;
    try {
      await api.deleteOrganization(organizationId);
      await onOrganizationsChanged();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function saveQuota() {
    try {
      await api.setQuota(organizationId, parseResourceDraft(quotaDraft));
      setEditingQuota(false);
      await refresh();
    } catch (error) {
      onError(error instanceof InvalidResourceDraft ? t("invalidTemplateNumber") : message(error, t("requestFailed")));
    }
  }

  async function editUserQuota(userId: string) {
    try {
      const current = await api.userQuota(userId);
      const cpu = prompt(t("userCpuQuotaPrompt"), String(current?.cpu_millis ?? 4000));
      const memory = prompt(t("userMemoryQuotaPrompt"), String(current?.memory_mib ?? 8192));
      const gpu = prompt(t("userGpuQuotaPrompt"), String(current?.gpu_count ?? 0));
      const disk = prompt(t("userDiskQuotaPrompt"), String(current?.disk_gib ?? 100));
      if ([cpu, memory, gpu, disk].some((value) => value === null)) return;
      await api.setUserQuota(userId, { cpu_millis: Number(cpu), memory_mib: Number(memory), gpu_count: Number(gpu), disk_gib: Number(disk) });
      await refresh();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function newWebhook() {
    const url = prompt(t("webhookUrlPrompt"));
    const secret = prompt(t("webhookSecretPrompt"));
    if (!url || !secret) return;
    try {
      await api.createWebhook({ organization_id: organizationId, url, event_prefix: "workspace.", signing_secret: secret });
      await refresh();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  return <section className="panel-stack">
    <div className="section-heading">
      <div><p className="eyebrow">{t("administrationEyebrow")}</p><h2>{t("administrationTitle")}</h2></div>
      <a className="button" href="/api/v1/openapi.json" target="_blank" rel="noreferrer">{t("openApi")}</a>
    </div>

    <div className="system-grid">
      <IdentityQuotaCard
        principal={principal}
        quota={quota}
        canManageQuota={canManageQuota}
        editingQuota={editingQuota}
        quotaDraft={quotaDraft}
        onEdit={() => setEditingQuota(true)}
        onCancel={() => setEditingQuota(false)}
        onSave={() => void saveQuota()}
        onDraftChange={setQuotaDraft}
      />
      <div className="system-card">
        <h3>{t("workspaceState")}</h3>
        <div className="state-bars">{Object.entries(states).map(([state, count]) => <div key={state}><span>{workspaceStateLabel(state, t)}</span><strong>{count}</strong></div>)}</div>
      </div>
      {scaling && <div className="system-card"><h3>{t("scaling")}</h3><dl><dt>{t("database")}</dt><dd>{scaling.database_mode}</dd><dt>{t("replicas")}</dt><dd>{scaling.configured_replicas}</dd><dt>{t("jobs")}</dt><dd>{scaling.jobs.pending} {t("pendingJobs")} · {scaling.jobs.running} {t("runningJobs")}</dd><dt>{t("schema")}</dt><dd>v{scaling.schema_version}</dd></dl></div>}

      <OrganizationManager
        organization={currentOrganization}
        organizationName={currentOrganizationName}
        newOrganizationName={organizationName}
        canCreate={canManageGlobalState}
        canEdit={canManageQuota}
        canDelete={canManageGlobalState}
        onOrganizationNameChange={setCurrentOrganizationName}
        onNewOrganizationNameChange={setOrganizationName}
        onSave={() => void saveOrganization()}
        onDelete={() => void deleteOrganization()}
        onCreate={() => void createOrganization()}
      />

      {canManageMembers && <>
        <div className="system-card wide">
          <div className="card-heading"><h3>{t("usersRoles")}</h3>{canManageGlobalState && <button className="button" onClick={() => setShowCreateUser((visible) => !visible)}>{showCreateUser ? t("collapse") : t("createUser")}</button>}</div>
          {canManageGlobalState && showCreateUser && <CreateUserForm api={api} principal={principal} organizationId={organizationId} onCancel={() => setShowCreateUser(false)} onError={onError} onCreated={async () => { setShowCreateUser(false); setDirectoryVersion((value) => value + 1); await refresh(); }} />}
          <UsersDirectory api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageGlobalState} canEditQuota={canManageGlobalState} refreshVersion={directoryVersion} onError={onError} onEditQuota={(userId) => void editUserQuota(userId)} />
        </div>
      </>}
      {canManageGlobalState && <ImageAllowlist images={images} image={image} onImageChange={setImage} onAllow={() => void allowImage()} />}

      {canManageQuota && <div className="system-card wide template-system-card"><TemplateEditor api={api} organizationId={organizationId} templates={templates} canGrantClusterAccess={canManageGlobalState} onRefresh={refresh} onError={onError} /></div>}
      {canManageQuota && <div className="system-card wide"><h3>{t("webhook")}</h3><button className="button" onClick={() => void newWebhook()}>{t("addWebhook")}</button>{webhooks.length ? <div className="state-bars">{webhooks.map((hook) => <div key={hook.id}><span>{hook.event_prefix}</span><code>{hook.url}</code></div>)}</div> : <p>{t("noWebhooks")}</p>}</div>}
    </div>
  </section>;
}

function IdentityQuotaCard({
  principal,
  quota,
  canManageQuota,
  editingQuota,
  quotaDraft,
  onEdit,
  onCancel,
  onSave,
  onDraftChange,
}: {
  principal: Principal;
  quota: Resources | null;
  canManageQuota: boolean;
  editingQuota: boolean;
  quotaDraft: ResourceDraft;
  onEdit: () => void;
  onCancel: () => void;
  onSave: () => void;
  onDraftChange: (draft: ResourceDraft) => void;
}) {
  const { t } = useI18n();
  return <div className="system-card">
    <h3>{t("identityQuota")}</h3>
    <dl><dt>{t("user")}</dt><dd>{principal.display_name}</dd><dt>{t("systemAdmin")}</dt><dd>{principal.system_admin ? t("enabled") : t("disabled")}</dd><dt>{t("orgQuota")}</dt><dd>{quota ? `${quota.cpu_millis}m / ${quota.memory_mib}Mi / ${quota.disk_gib}Gi / ${quota.gpu_count} ${t("gpu")}` : t("notEnabled")}</dd></dl>
    {editingQuota ? <div className="quota-editor">
      <label>{t("cpu")} (m)<input type="number" inputMode="numeric" min="100" step="100" value={quotaDraft.cpu_millis} onChange={(event) => onDraftChange({ ...quotaDraft, cpu_millis: event.target.value })} /></label>
      <label>{t("memory")} (MiB)<input type="number" inputMode="numeric" min="128" step="128" value={quotaDraft.memory_mib} onChange={(event) => onDraftChange({ ...quotaDraft, memory_mib: event.target.value })} /></label>
      <label>{t("gpu")}<input type="number" inputMode="numeric" min="0" step="1" value={quotaDraft.gpu_count} onChange={(event) => onDraftChange({ ...quotaDraft, gpu_count: event.target.value })} /></label>
      <label>{t("disk")} (GiB)<input type="number" inputMode="numeric" min="1" step="1" value={quotaDraft.disk_gib} onChange={(event) => onDraftChange({ ...quotaDraft, disk_gib: event.target.value })} /></label>
      <div className="quota-actions"><button className="button primary" onClick={onSave}>{t("saveQuota")}</button><button className="button" onClick={onCancel}>{t("cancel")}</button></div>
    </div> : <><button className="button" disabled={!canManageQuota} title={canManageQuota ? undefined : t("noQuotaPermission")} onClick={onEdit}>{t("editQuota")}</button>{!canManageQuota && <p className="security-note">{t("noQuotaPermission")}</p>}</>}
  </div>;
}

function ImageAllowlist({ images, image, onImageChange, onAllow }: { images: ImagePolicy[]; image: string; onImageChange: (value: string) => void; onAllow: () => void }) {
  const { t } = useI18n();
  return <div className="system-card"><h3>{t("imageAllowlist")} · {t("imageContract")}</h3><label>{t("ociImage")}<input value={image} onChange={(event) => onImageChange(event.target.value)} placeholder={t("imagePlaceholder")} /></label><button className="button" disabled={!image.trim()} onClick={onAllow}>{t("allowImage")}</button><div className="state-bars">{images.map((item) => <div key={item.image}><code>{item.image}</code><strong>{item.enabled ? t("enabled") : t("disabled")}</strong></div>)}</div></div>;
}

function workspaceStateLabel(state: string, t: (key: "stateProvisioning" | "stateReady" | "stateStopping" | "stateStopped" | "stateStarting" | "stateRestarting" | "stateDeleting" | "stateDeleted" | "stateFailed") => string) {
  const labels = {
    provisioning: "stateProvisioning",
    ready: "stateReady",
    stopping: "stateStopping",
    stopped: "stateStopped",
    starting: "stateStarting",
    restarting: "stateRestarting",
    deleting: "stateDeleting",
    deleted: "stateDeleted",
    failed: "stateFailed",
  } as const;
  return state in labels ? t(labels[state as keyof typeof labels]) : state;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

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
