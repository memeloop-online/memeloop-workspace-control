import { useMemo, useState } from "react";
import { useI18n } from "./i18n";
import { runtimeProfileLabel } from "./runtimeProfiles";
import type { Locale } from "./i18n";
import type { WorkspaceResponse, WorkspaceRuntime } from "./types";
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
        <span>{runtimeProfileLabel(workspace.runtime_profile, t)}</span>
        <span>{workspace.access_mode === "public" ? t("public") : t("internal")}</span>
        <code>{item.namespace}</code>
        {workspace.resources.gpu_count === 0 && <span className="gpu-meta">0 GPU</span>}
      </div>

      <ResourceOverview item={item} runtime={runtime} />

      {(item.ssh_command || item.ssh_config) && (
        <div className="connection-strip">
          {item.ssh_command && <ConnectionItem label="SSH" value={item.ssh_command} />}
          {item.ssh_config && <ConnectionItem label="Codex" value={`mwc-${workspace.short_id}`} />}
        </div>
      )}

      {(item.ssh_config || item.workspace_host_key || item.jump_host_key) && (
        <details className="connection-details">
          <summary>{t("connectionDetails")}</summary>
          {item.ssh_config && <CopyLine label={t("copySshConfig")} value={item.ssh_config} />}
          {item.workspace_host_key && <CopyLine label={t("hostKey")} value={`${item.workspace_host_key.fingerprint} ${item.workspace_host_key.public_key}`} />}
          {item.jump_host_key && <CopyLine label={t("jumpKey")} value={`${item.jump_host_key.fingerprint} ${item.jump_host_key.public_key}`} />}
        </details>
      )}

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
    <div className="capacity-meter"><div><span>{t("disk")}</span><strong>{formatStorageCapacity(runtime?.pvc_capacity, resources.disk_gib)}</strong></div><small>{t("configuredCapacity")} · {t("storageTelemetryUnavailable")}</small></div>
    {(resources.gpu_count > 0 || (runtime?.allocated.gpu_count ?? 0) > 0) && <ResourceMeter label="GPU" actual={String(runtime?.allocated.gpu_count ?? 0)} requested={String(resources.gpu_count)} percent={usagePercent(runtime?.allocated.gpu_count ?? null, resources.gpu_count)} />}
  </div>;
}

function ResourceMeter({ label, actual, requested, percent }: { label: string; actual: string; requested: string; percent: number | null }) {
  const { t } = useI18n();
  const valueText = percent === null ? `${label}: ${t("metricsUnavailable")}` : `${actual} / ${requested}, ${formatPercent(percent)}`;
  return <div className="resource-meter">
    <div className="resource-meter-heading"><span>{label}</span><strong>{actual} <small>/ {requested}</small></strong></div>
    <div className="resource-track" role="progressbar" aria-label={`${label} ${t("usageOfRequested")}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined} aria-valuetext={valueText}><span className={percent !== null && percent > 0 ? "has-value" : ""} style={{ width: percent === null ? "0" : `${percent}%` }} /></div>
    <small>{t("usageOfRequested")} · {formatPercent(percent)}</small>
  </div>;
}

function ConnectionItem({ label, value }: { label: string; value: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  return <div className="connection-item"><span>{label}</span><code title={value}>{value}</code><button aria-label={`${t("copy")} ${label}`} onClick={() => { void navigator.clipboard.writeText(value); setCopied(true); window.setTimeout(() => setCopied(false), 1200); }}>{copied ? t("copied") : t("copy")}</button></div>;
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

function CopyLine({ label, value }: { label: string; value: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  return <div className="copy-line"><span>{label}</span><code>{value}</code><button onClick={() => { void navigator.clipboard.writeText(value); setCopied(true); window.setTimeout(() => setCopied(false), 1200); }}>{copied ? t("copied") : t("copy")}</button></div>;
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
