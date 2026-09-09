import { useI18n } from "../i18n";
import type { OrganizationUsageSummary, Resources, UsageAvailability } from "../types";
import { availabilityLabel, formatBytesAsGiB, formatCores, formatGiB, usageBarPercentage, usagePercentage } from "./usageSummaryPresentation";

interface Props { summary: OrganizationUsageSummary | null; quota: Resources | null; stale?: boolean; }
type ResourceKind = "cpu" | "memory" | "disk";

export function WorkspaceStats({ summary, quota, stale = false }: Props) {
  const { locale, t } = useI18n();
  const observed = summary?.observed_at === null || summary?.observed_at === undefined
    ? t("usageNotObserved")
    : new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(summary.observed_at * 1000);
  const totalDetail = summary
    ? `${t("workspaces")}: ${summary.total_count}. ${t("usageCoverage")}: ${summary.coverage.eligible_workspaces}/${summary.coverage.total_workspaces}. ${t(summary.coverage.template_label_coverage === "complete" ? "usageCoverageComplete" : "usageCoverageIncomplete")}.`
    : t("usageLoading");
  return <div className="workspace-stats" aria-label={t("actualUsage")}>
    <article className="workspace-stat workspace-stat-count" title={totalDetail} aria-label={totalDetail} tabIndex={0}>
      <span className="workspace-stat-label">{t("workspaces")}</span><strong>{summary ? summary.total_count : "—"}</strong>
      <small>{summary ? `${summary.state_counts.ready ?? 0} ${t("stateReady")}` : t("usageLoading")}</small>
    </article>
    <UsageStat kind="cpu" summary={summary} quota={quota?.cpu_millis ?? null} observed={observed} stale={stale} />
    <UsageStat kind="memory" summary={summary} quota={quota?.memory_mib ?? null} observed={observed} stale={stale} />
    <UsageStat kind="disk" summary={summary} quota={quota?.disk_gib ?? null} observed={observed} stale={stale} />
  </div>;
}

function UsageStat({ kind, summary, quota, observed, stale }: { kind: ResourceKind; summary: OrganizationUsageSummary | null; quota: number | null; observed: string; stale: boolean }) {
  const { t } = useI18n();
  const requested = summary?.requested ?? EMPTY_RESOURCES;
  const actual = summary ? actualFor(kind, summary) : null;
  const requestedValue = requestedFor(kind, requested);
  const availability: UsageAvailability = summary?.availability[kind] ?? "unknown";
  const percentage = usagePercentage(actual, requestedValue);
  const value = actual === null ? "—" : format(kind, actual, t("cores"));
  const requestedDisplay = format(kind, requestedValue, t("cores"));
  const quotaDetail = quota === null ? "" : ` ${t("usageQuota")}: ${format(kind, kind === "disk" ? quota * 1024 ** 3 : quota, t("cores"))}.`;
  const availabilityText = t(availabilityMessageKey(availabilityLabel(availability)));
  const detail = `${t("actualUsage")}: ${value}. ${t("usageRequested")}: ${requestedDisplay}. ${t("usageObservedAt")}: ${observed}. ${availabilityText}.${quotaDetail}`;
  const footnote = actual === null ? availabilityText : percentage === null ? t("usageNoRequested") : `${percentage}% ${t("usageOfOrganizationRequested")}${stale ? ` · ${t("usageStale")}` : ""}`;
  return <article className="workspace-stat" title={detail} aria-label={detail} tabIndex={0}>
    {usageBarPercentage(percentage) !== null && <span className="workspace-stat-fill" style={{ width: `${usageBarPercentage(percentage)}%` }} aria-hidden="true" />}
    <span className="workspace-stat-label">{kind === "cpu" ? "CPU" : kind === "memory" ? t("memory") : t("persistentDisk")}</span><strong>{value}</strong><small>{footnote}</small>
  </article>;
}

const EMPTY_RESOURCES: Resources = { cpu_millis: 0, memory_mib: 0, disk_gib: 0, gpu_count: 0 };
function actualFor(kind: ResourceKind, summary: OrganizationUsageSummary): number | null { return kind === "cpu" ? summary.actual.cpu_millis : kind === "memory" ? summary.actual.memory_mib : summary.actual.disk_bytes; }
function requestedFor(kind: ResourceKind, resources: Resources): number { return kind === "cpu" ? resources.cpu_millis : kind === "memory" ? resources.memory_mib : resources.disk_gib * 1024 ** 3; }
function format(kind: ResourceKind, value: number, cores: string): string { return kind === "cpu" ? `${formatCores(value)} ${cores}` : kind === "memory" ? `${formatGiB(value)} GiB` : `${formatBytesAsGiB(value)} GiB`; }
function availabilityMessageKey(value: UsageAvailability): "usageAvailable" | "usageUnavailable" | "usageUnknown" {
  return value === "available" ? "usageAvailable" : value === "unavailable" ? "usageUnavailable" : "usageUnknown";
}
