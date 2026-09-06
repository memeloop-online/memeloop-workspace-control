import { useI18n } from "../i18n";
import type { Resources, WorkspaceSummary } from "../types";

interface Props {
  summary: WorkspaceSummary | null;
  quota: Resources | null;
}

export function WorkspaceStats({ summary, quota }: Props) {
  const { t } = useI18n();
  const requested = summary?.requested ?? EMPTY_RESOURCES;
  const cards: Array<{ kind: "count" | "cpu" | "memory" | "disk"; label: string; value: string; used: number | null; limit: number | null }> = [
    { kind: "count", label: t("workspaces"), value: summary ? String(summary.total_count) : "—", used: null, limit: null },
    { kind: "cpu", label: "CPU", value: summary ? `${formatCores(requested.cpu_millis)} ${t("cores")}` : "—", used: summary ? requested.cpu_millis : null, limit: quota?.cpu_millis ?? null },
    { kind: "memory", label: t("memory"), value: summary ? `${formatGiB(requested.memory_mib)} GiB` : "—", used: summary ? requested.memory_mib : null, limit: quota?.memory_mib ?? null },
    { kind: "disk", label: t("persistentDisk"), value: summary ? `${requested.disk_gib} GiB` : "—", used: summary ? requested.disk_gib : null, limit: quota?.disk_gib ?? null },
  ];

  return <div className="workspace-stats" aria-label={t("resources")}>
    {cards.map((card) => <UsageStat key={card.kind} {...card} gpu={card.kind === "disk" && summary ? requested.gpu_count : null} ready={card.kind === "count" && summary ? summary.state_counts.ready ?? 0 : null} />)}
  </div>;
}

function UsageStat({ kind, label, value, used, limit, gpu, ready }: { kind: "count" | "cpu" | "memory" | "disk"; label: string; value: string; used: number | null; limit: number | null; gpu: number | null; ready: number | null }) {
  const { t } = useI18n();
  const percentage = usagePercentage(used, limit);
  const detail = kind === "count"
    ? `${label}: ${value}`
    : used === null
    ? t("requestedTotal")
    : limit === null
      ? `${t("requestedTotal")}: ${value}`
      : `${t("usageOfLimit")}: ${value} / ${formatLimit(kind, limit, t("cores"))}`;
  const footnote = ready !== null
    ? `${ready} ${t("stateReady")}`
    : gpu !== null
      ? `${percentage === null ? "" : `${percentage}% · `}${gpu} GPU`
      : limit === null ? t("requestedTotal") : `${percentage}%`;
  return <article className="workspace-stat" title={detail} aria-label={detail} tabIndex={0}>
    {percentage !== null && <span className="workspace-stat-fill" style={{ width: `${percentage}%` }} aria-hidden="true" />}
    <span className="workspace-stat-label">{label}</span>
    <strong>{value}</strong>
    <small>{footnote}</small>
  </article>;
}

const EMPTY_RESOURCES: Resources = { cpu_millis: 0, memory_mib: 0, disk_gib: 0, gpu_count: 0 };

function usagePercentage(used: number | null, limit: number | null): number | null {
  if (used === null || limit === null || limit <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((used / limit) * 100)));
}

function formatCores(millis: number) {
  return (millis / 1000).toFixed(millis % 1000 === 0 ? 0 : 1);
}

function formatGiB(mib: number) {
  return (mib / 1024).toFixed(mib % 1024 === 0 ? 0 : 1);
}

function formatLimit(kind: "count" | "cpu" | "memory" | "disk", value: number, cores: string): string {
  if (kind === "cpu") return `${formatCores(value)} ${cores}`;
  if (kind === "memory") return `${formatGiB(value)} GiB`;
  return `${value} GiB`;
}
