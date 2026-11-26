import { useEffect, useRef, useState } from "react";
import { Button, Caption1, Field, Input, Select, Text } from "@fluentui/react-components";
import type { ApiClient } from "../api";
import { useI18n } from "../i18n";
import type { StorageGrowthStatus, StorageGrowthVolume, WorkspaceResponse } from "../types";
import { useWorkspaceStyles } from "./workspaceStyles";

interface Props {
  api: ApiClient;
  item: WorkspaceResponse;
  disabled: boolean;
  onUpdated: (updated: WorkspaceResponse) => void;
}

export function StorageGrowthControl({ api, item, disabled, onUpdated }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspace = item.workspace;
  const refresh = useRef(onUpdated);
  refresh.current = onUpdated;
  const [volume, setVolume] = useState<StorageGrowthVolume>("home");
  const [value, setValue] = useState("");
  const [operation, setOperation] = useState<StorageGrowthStatus | null>();
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = operation?.status === "pending" || operation?.status === "running";
  const homeGiB = workspace.resources.disk_gib;
  const temporaryGiB = workspace.storage_policy.temporary_storage_gib;
  const applied = volume === "home" ? homeGiB : temporaryGiB;
  const requested = Number(value);
  const maximum = volume === "home" ? 8_589_934_591 : 2048;
  const valid = Number.isSafeInteger(requested) && requested > applied && requested <= maximum;
  const locked = disabled || sending || active || operation === undefined;

  useEffect(() => {
    if (operation === null || (operation !== undefined && !active)) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const id = workspace.id;
    async function poll() {
      try {
        const result = await api.storageGrowth(id);
        if (cancelled) return;
        if (result?.status === "completed" && (active ||
          (result.volume === "home" ? homeGiB : temporaryGiB) !== result.requested_gib)) {
          const updated = await api.workspaceDetail(id);
          if (cancelled) return;
          refresh.current(updated);
          setValue("");
        }
        setOperation(result);
        if (!result || result.status === "completed" || result.status === "failed") return;
      } catch {
        if (cancelled) return;
        // Keep any known pending request visible during a temporary API outage.
      }
      if (!cancelled) timer = setTimeout(() => void poll(), 2500);
    }
    void poll();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [api, workspace.id, active, homeGiB, temporaryGiB, operation !== undefined]);

  async function submit() {
    if (locked || !valid) return;
    setSending(true);
    setError(null);
    try {
      setOperation(await api.startStorageGrowth(workspace.id, {
        volume, capacity_gib: requested, expected_generation: workspace.generation,
      }));
    } catch (failure) {
      const code = (failure as { code?: string } | null)?.code;
      if (code === "workspace_storage_growth_conflict") setOperation(undefined);
      setError(code === "workspace_storage_growth_unavailable" ? t("storageGrowthUnavailable")
        : code === "workspace_storage_growth_conflict" ? t("workspaceSettingsConflict")
        : code === "quota_exceeded" ? t("workspaceSettingsQuotaExceeded")
        : failure instanceof Error ? failure.message : t("operationFailed"));
    } finally {
      setSending(false);
    }
  }

  const operationDisk = t(operation?.volume === "home" ? "homeStorage" : "temporaryStorage");
  const status = operation && active ? t("storageGrowthPending", { disk: operationDisk, requested: operation.requested_gib })
    : operation?.status === "completed" ? t("storageGrowthCompleted", { disk: operationDisk, requested: operation.requested_gib })
    : operation?.status === "failed" ? t("storageGrowthFailed") : null;

  return <>
    <Caption1 className={styles.summaryLabel}>{t("storageGrowthHelp")}</Caption1>
    <div className={styles.formGrid}>
      <Field label={t("storageGrowthDisk")}>
        <Select value={volume} disabled={locked} onChange={(_, data) => {
          setVolume(data.value === "home" ? "home" : "temporary_storage");
          setValue("");
          setError(null);
        }}>
          <option value="home">{t("homeStorage")}</option>
          <option value="temporary_storage">{t("temporaryStorage")}</option>
        </Select>
      </Field>
      <Field label={t("storageGrowthTarget")}
        validationState={value !== "" && !valid ? "error" : undefined}
        validationMessage={value !== "" && !valid ? t("storageGrowthMinimum", { current: applied }) : undefined}>
        <Input type="number" min={applied + 1} max={maximum} step={1} value={value} disabled={locked} onChange={(_, data) => setValue(data.value)} contentAfter="GiB" />
      </Field>
      <div className={`${styles.formActions} ${styles.formWide}`}>
        <Button appearance="secondary" disabled={locked || !valid} onClick={() => void submit()}>{t(sending ? "saving" : "storageGrowthAction")}</Button>
      </div>
    </div>
    {status && <Text role={operation?.status === "failed" ? "alert" : "status"}>{status}</Text>}
    {error && <Text role="alert">{error}</Text>}
  </>;
}
