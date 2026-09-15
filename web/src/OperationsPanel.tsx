import { useEffect, useState } from "react";
import {
  Button,
  DataGrid,
  DataGridBody,
  DataGridCell,
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
  Option,
  Select,
  Spinner,
  Text,
} from "@fluentui/react-components";
import type { TableColumnDefinition } from "@fluentui/react-components";
import { AddRegular, OpenRegular, SaveRegular } from "@fluentui/react-icons";

import { CreateUserForm } from "./admin/CreateUserForm";
import { workspaceStateCounts } from "./admin/workspaceSummary";
import { AdminCard, AdminToolbar, SaveButton, useAdminStyles } from "./admin/fluentAdmin";
import type { ApiClient } from "./api";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { canManageOrganization, canManageSystem } from "./permissions";
import { UsersDirectory } from "./UsersDirectory";
import { useI18n } from "./i18n";
import { OrganizationManager } from "./OrganizationManager";
import { TemplateEditor } from "./TemplateEditor";
import type {
  ImagePolicy,
  Organization,
  Principal,
  QuotaResources,
  ScalingStatus,
  WebhookSubscription,
  WorkspaceSummary,
  WorkspaceTemplate,
} from "./types";

interface ResourceDraft {
  cpu_millis: string;
  memory_mib: string;
  gpu_count: string;
  disk_gib: string;
  temporary_storage_gib: string;
}

const DEFAULT_QUOTA: QuotaResources = { cpu_millis: 4000, memory_mib: 8192, gpu_count: 0, disk_gib: 100, temporary_storage_gib: 200 };

interface AdminPanelProps {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  onError: (message: string) => void;
  onOrganizationsChanged: (preferredOrganizationId?: string) => Promise<void>;
}

