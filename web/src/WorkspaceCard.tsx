import { useMemo, useState } from "react";
import { Badge, Button, Caption1, Divider, Spinner, Text, Title3, Tooltip } from "@fluentui/react-components";
import { ArrowClockwiseRegular, ChevronDownRegular, ChevronUpRegular, DeleteRegular, DesktopRegular, LocationRegular, PlayRegular, StopRegular, WindowConsoleRegular } from "@fluentui/react-icons";
import { useI18n } from "./i18n";
import type { Locale, MessageKey } from "./i18n";
import type { AvailableNodePool, WorkspaceResponse, WorkspaceRuntime, WorkspaceRuntimeEvent } from "./types";
import { WorkspaceConnectionDialog } from "./WorkspaceConnectionDialog";
import { WorkspacePortMappings } from "./WorkspacePortMappings";
import type { ApiClient } from "./api";
import { nodePoolDisplayName } from "./forms/NodePoolPicker";
import { safeBootstrapUrl } from "./portMappings";
import { reserveWebShellWindow } from "./workspaceShell";
import { ResourceMeter, WorkspaceStatusCard } from "./shared";
import { aggregateRuntimeUsage, formatCpuMillis, formatMemoryMiB, parseCpuMillis, parseMemoryMiB, usagePercent } from "./workspaceMetrics";
import { StorageMeter } from "./workspaces/StorageMeter";
import { useWorkspaceStyles } from "./workspaces/workspaceStyles";

type WorkspaceAction = "start" | "stop" | "restart" | "delete";
type DetailView = "status" | "events";

interface Props {
  api: ApiClient;
  item: WorkspaceResponse;
  runtime?: WorkspaceRuntime;
  nodePools: AvailableNodePool[];
  onAction: (item: WorkspaceResponse, action: WorkspaceAction) => void;
  busyAction: WorkspaceAction | null;
  onOpenShell: (id: string) => Promise<void>;
  onRequestRuntime: (id: string) => Promise<void>;
  onChangePlacement: (item: WorkspaceResponse) => void;
  onError: (message: string) => void;
  canConnect: boolean;
  canChangeState: boolean;
  canDelete: boolean;
  canChangePlacement: boolean;
  templateName?: string;
}

