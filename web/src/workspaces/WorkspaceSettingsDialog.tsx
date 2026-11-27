import { useEffect, useState } from "react";
import {
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  Spinner,
  Text,
} from "@fluentui/react-components";
import { DismissRegular, EditRegular, LocationRegular, SaveRegular } from "@fluentui/react-icons";

import type { ApiClient } from "../api";
import { useI18n } from "../i18n";
import type { WorkspaceResponse } from "../types";
import { useWorkspaceStyles } from "./workspaceStyles";
import { StorageGrowthControl } from "./StorageGrowthControl";
import { formatBytesAsGiB } from "./usageSummaryPresentation";

/** Matches the backend's 120 Unicode code point workspace-name limit. */
export const WORKSPACE_NAME_MAX_CODE_POINTS = 120;

export function codePointLength(value: string): number {
  return Array.from(value).length;
}

interface Props {
  api: ApiClient;
  item: WorkspaceResponse | null;
  canChangeState: boolean;
  canManageTemporaryStorage: boolean;
  canChangePlacement: boolean;
  canShrinkStorage: boolean;
  onChangePlacement: (item: WorkspaceResponse) => void;
  onShrinkStorage: (item: WorkspaceResponse) => void;
  /** Receives the server's authoritative response so the card can update in place. */
  onUpdated: (updated: WorkspaceResponse) => void;
  onClose: () => void;
}

interface Draft {
  name: string;
  cpu: string;
  memory: string;
  gpu: string;
  temporaryStorage: string;
}

function draftFor(item: WorkspaceResponse): Draft {
  const workspace = item.workspace;
  return {
    name: workspace.name,
    cpu: String(workspace.resources.cpu_millis),
    memory: String(workspace.resources.memory_mib),
    gpu: String(workspace.resources.gpu_count),
    temporaryStorage: String(workspace.storage_policy.temporary_storage_gib),
  };
}

/**
 * Edits workspace settings. An eligible memory increase can apply while
 * running. The draft keeps the generation it was based on. Only reloading
 * the draft or saving one of its sections can advance that generation.
 */
