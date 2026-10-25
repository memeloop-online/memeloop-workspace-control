import { Fragment, useMemo, useState } from "react";
import { Badge, Button, Caption1, Divider, Spinner, Text, Title3, Tooltip } from "@fluentui/react-components";
import { ArrowClockwiseRegular, ChevronDownRegular, ChevronUpRegular, DeleteRegular, DesktopRegular, LocationRegular, PlayRegular, SaveRegular, StopRegular, WindowConsoleRegular } from "@fluentui/react-icons";
import { useI18n } from "./i18n";
import type { Locale, MessageKey } from "./i18n";
import type { AvailableNodePool, WorkspaceResponse, WorkspaceRuntime, WorkspaceRuntimeEvent, WorkspaceRuntimePod } from "./types";
import { WorkspaceConnectionDialog } from "./WorkspaceConnectionDialog";
import { WorkspacePortMappings } from "./WorkspacePortMappings";
import type { ApiClient } from "./api";
import { nodePoolDisplayName } from "./forms/NodePoolPicker";
import { safeBootstrapUrl } from "./portMappings";
import { reserveWebShellWindow } from "./workspaceShell";
import { formatBytes, ResourceMeter, WorkspaceStatusCard } from "./shared";
import { aggregateRuntimeUsage, formatCpuMillis, formatMemoryMiB, parseCpuMillis, parseMemoryMiB, usagePercent } from "./workspaceMetrics";
import { StorageMeter } from "./workspaces/StorageMeter";
import { useWorkspaceStyles } from "./workspaces/workspaceStyles";
import { containerNameLabelKey, containerStateLabelKey, formatDurationSeconds, startupStageLabelKey } from "./workspaceStartup";
import { eventPresentationKeys, eventSchedulingPresentationKeys } from "./workspaceEvents";

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
  onShrinkStorage: (item: WorkspaceResponse) => void;
  onError: (message: string) => void;
  canConnect: boolean;
  canChangeState: boolean;
  canDelete: boolean;
  canChangePlacement: boolean;
  canShrinkStorage: boolean;
  templateName?: string;
}

export function WorkspaceCard({ api, item, runtime, nodePools, busyAction, onAction, onOpenShell, onRequestRuntime, onChangePlacement, onShrinkStorage, onError, canConnect, canChangeState, canDelete, canChangePlacement, canShrinkStorage, templateName }: Props) {
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
          {canConnect && running && item.ssh_connection && <WorkspaceConnectionDialog connection={item.ssh_connection} shortId={workspace.short_id} hostKey={item.workspace_host_key} />}
          {canConnect && running && <Tooltip content={t("webShellClipboardHelp")} relationship="description"><Button appearance="primary" icon={<WindowConsoleRegular />} onClick={() => void onOpenShell(workspace.id)}>{t("webShell")}</Button></Tooltip>}
          {canConnect && running && item.desktop?.status === "ready" && item.desktop.https_url && <Button appearance="outline" icon={<DesktopRegular />} disabled={openingDesktop} onClick={() => void openDesktop(api, workspace.id, item.desktop!.mapping_id, setOpeningDesktop, onError, t)}>{openingDesktop ? t("desktopOpening") : t("openDesktop")}</Button>}
          {canConnect && running && <WorkspacePortMappings api={api} workspaceId={workspace.id} workspaceReady onError={onError} />}
          {canChangeState && running && <Button appearance="subtle" icon={<StopRegular />} onClick={() => onAction(item, "stop")}>{t("stop")}</Button>}
          {canChangeState && running && <Button appearance="subtle" icon={<ArrowClockwiseRegular />} onClick={() => onAction(item, "restart")}>{t("restart")}</Button>}
          {canChangeState && (workspace.state === "stopped" || workspace.state === "failed") && <Button appearance="primary" icon={<PlayRegular />} onClick={() => onAction(item, "start")}>{t("start")}</Button>}
          {canChangePlacement && stopped && <Button appearance="subtle" icon={<LocationRegular />} onClick={() => onChangePlacement(item)}>{t("changeLocation")}</Button>}
          {canShrinkStorage && (running || stopped) && <Button appearance="subtle" icon={<SaveRegular />} onClick={() => onShrinkStorage(item)}>{t("shrinkStorage")}</Button>}
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
  const startupPods = runtime.pods.filter((pod) => pod.startup || (pod.containers && pod.containers.length > 0));
  return <section id={id} className={styles.statusPanel} aria-label={t("eventLog")}>
    <Title3 as="h3">{t("eventLog")}</Title3>
    {startupPods.map((pod) => <StartupSummary key={pod.name} pod={pod} />)}
    {runtime.events.length === 0 && <Text>{t("noEvents")}</Text>}
    <div className={styles.eventList}>{runtime.events.slice(0, 12).map((event) => <EventEntry key={event.id} event={event} locale={locale} />)}</div>
  </section>;
}

