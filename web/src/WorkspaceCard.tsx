import { useMemo, useState } from "react";
import { useI18n } from "./i18n";
import type { Locale } from "./i18n";
import type { WorkspaceResponse, WorkspaceRuntime } from "./types";
import { WorkspaceConnectionDialog } from "./WorkspaceConnectionDialog";
import {
  aggregateRuntimeUsage,
  formatCpuMillis,
  formatMemoryMiB,
  formatPercent,
  parseCpuMillis,
  parseMemoryMiB,
  usagePercent,
} from "./workspaceMetrics";

type WorkspaceAction = "start" | "stop" | "restart" | "delete";
type DetailView = "status" | "events";

interface Props {
  item: WorkspaceResponse;
  runtime?: WorkspaceRuntime;
  onAction: (id: string, action: WorkspaceAction) => Promise<void>;
  onOpenShell: (id: string) => Promise<void>;
  onRequestRuntime: (id: string) => Promise<void>;
}

export function WorkspaceCard({ item, runtime, onAction, onOpenShell, onRequestRuntime }: Props) {
  const { locale, t } = useI18n();
  const [detailView, setDetailView] = useState<DetailView | null>(null);
  const workspace = item.workspace;
  const toggleDetail = (view: DetailView) => {
    setDetailView((current) => current === view ? null : view);
    if (view === "events" && detailView !== "events") void onRequestRuntime(workspace.id);
  };

  return (
    <article className="workspace-card">
      <div className="workspace-main">
        <div className="workspace-title"><div><h3>{workspace.name}</h3><code>{workspace.short_id}</code></div></div>
        <StateBadge state={workspace.state} locale={locale} />
      </div>

      <div className="workspace-meta compact">
        <span>{workspace.workspace_user}</span>
        <span>{workspace.access_mode === "public" ? t("public") : t("internal")}</span>
        <code>{item.namespace}</code>
        {workspace.resources.gpu_count === 0 && <span className="gpu-meta">0 GPU</span>}
      </div>

      <ResourceOverview item={item} runtime={runtime} />

      {item.ssh_connection && <WorkspaceConnectionDialog connection={item.ssh_connection} workspaceHostKey={item.workspace_host_key} jumpHostKey={item.jump_host_key} />}

      <div className="workspace-toolbar">
        <div className="primary-actions">
          {workspace.state === "ready" && <button className="terminal-action" onClick={() => void onOpenShell(workspace.id)}>{t("webShell")}</button>}
          {workspace.state === "ready" && <button onClick={() => void onAction(workspace.id, "stop")}>{t("stop")}</button>}
          {workspace.state === "ready" && <button onClick={() => void onAction(workspace.id, "restart")}>{t("restart")}</button>}
          {(workspace.state === "stopped" || workspace.state === "failed") && <button onClick={() => void onAction(workspace.id, "start")}>{t("start")}</button>}
        </div>
        <div className="detail-actions">
          {runtime && <button className={detailView === "status" ? "active" : ""} aria-controls={`runtime-${workspace.short_id}`} aria-expanded={detailView === "status"} onClick={() => toggleDetail("status")}>{t("runtimeStatus")} <span aria-hidden="true">{detailView === "status" ? "▴" : "▾"}</span></button>}
          {runtime && <button className={detailView === "events" ? "active" : ""} aria-controls={`events-${workspace.short_id}`} aria-expanded={detailView === "events"} onClick={() => toggleDetail("events")}>{t("eventLog")} <span aria-hidden="true">{detailView === "events" ? "▴" : "▾"}</span></button>}
          {!(["deleting", "deleted"] as string[]).includes(workspace.state) && <button className="danger" onClick={() => void onAction(workspace.id, "delete")}>{t("delete")}</button>}
        </div>
      </div>

      {runtime && detailView === "status" && <RuntimeStatus id={`runtime-${workspace.short_id}`} runtime={runtime} />}
      {runtime && detailView === "events" && <EventLog id={`events-${workspace.short_id}`} runtime={runtime} locale={locale} />}
    </article>
  );
}

function ResourceOverview({ item, runtime }: { item: WorkspaceResponse; runtime?: WorkspaceRuntime }) {
  const { t } = useI18n();
  const usage = useMemo(() => runtime ? aggregateRuntimeUsage(runtime) : { cpuMillis: null, memoryMiB: null }, [runtime]);
  const resources = item.workspace.resources;
  const cpuPercent = usagePercent(usage.cpuMillis, resources.cpu_millis);
  const memoryPercent = usagePercent(usage.memoryMiB, resources.memory_mib);
  return <div className="resource-overview">
    <ResourceMeter label="CPU" actual={formatCpuMillis(usage.cpuMillis)} requested={`${resources.cpu_millis}m`} percent={cpuPercent} />
    <ResourceMeter label={t("memory")} actual={formatMemoryMiB(usage.memoryMiB)} requested={`${formatMemoryMiB(resources.memory_mib)}`} percent={memoryPercent} />
    <DiskMeter runtime={runtime} configuredGiB={resources.disk_gib} />
    {resources.gpu_count > 0 && <div className="capacity-meter"><div><span>GPU</span><strong>{resources.gpu_count} GPU</strong></div><small>{t("configuredAllocation")} · {t("gpuTelemetryUnavailable")}</small></div>}
  </div>;
}