export function WorkspaceCard({ api, item, runtime, nodePools, busyAction, onAction, onOpenShell, onRequestRuntime, onChangePlacement, onError, canConnect, canChangeState, canDelete, canChangePlacement, templateName }: Props) {
  const { locale, t } = useI18n();
  const styles = useWorkspaceStyles();
  const [detailView, setDetailView] = useState<DetailView | null>(null);
  const [openingDesktop, setOpeningDesktop] = useState(false);
  const workspace = item.workspace;
  const titleId = `workspace-${workspace.id}-title`;
  const statusId = `workspace-${workspace.id}-runtime-status`;
  const eventsId = `workspace-${workspace.id}-event-log`;
  const toggleDetail = (view: DetailView) => {
    setDetailView((current) => current === view ? null : view);
    if (view === "events" && detailView !== "events") void onRequestRuntime(workspace.id);
  };
  const running = workspace.state === "ready";
  const stopped = workspace.state === "stopped";
  const transition = busyAction ? { action: busyAction, label: actionProgressLabel(busyAction) } : workspaceTransition(workspace.state);
  const stateLabels = {
    provisioning: t("stateProvisioning"),
    ready: t("stateReady"),
    stopping: t("stateStopping"),
    stopped: t("stateStopped"),
    starting: t("stateStarting"),
    restarting: t("stateRestarting"),
    deleting: t("stateDeleting"),
    deleted: t("stateDeleted"),
    failed: t("stateFailed"),
  };

  return <WorkspaceStatusCard
    title={workspace.name}
    titleId={titleId}
    shortId={workspace.short_id}
    state={workspace.state}
    stateLabels={stateLabels}
    metadata={[
      workspace.workspace_user,
      workspace.access_mode === "public" ? t("public") : t("internal"),
      ...(templateName ? [`${t("template")}: ${templateName}`] : []),
      <Tooltip key="image" content={workspace.image} relationship="description"><Text tabIndex={0} className={styles.metadataCode}>{t("image")}: {shortImage(workspace.image)}</Text></Tooltip>,
      <Tooltip key="node-pool" content={running ? t("locationChangeAfterStop") : t("changeLocation")} relationship="description"><Text tabIndex={0}>{t("nodePool")}: {nodePoolDisplayName(nodePools, workspace.node_pool)}</Text></Tooltip>,
      ...(runtime?.node_name ? [<Tooltip key="node-name" content={runtime.node_name} relationship="description"><Text tabIndex={0} className={styles.metadataCode}>{t("nodeName")}: {runtime.node_name}</Text></Tooltip>] : []),
      ...(workspace.resources.gpu_count > 0 ? [`${workspace.resources.gpu_count} GPU`] : []),
    ]}
    meters={[]}
    resources={<ResourceOverview item={item} runtime={runtime} locale={locale} />}
  >
    <div className={styles.toolbar} role="group" aria-labelledby={titleId} aria-busy={transition ? "true" : "false"}>
      <div className={styles.toolbarGroup}>
        {transition ? <Button appearance="subtle" disabled icon={<Spinner size="tiny" />} aria-busy="true">{t(transition.label)}</Button> : <>
          {canConnect && running && item.ssh_connection && <WorkspaceConnectionDialog connection={item.ssh_connection} />}
          {canConnect && running && <Tooltip content={t("webShellClipboardHelp")} relationship="description"><Button appearance="primary" icon={<WindowConsoleRegular />} onClick={() => void onOpenShell(workspace.id)}>{t("webShell")}</Button></Tooltip>}
          {canConnect && running && item.desktop?.status === "ready" && item.desktop.https_url && <Button appearance="outline" icon={<DesktopRegular />} disabled={openingDesktop} onClick={() => void openDesktop(api, workspace.id, item.desktop!.mapping_id, setOpeningDesktop, onError, t)}>{openingDesktop ? t("desktopOpening") : t("openDesktop")}</Button>}
          {canConnect && running && <WorkspacePortMappings api={api} workspaceId={workspace.id} workspaceReady onError={onError} />}
          {canChangeState && running && <Button appearance="subtle" icon={<StopRegular />} onClick={() => onAction(item, "stop")}>{t("stop")}</Button>}
          {canChangeState && running && <Button appearance="subtle" icon={<ArrowClockwiseRegular />} onClick={() => onAction(item, "restart")}>{t("restart")}</Button>}
          {canChangeState && (workspace.state === "stopped" || workspace.state === "failed") && <Button appearance="primary" icon={<PlayRegular />} onClick={() => onAction(item, "start")}>{t("start")}</Button>}
          {canChangePlacement && stopped && <Button appearance="subtle" icon={<LocationRegular />} onClick={() => onChangePlacement(item)}>{t("changeLocation")}</Button>}
        </>}
      </div>
      <div className={styles.toolbarGroupEnd}>
        {runtime && workspace.state !== "stopped" && <Button appearance={detailView === "status" ? "secondary" : "subtle"} icon={detailView === "status" ? <ChevronUpRegular /> : <ChevronDownRegular />} aria-expanded={detailView === "status"} aria-controls={statusId} onClick={() => toggleDetail("status")}>{t("runtimeStatus")}</Button>}
        {runtime && <Button appearance={detailView === "events" ? "secondary" : "subtle"} icon={detailView === "events" ? <ChevronUpRegular /> : <ChevronDownRegular />} aria-expanded={detailView === "events"} aria-controls={eventsId} onClick={() => toggleDetail("events")}>{t("eventLog")}</Button>}
        {canDelete && !transition && !(workspace.state === "deleting" || workspace.state === "deleted") && <Button appearance="subtle" className={styles.dangerButton} icon={<DeleteRegular />} onClick={() => onAction(item, "delete")}>{t("delete")}</Button>}
      </div>
    </div>
    {runtime && workspace.state !== "stopped" && detailView === "status" && <RuntimeStatus id={statusId} runtime={runtime} />}
    {runtime && detailView === "events" && <EventLog id={eventsId} runtime={runtime} locale={locale} />}
  </WorkspaceStatusCard>;
}