export function WorkspaceSettingsDialog({ api, item, canChangeState, canManageTemporaryStorage, canChangePlacement, canShrinkStorage, onChangePlacement, onShrinkStorage, onUpdated, onClose }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspaceId = item?.workspace.id ?? null;
  const [draftGeneration, setDraftGeneration] = useState(item?.workspace.generation ?? 0);
  const [draft, setDraft] = useState<Draft>(() => item ? draftFor(item) : { name: "", cpu: "", memory: "", gpu: "", temporaryStorage: "" });
  const [savingSettings, setSavingSettings] = useState(false);
  const [savingStorage, setSavingStorage] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [saveConflict, setSaveConflict] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [storageNotice, setStorageNotice] = useState<string | null>(null);
  const [memoryObservation, setMemoryObservation] = useState<{ id: string; generation: number; applied: number | null; failed: boolean } | null>(null);
  const liveResizeEligible = item?.workspace.state === "ready" && item.live_memory_resize_supported === true;
  const requestedMemory = item?.workspace.resources.memory_mib ?? 0;
  const settingsGeneration = item?.workspace.generation ?? 0;

  useEffect(() => {
    if (!item) return;
    setDraftGeneration(item.workspace.generation);
    setDraft(draftFor(item));
    setSaveConflict(false);
    setError(null);
    setStorageNotice(null);
  }, [workspaceId]);

  useEffect(() => {
    if (!workspaceId || !liveResizeEligible || !canChangeState) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = Date.now() + 190_000;
    const load = async () => {
      try {
        const observation = await api.workspaceRuntime(workspaceId);
        if (cancelled) return;
        const applied = observation.pods.find((pod) => pod.ready && pod.phase === "Running")?.containers
          ?.find((container) => container.name === "workspace")?.applied_memory_mib ?? null;
        const failed = Boolean(observation.memory_resize_failure) && applied !== requestedMemory;
        setMemoryObservation({ id: workspaceId, generation: settingsGeneration, applied, failed });
        if (failed || applied === requestedMemory) return;
      } catch {
        // Retry without claiming the saved limit has taken effect.
      }
      if (!cancelled && Date.now() < deadline) timer = setTimeout(() => void load(), 3_000);
    };
    void load();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [api, canChangeState, workspaceId, liveResizeEligible, requestedMemory, settingsGeneration]);

  if (!item) return null;
  const workspace = item.workspace;
  const stopped = workspace.state === "stopped";
  const busy = savingSettings || savingStorage || reloading;
  const stale = draftGeneration !== settingsGeneration || saveConflict;
  const nameLength = codePointLength(draft.name.trim());
  const nameTooLong = nameLength > WORKSPACE_NAME_MAX_CODE_POINTS;
  const nameInvalid = draft.name.trim().length === 0 || nameTooLong;
  const cpu = Number(draft.cpu);
  const memory = Number(draft.memory);
  const gpu = Number(draft.gpu);
  const currentMemory = memoryObservation?.id === workspaceId && memoryObservation.generation === settingsGeneration ? memoryObservation : null;
  const appliedMemory = currentMemory?.applied ?? null;
  const resizeFailure = currentMemory?.failed ?? false;
  const cpuValid = Number.isInteger(cpu) && cpu >= workspace.pod_requests.cpu_millis;
  const memoryValid = Number.isInteger(memory) && memory >= workspace.pod_requests.memory_mib
    && (!liveResizeEligible || memory >= requestedMemory);
  const memoryIncreaseInvalid = liveResizeEligible && !stopped && Number.isInteger(memory) && memory < requestedMemory;
  const gpuValid = Number.isInteger(gpu) && gpu >= 0;
  const settingsChanged = draft.name.trim() !== workspace.name
    || (stopped && (cpu !== workspace.resources.cpu_millis || memory !== requestedMemory || gpu !== workspace.resources.gpu_count))
    || (liveResizeEligible && !stopped && Number.isInteger(memory) && memory !== requestedMemory);
  const settingsValid = (stopped || workspace.state === "ready") && !nameInvalid
    && (stopped ? cpuValid && memoryValid && gpuValid : !liveResizeEligible || memoryValid);
  const currentTemporary = workspace.storage_policy.temporary_storage_gib;
  const temporary = Number(draft.temporaryStorage);
  const temporaryValid = Number.isInteger(temporary) && temporary >= 1 && temporary <= 2048;
  const temporaryChanged = temporary !== currentTemporary;
  const temporaryEditable = stopped && canManageTemporaryStorage && !workspace.scratch_volume_binding;

  async function saveSettings() {
    if (!canChangeState || !settingsValid || !settingsChanged || busy || stale) return;
    setSavingSettings(true); setError(null); setStorageNotice(null);
    try {
      const updated = await api.updateWorkspaceSettings(workspace.id, {
        name: draft.name.trim(),
        cpu_millis: stopped ? cpu : workspace.resources.cpu_millis,
        memory_mib: stopped || liveResizeEligible ? memory : workspace.resources.memory_mib,
        gpu_count: stopped ? gpu : workspace.resources.gpu_count,
        expected_generation: draftGeneration,
      });
      setDraftGeneration(updated.workspace.generation);
      setDraft((current) => ({ ...draftFor(updated), temporaryStorage: current.temporaryStorage }));
      onUpdated(updated);
    } catch (errorValue) {
      handleSaveError(errorValue);
    } finally { setSavingSettings(false); }
  }

  async function saveTemporaryStorage() {
    if (!temporaryEditable || !temporaryValid || !temporaryChanged || busy || stale) return;
    setSavingStorage(true); setError(null); setStorageNotice(null);
    try {
      const updated = await api.updateWorkspaceTemporaryStorage(workspace.id, {
        temporary_storage_gib: temporary,
        expected_generation: draftGeneration,
      });
      setDraftGeneration(updated.workspace.generation);
      setDraft((current) => ({ ...current, temporaryStorage: String(updated.workspace.storage_policy.temporary_storage_gib) }));
      onUpdated(updated);
      setStorageNotice(t("settingsStorageSaved"));
    } catch (errorValue) {
      handleSaveError(errorValue);
    } finally { setSavingStorage(false); }
  }

  function handleSaveError(failure: unknown) {
    const code = (failure as { code?: string } | null)?.code;
    if (code === "workspace_settings_update_conflict" || code === "workspace_temporary_storage_update_conflict") {
      setSaveConflict(true);
    } else {
      setError(messageOf(failure, t));
    }
  }

  async function reloadSettings() {
    setReloading(true); setError(null);
    try {
      const updated = await api.workspaceDetail(workspace.id);
      setDraft(draftFor(updated));
      setDraftGeneration(updated.workspace.generation);
      setSaveConflict(false);
      setStorageNotice(null);
      onUpdated(updated);
    } catch (failure) {
      setError(messageOf(failure, t));
    } finally { setReloading(false); }
  }

  return (
    <Dialog open onOpenChange={(_, data) => { if (!data.open && !busy) onClose(); }}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody>
          <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={t("cancel")} disabled={busy} onClick={onClose} />}>{t("workspaceSettingsTitle", { name: workspace.name })}</DialogTitle>
          <DialogContent className={styles.dialogBody}>
            {!canChangeState && <Text role="status">{t("workspaceSettingsPermissionRequired")}</Text>}
            <section className={styles.dialogSection} aria-label={t("workspaceGeneralSettings")}>
              <Field label={t("workspaceName")} required validationState={nameInvalid ? "error" : undefined} validationMessage={nameTooLong ? t("workspaceSettingsNameTooLong") : draft.name.trim() ? undefined : t("workspaceNameRequired")}>
                <Input aria-label={t("workspaceName")} value={draft.name} disabled={!canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, name: data.value }))} />
              </Field>
              <Caption1 className={styles.summaryLabel} aria-live="polite">{t("workspaceSettingsNameCounter", { count: nameLength, max: WORKSPACE_NAME_MAX_CODE_POINTS })}</Caption1>
            </section>
            <section className={styles.dialogSection} aria-label={t("workspaceComputeSettings")}>
              <Text weight="semibold">{t("workspaceComputeSettings")}</Text>
              {!stopped && !liveResizeEligible && <Caption1 className={styles.summaryLabel}>{t("workspaceSettingsComputeNeedsStop")}</Caption1>}
              {liveResizeEligible && <Caption1 className={styles.summaryLabel}>{t("workspaceSettingsMemoryLiveResizeHint")}</Caption1>}
              <div className={styles.formGrid}>
                <Field label={t("workspaceCpuMillis")} validationState={stopped && !cpuValid ? "error" : undefined} validationMessage={stopped && !cpuValid ? t("workspaceSettingsCpuMin", { value: workspace.pod_requests.cpu_millis }) : undefined}>
                  <Input type="number" min={workspace.pod_requests.cpu_millis} step={1} value={draft.cpu} disabled={!stopped || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, cpu: data.value }))} contentAfter="m" />
                </Field>
                <Field label={t("workspaceMemoryMiB")} validationState={(stopped && !memoryValid) || memoryIncreaseInvalid ? "error" : undefined} validationMessage={stopped && !memoryValid ? t("workspaceSettingsMemoryMin", { value: workspace.pod_requests.memory_mib }) : memoryIncreaseInvalid ? t("workspaceSettingsMemoryIncreaseMin", { value: requestedMemory }) : undefined}>
                  <Input type="number" min={stopped ? workspace.pod_requests.memory_mib : liveResizeEligible ? requestedMemory : workspace.pod_requests.memory_mib} step={1} value={draft.memory} disabled={(!stopped && !liveResizeEligible) || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, memory: data.value }))} contentAfter="MiB" />
                </Field>
                <Field label={t("workspaceGpuCount")} validationState={stopped && !gpuValid ? "error" : undefined} validationMessage={stopped && !gpuValid ? t("workspaceSettingsGpuInvalid") : undefined}>
                  <Input type="number" min={0} step={1} value={draft.gpu} disabled={!stopped || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, gpu: data.value }))} />
                </Field>
              </div>
              {liveResizeEligible && canChangeState && <Caption1 className={styles.summaryLabel} aria-live="polite">
                {resizeFailure
                  ? t("workspaceSettingsMemoryResizeFailed")
                  : appliedMemory === null
                    ? t("workspaceSettingsMemoryUnconfirmed", { requested: requestedMemory })
                  : appliedMemory !== requestedMemory
                    ? t("workspaceSettingsMemoryResizePending", { requested: requestedMemory, applied: appliedMemory })
                    : t("workspaceSettingsMemoryApplied", { value: appliedMemory })}
              </Caption1>}
              <Caption1 className={styles.summaryLabel}>{t("workspaceSettingsComputeHelp", { cpu: workspace.pod_requests.cpu_millis, memory: workspace.pod_requests.memory_mib })}</Caption1>
            </section>
            <section className={styles.dialogSection} aria-label={t("workspaceStorageSettings")}>
              <Text weight="semibold">{t("workspaceStorageSettings")}</Text>
              <dl className={styles.dialogFacts}>
                <div className={styles.dialogFact}>
                  <Text>{t("homeStorage")}</Text>
                  <Text>{formatBytesAsGiB(workspace.resources.disk_gib * 1024 ** 3)} GiB · {t("workspaceHomeStorageHelp")}</Text>
                </div>
                <div className={styles.dialogFact}>
                  <Text>{t("temporaryStorage")}</Text>
                  <Text>{currentTemporary} GiB · {t("workspaceTemporaryStorageHelp")}</Text>
                </div>
              </dl>
              {workspace.state === "ready" && canManageTemporaryStorage ? <StorageGrowthControl key={workspace.id} api={api} item={item} disabled={busy} onUpdated={onUpdated} /> : temporaryEditable ? <div className={styles.formGrid}>
                <Field label={t("workspaceTemporaryStorageGiB")} validationState={!temporaryValid ? "error" : undefined} validationMessage={!temporaryValid ? t("workspaceSettingsTemporaryStorageInvalid") : undefined}>
                  <Input type="number" min={1} max={2048} step={1} value={draft.temporaryStorage} disabled={busy} onChange={(_, data) => setDraft((current) => ({ ...current, temporaryStorage: data.value }))} />
                </Field>
                <div className={`${styles.formActions} ${styles.formWide}`}>
                  <Button appearance="secondary" icon={<SaveRegular />} disabled={!temporaryValid || !temporaryChanged || busy || stale} onClick={() => void saveTemporaryStorage()}>{savingStorage ? t("saving") : t("saveTemporaryStorage")}</Button>
                </div>
              </div> : <Caption1 className={styles.summaryLabel}>{!stopped ? t("workspaceTemporaryStorageNeedsStop") : workspace.scratch_volume_binding ? t("workspaceTemporaryStorageBoundVolume") : t("workspaceTemporaryStorageAdminOnly")}</Caption1>}
              <Caption1 className={styles.summaryLabel}>{t("workspaceShrinkStorageHint")}</Caption1>
              <div className={styles.formActions}>
                {canShrinkStorage && <Button appearance="subtle" icon={<EditRegular />} disabled={busy} onClick={() => onShrinkStorage(item)}>{t("shrinkStorage")}</Button>}
                {stopped && canChangePlacement && <Button appearance="subtle" icon={<LocationRegular />} disabled={busy} onClick={() => onChangePlacement(item)}>{t("changeLocation")}</Button>}
              </div>
              <Caption1 className={styles.summaryLabel}>{t("workspaceLocationChangeHelp")}</Caption1>
            </section>
            {stale && <div>
              <Text role="alert">{t("workspaceSettingsDraftStale")}</Text>{" "}
              <Button appearance="subtle" disabled={busy} onClick={() => void reloadSettings()}>{t("workspaceSettingsReload")}</Button>
            </div>}
            {error && <Text role="alert">{error}</Text>}
            {storageNotice && <Text role="status">{storageNotice}</Text>}
            {savingSettings && <Spinner size="tiny" label={t("saving")} />}
          </DialogContent>
          <DialogActions className={styles.dialogActions}>
            <Button appearance="secondary" disabled={busy} onClick={onClose}>{t("cancel")}</Button>
            <Button appearance="primary" icon={<SaveRegular />} disabled={!canChangeState || !settingsValid || !settingsChanged || busy || stale} onClick={() => void saveSettings()}>{savingSettings ? t("saving") : t("saveSettings")}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

function messageOf(error: unknown, t: ReturnType<typeof useI18n>["t"]): string {
  const code = (error as { code?: string } | null)?.code;
  if (code === "quota_exceeded") return t("workspaceSettingsQuotaExceeded");
  if (code === "invalid_workspace") return t("workspaceResourcesInvalid");
  return error instanceof Error ? error.message : t("operationFailed");
}