function ResourceMeter({ label, actual, requested, percent }: { label: string; actual: string; requested: string; percent: number | null }) {
  const { t } = useI18n();
  const valueText = percent === null ? `${label}: ${t("metricsUnavailable")}` : `${actual} / ${requested}, ${formatPercent(percent)}`;
  return <div className="resource-meter">
    <div className="resource-meter-heading"><span>{label}</span><strong>{actual} <small>/ {requested}</small></strong></div>
    <div className="resource-track" role="progressbar" aria-label={`${label} ${t("usageOfLimit")}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined} aria-valuetext={valueText}><span className={percent !== null && percent > 0 ? "has-value" : ""} style={{ width: percent === null ? "0" : `${percent}%` }} /></div>
    <small>{t("usageOfLimit")} · {formatPercent(percent)}</small>
  </div>;
}

function DiskMeter({ runtime, configuredGiB }: { runtime?: WorkspaceRuntime; configuredGiB: number }) {
  const { t } = useI18n();
  const telemetry = runtime?.storage;
  const usable = telemetry?.status === "available" || telemetry?.status === "stale";
  const used = usable ? telemetry.used_bytes : null;
  const capacity = usable ? telemetry.capacity_bytes : null;
  const percent = used !== null && capacity !== null && capacity > 0 ? Math.min(100, Math.max(0, used / capacity * 100)) : null;
  const configured = formatStorageCapacity(runtime?.pvc_capacity, configuredGiB);
  const actual = used === null ? "—" : formatBytes(used);
  const status = telemetry?.status === "stale" ? t("storageTelemetryStale") : telemetry?.status === "disabled" ? t("storageTelemetryDisabled") : telemetry?.status === "available" ? t("storageTelemetryAvailable") : t("storageTelemetryUnavailable");
  const valueText = percent === null ? `${t("disk")}: ${status}` : `${actual} / ${formatBytes(capacity ?? 0)}, ${formatPercent(percent)}`;
  return <div className="resource-meter disk-meter">
    <div className="resource-meter-heading"><span>{t("disk")}</span><strong>{actual} <small>/ {configured}</small></strong></div>
    <div className="resource-track" role="progressbar" aria-label={`${t("disk")} ${t("storageUsage")}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined} aria-valuetext={valueText}><span className={percent !== null && percent > 0 ? "has-value" : ""} style={{ width: percent === null ? "0" : `${percent}%` }} /></div>
    <small>{status}{telemetry?.observed_at ? ` · ${t("observedAt")} ${new Date(telemetry.observed_at * 1_000).toLocaleString()}` : ""}</small>
  </div>;
}

function RuntimeStatus({ id, runtime }: { id: string; runtime: WorkspaceRuntime }) {
  const { t } = useI18n();
  return <section id={id} className="runtime-panel" aria-label={t("runtimeStatus")}>
    <h4>{t("containers")}</h4>
    {runtime.pods.length === 0 && runtime.metrics.length === 0 && <p>{t("noRuntimeData")}</p>}
    <div className="pod-status-grid">{runtime.pods.map((pod) => <div key={pod.name}><code>{pod.name}</code><span>{pod.phase ?? "unknown"}</span><span className={pod.ready ? "healthy" : "unhealthy"}>{pod.ready ? t("ready") : t("notReady")}</span><small>{pod.restarts} {t("restarts")}</small></div>)}</div>
    {runtime.metrics.length > 0 && <div className="container-metrics">{runtime.metrics.map((metric) => <div key={`${metric.pod}-${metric.container}`}><code>{metric.container}</code><span>CPU {formatCpuMillis(parseCpuMillis(metric.cpu))}</span><span>{t("memory")} {formatMemoryMiB(parseMemoryMiB(metric.memory))}</span></div>)}</div>}
  </section>;
}

function EventLog({ id, runtime, locale }: { id: string; runtime: WorkspaceRuntime; locale: Locale }) {
  const { t } = useI18n();
  return <section id={id} className="runtime-panel event-panel" aria-label={t("eventLog")}>
    <h4>{t("eventLog")}</h4>
    {runtime.events.length === 0 && <p>{t("noEvents")}</p>}
    {runtime.events.slice(0, 12).map((event, index) => <article key={`${event.last_timestamp}-${event.reason}-${index}`}>
      <div><strong>{event.reason ?? event.event_type ?? "Event"}</strong>{event.event_type && <span className={`event-type ${event.event_type.toLowerCase()}`}>{event.event_type}</span>}</div>
      <p>{event.message ?? "—"}</p>
      <small>{t("observedAt")} {formatTimestamp(event.last_timestamp, locale)} · {t("eventCount")} {event.count ?? 1}</small>
    </article>)}
  </section>;
}

function StateBadge({ state }: { state: string; locale: Locale }) {
  const { t } = useI18n();
  const labels = {
    provisioning: "stateProvisioning", ready: "stateReady", stopping: "stateStopping", stopped: "stateStopped", starting: "stateStarting", restarting: "stateRestarting", deleting: "stateDeleting", deleted: "stateDeleted", failed: "stateFailed",
  } as const;
  return <span className={`state-badge ${state}`}>{state in labels ? t(labels[state as keyof typeof labels]) : state}</span>;
}

function formatTimestamp(value: string | null, locale: Locale): string {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : parsed.toLocaleString(locale);
}

function formatStorageCapacity(value: string | null | undefined, fallbackGiB: number): string {
  if (!value) return `${fallbackGiB} GiB`;
  const binary = value.match(/^([0-9]+(?:\.[0-9]+)?)(Ki|Mi|Gi|Ti)$/);
  return binary ? `${binary[1]} ${binary[2]}B` : value;
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled >= 10 || unit === 0 ? scaled.toFixed(0) : scaled.toFixed(1)} ${units[unit]}`;
}