async function openDesktop(api: ApiClient, workspaceId: string, mappingId: string, setOpening: (value: boolean) => void, onError: (message: string) => void, t: ReturnType<typeof useI18n>["t"]) {
  const target = reserveWebShellWindow(undefined, t("connectionPreparing"));
  setOpening(true);
  try {
    const bootstrap = await api.bootstrapPortMapping(workspaceId, mappingId);
    const destination = safeBootstrapUrl(bootstrap.bootstrap_url);
    if (!destination) throw new Error(t("desktopUnsafeBootstrapUrl"));
    if (target) target.location.replace(destination); else window.location.assign(destination);
  } catch (error) { target?.close(); onError(error instanceof Error ? error.message : t("desktopOpenFailed")); }
  finally { setOpening(false); }
}

function ResourceOverview({ item, runtime, locale }: { item: WorkspaceResponse; runtime?: WorkspaceRuntime; locale: Locale }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const isStopped = item.workspace.state === "stopped";
  const usage = useMemo(() => runtime && !isStopped ? aggregateRuntimeUsage(runtime) : { cpuMillis: null, memoryMiB: null }, [isStopped, runtime]);
  const resources = item.workspace.resources;
  const persistent = isStopped ? undefined : runtime?.persistent_storage;
  const temporary = isStopped ? undefined : runtime?.temporary_storage;
  return <div className={styles.resourceGrid}>
    <ResourceMeter label="CPU" actual={formatCpuMillis(usage.cpuMillis)} requested={`${resources.cpu_millis}m`} percent={usagePercent(usage.cpuMillis, resources.cpu_millis)} unavailableLabel={t("metricsUnavailable")} usageOfLimitLabel={t("usageOfLimit")} />
    <ResourceMeter label={t("memory")} actual={formatMemoryMiB(usage.memoryMiB)} requested={`${formatMemoryMiB(resources.memory_mib)}`} percent={usagePercent(usage.memoryMiB, resources.memory_mib)} unavailableLabel={t("metricsUnavailable")} usageOfLimitLabel={t("usageOfLimit")} />
    <StorageMeter label={t("persistentDisk")} telemetry={persistent} configuredGiB={resources.disk_gib} locale={locale} />
    <StorageMeter label={t("temporaryStorage")} telemetry={temporary} configuredGiB={item.workspace.storage_policy.temporary_storage_gib} locale={locale} statusOverride={isStopped ? t("temporaryStorageReleased") : undefined} />
    {resources.gpu_count > 0 && <div className={styles.meter}><div className={styles.meterHeader}><Text>GPU</Text><Text className={styles.meterValue}>{resources.gpu_count} GPU</Text></div><Caption1 className={styles.meterHint}>{t("configuredAllocation")} · {t("gpuTelemetryUnavailable")}</Caption1></div>}
  </div>;
}

function RuntimeStatus({ id, runtime }: { id: string; runtime: WorkspaceRuntime }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  return <section id={id} className={styles.statusPanel} aria-label={t("runtimeStatus")}><div className={styles.panelHeading}><Title3 as="h3">{t("containers")}</Title3><Caption1>{runtime.metrics_available ? t("usageAvailable") : t("metricsUnavailable")}</Caption1></div>{runtime.pods.length === 0 && runtime.metrics.length === 0 && <Text>{t("noRuntimeData")}</Text>}<div className={styles.podGrid}>{runtime.pods.map((pod) => <div className={styles.podRow} key={pod.name}><Text className={styles.code}>{pod.name}</Text><Text>{pod.phase ?? "unknown"}</Text><Badge appearance="tint" color={pod.ready ? "success" : "danger"}>{pod.ready ? t("ready") : t("notReady")}</Badge><Caption1>{pod.restarts} {t("restarts")}</Caption1></div>)}</div>{runtime.metrics.length > 0 && <Divider />}{runtime.metrics.length > 0 && <div className={styles.podGrid}>{runtime.metrics.map((metric) => <div className={styles.podRow} key={`${metric.pod}-${metric.container}`}><Text className={styles.code}>{metric.container}</Text><Text>CPU {formatCpuMillis(parseCpuMillis(metric.cpu))}</Text><Text>{t("memory")} {formatMemoryMiB(parseMemoryMiB(metric.memory))}</Text></div>)}</div>}</section>;
}