export function AdminPanel({ api, principal, organizationId, onError, onOrganizationsChanged }: AdminPanelProps) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [images, setImages] = useState<ImagePolicy[]>([]);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [webhooks, setWebhooks] = useState<WebhookSubscription[]>([]);
  const [scaling, setScaling] = useState<ScalingStatus | null>(null);
  const [quota, setQuota] = useState<QuotaResources | null>(null);
  const [image, setImage] = useState("");
  const [organizationName, setOrganizationName] = useState("");
  const [newOrganizationName, setNewOrganizationName] = useState("");
  const [editingQuota, setEditingQuota] = useState(false);
  const [quotaDraft, setQuotaDraft] = useState<ResourceDraft>(() => resourceDraft(DEFAULT_QUOTA));
  const [directoryVersion, setDirectoryVersion] = useState(0);
  const [showCreateUser, setShowCreateUser] = useState(false);
  const [workspaceSummary, setWorkspaceSummary] = useState<WorkspaceSummary | null>(null);
  const [confirmOrganizationDelete, setConfirmOrganizationDelete] = useState(false);
  const [userQuotaTarget, setUserQuotaTarget] = useState<string | null>(null);
  const [userQuotaDraft, setUserQuotaDraft] = useState<ResourceDraft>(() => resourceDraft(DEFAULT_QUOTA));
  const [showWebhookForm, setShowWebhookForm] = useState(false);
  const [webhookUrl, setWebhookUrl] = useState("");
  const [webhookSecret, setWebhookSecret] = useState("");
  const [dialogBusy, setDialogBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const canManageQuota = canManageOrganization(principal, organizationId, "manage_organization");
  const canManageMembers = canManageOrganization(principal, organizationId, "manage_members");
  const canManageGlobalState = canManageSystem(principal);
  const currentOrganization = organizations.find((organization) => organization.id === organizationId);
  const states = workspaceStateCounts(workspaceSummary);

  async function refresh() {
    setLoading(true);
    try {
      const [managedResources, organizationPage, workspacePage] = await Promise.all([
        canManageQuota
          ? Promise.all([api.quota(organizationId), api.templates(organizationId), api.webhooks(organizationId)])
          : Promise.resolve([null, [], []] as [QuotaResources | null, WorkspaceTemplate[], WebhookSubscription[]]),
        api.organizationsPage({ limit: 200 }),
        api.workspacesPage(organizationId, { limit: 1 }),
      ]);
      const [currentQuota, visibleTemplates, subscriptions] = managedResources;
      setQuota(currentQuota);
      setQuotaDraft(resourceDraft(currentQuota ?? DEFAULT_QUOTA));
      setTemplates(visibleTemplates);
      setWebhooks(subscriptions);
      setOrganizations(organizationPage.items);
      setWorkspaceSummary(workspacePage.summary);
      if (canManageGlobalState) {
        const [allImages, status] = await Promise.all([api.images(), api.scaling()]);
        setImages(allImages);
        setScaling(status);
      }
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void refresh(); }, [api, organizationId, canManageGlobalState, canManageQuota]);

  useEffect(() => {
    setOrganizationName(currentOrganization?.name ?? "");
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
      const organization = await api.createOrganization(newOrganizationName.trim());
      setNewOrganizationName("");
      await onOrganizationsChanged(organization.id);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function saveOrganization() {
    const nextName = organizationName.trim();
    if (!nextName || nextName === currentOrganization?.name) return;
    try {
      await api.updateOrganization(organizationId, nextName);
      await onOrganizationsChanged(organizationId);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function deleteOrganization() {
    setDialogBusy(true);
    try {
      await api.deleteOrganization(organizationId);
      setConfirmOrganizationDelete(false);
      await onOrganizationsChanged();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setDialogBusy(false);
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
      setUserQuotaDraft(resourceDraft(current ?? DEFAULT_QUOTA));
      setUserQuotaTarget(userId);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function saveUserQuota() {
    if (!userQuotaTarget) return;
    setDialogBusy(true);
    try {
      await api.setUserQuota(userQuotaTarget, parseResourceDraft(userQuotaDraft));
      setUserQuotaTarget(null);
      await refresh();
    } catch (error) {
      onError(error instanceof InvalidResourceDraft ? t("invalidTemplateNumber") : message(error, t("requestFailed")));
    } finally {
      setDialogBusy(false);
    }
  }

  async function newWebhook() {
    setDialogBusy(true);
    try {
      await api.createWebhook({ organization_id: organizationId, url: webhookUrl.trim(), event_prefix: "workspace.", signing_secret: webhookSecret });
      setShowWebhookForm(false);
      setWebhookUrl("");
      setWebhookSecret("");
      await refresh();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setDialogBusy(false);
    }
  }

  return <div className={styles.page}>
    <AdminToolbar action={<Button as="a" href="/api/v1/openapi.json" target="_blank" rel="noreferrer" icon={<OpenRegular />}>{t("openApi")}</Button>}>
      <div className={styles.stack}><Text size={600} weight="semibold">{t("administrationTitle")}</Text><Text className={styles.muted}>{currentOrganization?.name ?? t("organizationUnavailable")}</Text></div>
      {loading && <Spinner size="tiny" label={t("loading")} />}
    </AdminToolbar>

    <div className={styles.sectionGrid}>
      <IdentityQuotaCard quota={quota} principal={principal} canManageQuota={canManageQuota} editingQuota={editingQuota} quotaDraft={quotaDraft} onEdit={() => setEditingQuota(true)} onCancel={() => setEditingQuota(false)} onSave={() => void saveQuota()} onDraftChange={setQuotaDraft} />
      <AdminCard title={t("workspaceState")}>
        <DataGrid items={Object.entries(states) as [string, number][]} columns={stateColumns(t)}>
          <DataGridHeader><DataGridRow<[string, number]>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader>
          <DataGridBody<[string, number]>>{({ item }) => <DataGridRow<[string, number]>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody>
        </DataGrid>
      </AdminCard>
      {scaling && <AdminCard title={t("scaling")}><dl className={styles.stack}><div><Text weight="semibold">{t("database")}</Text><Text>{scaling.database_mode}</Text></div><div><Text weight="semibold">{t("replicas")}</Text><Text>{scaling.configured_replicas}</Text></div><div><Text weight="semibold">{t("jobs")}</Text><Text>{scaling.jobs.pending} {t("pendingJobs")} · {scaling.jobs.running} {t("runningJobs")}</Text></div><div><Text weight="semibold">{t("schema")}</Text><Text>v{scaling.schema_version}</Text></div></dl></AdminCard>}
      <OrganizationManager organization={currentOrganization} organizationName={organizationName} newOrganizationName={newOrganizationName} canCreate={canManageGlobalState} canEdit={canManageQuota} canDelete={canManageGlobalState} onOrganizationNameChange={setOrganizationName} onNewOrganizationNameChange={setNewOrganizationName} onSave={() => void saveOrganization()} onDelete={() => setConfirmOrganizationDelete(true)} onCreate={() => void createOrganization()} />

      {canManageMembers && <div className={styles.wide}><AdminCard title={t("usersRoles")} action={canManageGlobalState ? <Button icon={<AddRegular />} onClick={() => setShowCreateUser((visible) => !visible)}>{showCreateUser ? t("collapse") : t("createUser")}</Button> : undefined}>{canManageGlobalState && showCreateUser && <CreateUserForm api={api} principal={principal} organizationId={organizationId} onCancel={() => setShowCreateUser(false)} onError={onError} onCreated={async () => { setShowCreateUser(false); setDirectoryVersion((value) => value + 1); await refresh(); }} />}<UsersDirectory api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageGlobalState} canEditQuota={canManageGlobalState} refreshVersion={directoryVersion} onError={onError} onEditQuota={(userId) => void editUserQuota(userId)} /></AdminCard></div>}
      {canManageGlobalState && <div className={styles.wide}><ImageAllowlist images={images} image={image} onImageChange={setImage} onAllow={() => void allowImage()} /></div>}
      {canManageQuota && <div className={styles.wide}><AdminCard title={t("templates")}><TemplateEditor api={api} organizationId={organizationId} templates={templates} canGrantClusterAccess={canManageGlobalState} onRefresh={refresh} onError={onError} /></AdminCard></div>}
      {canManageQuota && <div className={styles.wide}><AdminCard title={t("webhook")} action={<Button icon={<AddRegular />} onClick={() => setShowWebhookForm(true)}>{t("addWebhook")}</Button>}><DataGrid items={webhooks} columns={webhookColumns(t)}><DataGridHeader><DataGridRow<WebhookSubscription>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader><DataGridBody<WebhookSubscription>>{({ item }) => <DataGridRow<WebhookSubscription>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody></DataGrid>{webhooks.length === 0 && <Text>{t("noWebhooks")}</Text>}</AdminCard></div>}
    </div>

    <ConfirmDialog open={confirmOrganizationDelete} title={t("deleteOrganization")} description={t("deleteOrganizationConfirm")} confirmLabel={t("deleteOrganization")} cancelLabel={t("cancel")} busy={dialogBusy} danger details={currentOrganization && <strong>{currentOrganization.name}</strong>} onClose={() => setConfirmOrganizationDelete(false)} onConfirm={() => void deleteOrganization()} />
    <Dialog open={userQuotaTarget !== null} onOpenChange={(_, data) => { if (!data.open && !dialogBusy) setUserQuotaTarget(null); }}><DialogSurface><DialogBody><DialogTitle>{t("editUserQuota")}</DialogTitle><DialogContent className={styles.dialogBody}><div className={styles.formGrid}><ResourceInput label={t("userCpuQuotaPrompt")} value={userQuotaDraft.cpu_millis} min={100} step={100} onChange={(cpu_millis) => setUserQuotaDraft({ ...userQuotaDraft, cpu_millis })} /><ResourceInput label={t("userMemoryQuotaPrompt")} value={userQuotaDraft.memory_mib} min={128} step={128} onChange={(memory_mib) => setUserQuotaDraft({ ...userQuotaDraft, memory_mib })} /><ResourceInput label={t("userGpuQuotaPrompt")} value={userQuotaDraft.gpu_count} min={0} step={1} onChange={(gpu_count) => setUserQuotaDraft({ ...userQuotaDraft, gpu_count })} /><ResourceInput label={t("userDiskQuotaPrompt")} value={userQuotaDraft.disk_gib} min={1} step={1} onChange={(disk_gib) => setUserQuotaDraft({ ...userQuotaDraft, disk_gib })} /><ResourceInput label={t("userTemporaryStorageQuotaPrompt")} value={userQuotaDraft.temporary_storage_gib} min={0} step={1} onChange={(temporary_storage_gib) => setUserQuotaDraft({ ...userQuotaDraft, temporary_storage_gib })} /></div></DialogContent><DialogActions><Button appearance="secondary" disabled={dialogBusy} onClick={() => setUserQuotaTarget(null)}>{t("cancel")}</Button><SaveButton disabled={dialogBusy} onClick={() => void saveUserQuota()}>{dialogBusy ? t("saving") : t("saveQuota")}</SaveButton></DialogActions></DialogBody></DialogSurface></Dialog>
    <Dialog open={showWebhookForm} onOpenChange={(_, data) => setShowWebhookForm(data.open)}><DialogSurface><DialogBody><DialogTitle>{t("addWebhook")}</DialogTitle><DialogContent className={styles.dialogBody}><Field label={t("webhookUrlPrompt")} required><Input type="url" value={webhookUrl} onChange={(event) => setWebhookUrl(event.target.value)} placeholder="https://example.com/hooks/workspace" /></Field><Field label={t("webhookSecretPrompt")} required><Input type="password" minLength={32} autoComplete="new-password" value={webhookSecret} onChange={(event) => setWebhookSecret(event.target.value)} /></Field></DialogContent><DialogActions><Button appearance="secondary" disabled={dialogBusy} onClick={() => setShowWebhookForm(false)}>{t("cancel")}</Button><SaveButton icon={<SaveRegular />} disabled={dialogBusy || !webhookUrl.trim() || webhookSecret.length < 32} onClick={() => void newWebhook()}>{dialogBusy ? t("saving") : t("addWebhook")}</SaveButton></DialogActions></DialogBody></DialogSurface></Dialog>
  </div>;
}

function IdentityQuotaCard({ principal, quota, canManageQuota, editingQuota, quotaDraft, onEdit, onCancel, onSave, onDraftChange }: { principal: Principal; quota: QuotaResources | null; canManageQuota: boolean; editingQuota: boolean; quotaDraft: ResourceDraft; onEdit: () => void; onCancel: () => void; onSave: () => void; onDraftChange: (draft: ResourceDraft) => void }) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  return <AdminCard title={t("identityQuota")}><dl className={styles.stack}><div><Text weight="semibold">{t("user")}</Text><Text>{principal.display_name}</Text></div><div><Text weight="semibold">{t("systemAdmin")}</Text><Text>{principal.system_admin ? t("enabled") : t("disabled")}</Text></div><div><Text weight="semibold">{t("orgQuota")}</Text><Text>{quota ? `${quota.cpu_millis}m / ${quota.memory_mib}Mi / ${quota.disk_gib}Gi / ${quota.gpu_count} ${t("gpu")} / ${quota.temporary_storage_gib}Gi ${t("temporaryStorage")}` : t("notEnabled")}</Text></div></dl>{editingQuota ? <div className={styles.stack}><div className={styles.formGrid}><ResourceInput label={`${t("cpu")} (m)`} value={quotaDraft.cpu_millis} min={100} step={100} onChange={(cpu_millis) => onDraftChange({ ...quotaDraft, cpu_millis })} /><ResourceInput label={`${t("memory")} (MiB)`} value={quotaDraft.memory_mib} min={128} step={128} onChange={(memory_mib) => onDraftChange({ ...quotaDraft, memory_mib })} /><ResourceInput label={t("gpu")} value={quotaDraft.gpu_count} min={0} step={1} onChange={(gpu_count) => onDraftChange({ ...quotaDraft, gpu_count })} /><ResourceInput label={`${t("disk")} (GiB)`} value={quotaDraft.disk_gib} min={1} step={1} onChange={(disk_gib) => onDraftChange({ ...quotaDraft, disk_gib })} /><ResourceInput label={`${t("temporaryStorage")} (GiB)`} value={quotaDraft.temporary_storage_gib} min={0} step={1} onChange={(temporary_storage_gib) => onDraftChange({ ...quotaDraft, temporary_storage_gib })} /></div><div className={styles.actions}><SaveButton onClick={onSave}>{t("saveQuota")}</SaveButton><Button appearance="secondary" onClick={onCancel}>{t("cancel")}</Button></div></div> : <Button disabled={!canManageQuota} title={canManageQuota ? undefined : t("noQuotaPermission")} onClick={onEdit}>{t("editQuota")}</Button>}</AdminCard>;
}

function ResourceInput({ label, value, min, step, onChange }: { label: string; value: string; min: number; step: number; onChange: (value: string) => void }) {
  return <Field label={label} required><Input type="number" inputMode="numeric" min={min} step={step} value={value} onChange={(event) => onChange(event.target.value)} /></Field>;
}

function ImageAllowlist({ images, image, onImageChange, onAllow }: { images: ImagePolicy[]; image: string; onImageChange: (value: string) => void; onAllow: () => void }) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  return <AdminCard title={t("imageAllowlist")} action={<SaveButton icon={<SaveRegular />} disabled={!image.trim()} onClick={onAllow}>{t("allowImage")}</SaveButton>}><Field label={t("ociImage")}><Input value={image} onChange={(event) => onImageChange(event.target.value)} placeholder={t("imagePlaceholder")} /></Field><div className={styles.table}><DataGrid items={images} columns={imageColumns(t)}><DataGridHeader><DataGridRow<ImagePolicy>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader><DataGridBody<ImagePolicy>>{({ item }) => <DataGridRow<ImagePolicy>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody></DataGrid></div></AdminCard>;
}

function stateColumns(t: ReturnType<typeof useI18n>["t"]): TableColumnDefinition<[string, number]>[] {
  return [
    { columnId: "state", compare: (a, b) => a[0].localeCompare(b[0]), renderHeaderCell: () => t("workspaceState"), renderCell: (item) => workspaceStateLabel(item[0], t) },
    { columnId: "count", compare: (a, b) => a[1] - b[1], renderHeaderCell: () => t("stateCount"), renderCell: (item) => item[1] },
  ];
}

function webhookColumns(t: ReturnType<typeof useI18n>["t"]): TableColumnDefinition<WebhookSubscription>[] {
  return [
    { columnId: "webhook", compare: (a, b) => a.event_prefix.localeCompare(b.event_prefix), renderHeaderCell: () => t("webhook"), renderCell: (item) => item.event_prefix },
    { columnId: "url", compare: (a, b) => a.url.localeCompare(b.url), renderHeaderCell: () => t("webhookUrlPrompt"), renderCell: (item) => item.url },
  ];
}

function imageColumns(t: ReturnType<typeof useI18n>["t"]): TableColumnDefinition<ImagePolicy>[] {
  return [
    { columnId: "image", compare: (a, b) => a.image.localeCompare(b.image), renderHeaderCell: () => t("ociImage"), renderCell: (item) => item.image },
    { columnId: "enabled", compare: (a, b) => Number(a.enabled) - Number(b.enabled), renderHeaderCell: () => t("enabled"), renderCell: (item) => item.enabled ? t("enabled") : t("disabled") },
  ];
}

function workspaceStateLabel(state: string, t: (key: "stateProvisioning" | "stateReady" | "stateStopping" | "stateStopped" | "stateStarting" | "stateRestarting" | "stateDeleting" | "stateDeleted" | "stateFailed") => string) {
  const labels = { provisioning: "stateProvisioning", ready: "stateReady", stopping: "stateStopping", stopped: "stateStopped", starting: "stateStarting", restarting: "stateRestarting", deleting: "stateDeleting", deleted: "stateDeleted", failed: "stateFailed" } as const;
  return state in labels ? t(labels[state as keyof typeof labels]) : state;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

class InvalidResourceDraft extends Error {}

function resourceDraft(resources: QuotaResources): ResourceDraft {
  return { cpu_millis: String(resources.cpu_millis), memory_mib: String(resources.memory_mib), gpu_count: String(resources.gpu_count), disk_gib: String(resources.disk_gib), temporary_storage_gib: String(resources.temporary_storage_gib) };
}

function parseResourceDraft(draft: ResourceDraft): QuotaResources {
  return { cpu_millis: parseResourceValue(draft.cpu_millis, 100, 100), memory_mib: parseResourceValue(draft.memory_mib, 128, 128), gpu_count: parseResourceValue(draft.gpu_count, 0, 1), disk_gib: parseResourceValue(draft.disk_gib, 1, 1), temporary_storage_gib: parseResourceValue(draft.temporary_storage_gib, 0, 1) };
}

function parseResourceValue(value: string, min: number, step: number) {
  const parsed = Number(value);
  if (!value || !Number.isSafeInteger(parsed) || parsed < min || (parsed - min) % step !== 0) throw new InvalidResourceDraft();
  return parsed;
}