function EventEntry({ event, locale }: { event: WorkspaceRuntimeEvent; locale: Locale }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const blockerKeys = eventSchedulingPresentationKeys(event);
  const entries = blockerKeys.length > 0 ? blockerKeys : [eventPresentationKeys(event)];
  const details = event.details;
  const diagnostic = eventDiagnosticText(event, t);
  const showLast = event.last_observed_at && event.last_observed_at !== event.observed_at;
  return <article className={styles.event}>
    {entries.map((keys) => <Fragment key={keys.label}>
      <Text weight="semibold">{t(keys.label)}</Text>
      {!diagnostic && <Text>{t(keys.description)}</Text>}
    </Fragment>)}
    {diagnostic && <Text>{diagnostic}</Text>}
    <Caption1 className={styles.eventMeta}>
      {t("observedAt")} {formatTimestamp(event.observed_at, locale)}
      {showLast && <> · {t("eventLastObserved")} {formatTimestamp(event.last_observed_at ?? null, locale)}</>}
      {event.count !== null && <> · {t("eventCount")} {event.count}</>}
      {details.reason && <> · {t("eventReason")} <span className={styles.code}>{details.reason}</span></>}
    </Caption1>
    {(details.event_uid || details.pod_uid) && <details className={styles.eventMeta}>
      <summary>{t("eventDetails")}</summary>
      <Caption1>
        {details.event_uid && <>{t("eventUid")} <span className={styles.code}>{details.event_uid}</span></>}
        {details.event_uid && details.pod_uid && " · "}
        {details.pod_uid && <>{t("eventPodUid")} <span className={styles.code}>{details.pod_uid}</span></>}
      </Caption1>
    </details>}
  </article>;
}

function eventDiagnosticText(event: WorkspaceRuntimeEvent, t: ReturnType<typeof useI18n>["t"]): string | null {
  const diagnostic = event.details.diagnostic;
  if (!diagnostic) return null;
  if (diagnostic.kind === "container_local_ephemeral_limit") {
    const values = { container: diagnostic.container, limit: formatBytes(diagnostic.limit_bytes) };
    return event.details.reason === "Evicted" ? t("eventDiagnosticContainerLocalLimitEvicted", values) : t("eventDiagnosticContainerLocalLimit", values);
  }
  if (diagnostic.kind === "pod_local_ephemeral_limit") return t("eventDiagnosticPodLocalLimit", { limit: formatBytes(diagnostic.limit_bytes) });
  if (diagnostic.kind === "node_resource_pressure") {
    if (diagnostic.resource === "memory") return t("eventDiagnosticNodeMemoryPressure");
    if (diagnostic.resource === "pids") return t("eventDiagnosticNodePidPressure");
    return t("eventDiagnosticNodeEphemeralPressure");
  }
  return null;
}

function StartupSummary({ pod }: { pod: WorkspaceRuntimePod }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const startup = pod.startup;
  const containers = pod.containers ?? [];
  return <section className={styles.event} aria-label={`${t("startupTiming")} ${pod.name}`}>
    <div className={styles.panelHeading}>
      <Text weight="semibold">{t("startupTiming")} <span className={styles.code}>{pod.name}</span></Text>
      {startup?.total_seconds != null && <Badge appearance="tint" color="informative">{t("startupReadyIn")} {formatDurationSeconds(startup.total_seconds)}</Badge>}
    </div>
    {startup && startup.stages.length > 0 && <div className={styles.podGrid}>{startup.stages.map((stage, index) => {
      const labelKey = startupStageLabelKey(stage.name) ?? containerNameLabelKey(stage.name);
      return <div className={styles.podRow} key={`${stage.name}-${index}`}>
        <Text>{labelKey ? t(labelKey) : stage.name}</Text>
        <Caption1>{formatDurationSeconds(stage.duration_seconds)}</Caption1>
      </div>;
    })}</div>}
    {containers.length > 0 && <div className={styles.podGrid}>{containers.map((container) => {
      const nameKey = containerNameLabelKey(container.name);
      const stateKey = containerStateLabelKey(container.state);
      return <div className={styles.podRow} key={container.name}>
        <Text>{nameKey ? t(nameKey) : <span className={styles.code}>{container.name}</span>}</Text>
        <Text>{stateKey ? t(stateKey) : container.state}</Text>
        <Badge appearance="tint" color={container.ready ? "success" : "danger"}>{container.ready ? t("ready") : t("notReady")}</Badge>
        <Caption1>{container.restarts} {t("restarts")}{container.reason ? ` · ${container.reason}` : ""}{container.exit_code != null ? ` · ${t("exitCode")} ${container.exit_code}` : ""}</Caption1>
      </div>;
    })}</div>}
  </section>;
}

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