function EventLog({ id, runtime, locale }: { id: string; runtime: WorkspaceRuntime; locale: Locale }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  return <section id={id} className={styles.statusPanel} aria-label={t("eventLog")}><Title3 as="h3">{t("eventLog")}</Title3>{runtime.events.length === 0 && <Text>{t("noEvents")}</Text>}<div className={styles.eventList}>{runtime.events.slice(0, 12).map((event, index) => <article className={styles.event} key={`${event.observed_at}-${event.category}-${index}`}><Text weight="semibold">{t(eventCategoryKeys[event.category].label)}</Text><Text>{t(eventCategoryKeys[event.category].description)}</Text><Caption1 className={styles.eventMeta}>{t("observedAt")} {formatTimestamp(event.observed_at, locale)} · {t("eventCount")} {event.count ?? 1}</Caption1></article>)}</div></section>;
}

const eventCategoryKeys: Record<WorkspaceRuntimeEvent["category"], { label: MessageKey; description: MessageKey }> = {
  disk_pressure: { label: "eventCategoryDiskPressure", description: "eventCategoryDiskPressureDescription" },
  evicted: { label: "eventCategoryEvicted", description: "eventCategoryEvictedDescription" },
  temporary_storage_provisioning: { label: "eventCategoryTemporaryStorageProvisioning", description: "eventCategoryTemporaryStorageProvisioningDescription" },
  temporary_storage_attachment: { label: "eventCategoryTemporaryStorageAttachment", description: "eventCategoryTemporaryStorageAttachmentDescription" },
  volume_unavailable: { label: "eventCategoryVolumeUnavailable", description: "eventCategoryVolumeUnavailableDescription" },
  ephemeral_storage: { label: "eventCategoryEphemeralStorage", description: "eventCategoryEphemeralStorageDescription" },
  memory_pressure: { label: "eventCategoryMemoryPressure", description: "eventCategoryMemoryPressureDescription" },
  pid_pressure: { label: "eventCategoryPidPressure", description: "eventCategoryPidPressureDescription" },
  other: { label: "eventCategoryOther", description: "eventCategoryOtherDescription" },
};

function workspaceTransition(state: string): { action: WorkspaceAction; label: MessageKey } | null {
  const transitions: Record<string, { action: WorkspaceAction; label: MessageKey }> = {
    provisioning: { action: "start", label: "stateProvisioning" },
    starting: { action: "start", label: actionProgressLabel("start") },
    stopping: { action: "stop", label: actionProgressLabel("stop") },
    restarting: { action: "restart", label: actionProgressLabel("restart") },
    deleting: { action: "delete", label: actionProgressLabel("delete") },
  };
  return transitions[state] ?? null;
}

function actionProgressLabel(action: WorkspaceAction): MessageKey {
  const labels: Record<WorkspaceAction, MessageKey> = {
    start: "stateStarting",
    stop: "stateStopping",
    restart: "stateRestarting",
    delete: "stateDeleting",
  };
  return labels[action];
}

function formatTimestamp(value: string | null, locale: Locale): string { if (!value) return "—"; const parsed = new Date(value); return Number.isNaN(parsed.valueOf()) ? value : parsed.toLocaleString(locale); }
function shortImage(value: string): string {
  const withoutDigest = value.split("@", 1)[0] ?? value;
  const segments = withoutDigest.split("/");
  return segments.at(-1) || value;
}
