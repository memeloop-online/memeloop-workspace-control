import { Card, Caption1, Text, Title3, Tooltip } from "@fluentui/react-components";
import { useI18n } from "../i18n";
import type { OrganizationUsageSummary, QuotaResources, UsageAvailability } from "../types";
import { availabilityLabel, formatBytesAsGiB, formatCores, formatGiB, usageBarPercentage, usagePercentage } from "./usageSummaryPresentation";
import { useWorkspaceStyles } from "./workspaceStyles";

interface Props { summary: OrganizationUsageSummary | null; quota: QuotaResources | null; stale?: boolean; }
type ResourceKind = "cpu" | "memory" | "disk" | "temporary";

export function WorkspaceStats({ summary, quota, stale = false }: Props) {
  const { locale, t } = useI18n();
  const styles = useWorkspaceStyles();
  const observed = summary?.observed_at === null || summary?.observed_at === undefined
    ? t("usageNotObserved")
    : new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(summary.observed_at * 1000);
  const totalDetail = summary
    ? `${t("workspaces")}: ${summary.total_count}. ${t("usageCoverage")}: ${summary.coverage.eligible_workspaces}/${summary.coverage.total_workspaces}. ${t(summary.coverage.template_label_coverage === "complete" ? "usageCoverageComplete" : "usageCoverageIncomplete")}.`
    : t("usageLoading");
  return <section className={styles.statsGrid} aria-label={t("actualUsage")}>
    <Tooltip content={totalDetail} relationship="description">
      <Card className={styles.stat} appearance="filled-alternative">
        <Text className={styles.statLabel}>{t("workspaces")}</Text>
        <Title3 className={styles.statValue}>{summary ? summary.total_count : "—"}</Title3>
        <Caption1 className={styles.statHint}>{summary ? `${summary.state_counts.ready ?? 0} ${t("stateReady")}` : t("usageLoading")}</Caption1>
      </Card>
    </Tooltip>
    <UsageStat kind="cpu" summary={summary} quota={quota?.cpu_millis ?? null} observed={observed} stale={stale} />
    <UsageStat kind="memory" summary={summary} quota={quota?.memory_mib ?? null} observed={observed} stale={stale} />
    <UsageStat kind="disk" summary={summary} quota={quota?.disk_gib ?? null} observed={observed} stale={stale} />
    <UsageStat kind="temporary" summary={summary} quota={quota?.temporary_storage_gib ?? null} observed={observed} stale={stale} />
  </section>;
}

function UsageStat({ kind, summary, quota, observed, stale }: { kind: ResourceKind; summary: OrganizationUsageSummary | null; quota: number | null; observed: string; stale: boolean }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const requested = summary?.requested ?? EMPTY_RESOURCES;
  const actual = summary ? actualFor(kind, summary) : null;
  const requestedValue = requestedFor(kind, requested);
  const availability: UsageAvailability = kind === "temporary" ? "unknown" : summary?.availability[kind] ?? "unknown";
  const percentage = usagePercentage(actual, requestedValue);
  const value = actual === null ? "—" : format(kind, actual, t("cores"));
  const requestedDisplay = format(kind, requestedValue, t("cores"));
  const quotaDetail = quota === null ? "" : ` ${t("usageQuota")}: ${format(kind, kind === "disk" || kind === "temporary" ? quota * 1024 ** 3 : quota, t("cores"))}.`;
  const availabilityText = t(availabilityMessageKey(availabilityLabel(availability)));
  const detail = `${t("actualUsage")}: ${value}. ${t("usageRequested")}: ${requestedDisplay}. ${t("usageObservedAt")}: ${observed}. ${availabilityText}.${quotaDetail}`;
  const footnote = summary === null ? t("usageLoading") : actual === null ? `${t("usageRequested")}: ${requestedDisplay} · ${availabilityText}` : percentage === null ? t("usageNoRequested") : `${percentage}% ${t("usageOfOrganizationRequested")}${stale ? ` · ${t("usageStale")}` : ""}`;
  const fill = usageBarPercentage(percentage);
  return <Tooltip content={detail} relationship="description">
    <Card className={styles.stat} appearance="filled-alternative">
      {fill !== null && <span className={styles.statFill} style={{ width: `${fill}%` }} aria-hidden="true" />}
      <Text className={styles.statLabel}>{kind === "cpu" ? "CPU" : kind === "memory" ? t("memory") : kind === "disk" ? t("persistentDisk") : t("temporaryStorage")}</Text>
      <Title3 className={styles.statValue}>{value}</Title3>
      <Caption1 className={styles.statHint}>{footnote}</Caption1>
    </Card>
  </Tooltip>;
}

const EMPTY_RESOURCES: QuotaResources = { cpu_millis: 0, memory_mib: 0, disk_gib: 0, gpu_count: 0, temporary_storage_gib: 0 };
function actualFor(kind: ResourceKind, summary: OrganizationUsageSummary): number | null { return kind === "cpu" ? summary.actual.cpu_millis : kind === "memory" ? summary.actual.memory_mib : kind === "disk" ? summary.actual.disk_bytes : null; }
function requestedFor(kind: ResourceKind, resources: QuotaResources): number { return kind === "cpu" ? resources.cpu_millis : kind === "memory" ? resources.memory_mib : kind === "disk" ? resources.disk_gib * 1024 ** 3 : resources.temporary_storage_gib * 1024 ** 3; }
function format(kind: ResourceKind, value: number, cores: string): string { return kind === "cpu" ? `${formatCores(value)} ${cores}` : kind === "memory" ? `${formatGiB(value)} GiB` : `${formatBytesAsGiB(value)} GiB`; }
function availabilityMessageKey(value: UsageAvailability): "usageAvailable" | "usageUnavailable" | "usageUnknown" {
  return value === "available" ? "usageAvailable" : value === "unavailable" ? "usageUnavailable" : "usageUnknown";
}
