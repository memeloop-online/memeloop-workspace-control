import { useEffect, useRef, useState } from "react";
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
 * Edits a workspace name always, and compute/temporary storage only while it
 * is stopped. Settings and temporary storage save separately: each response
 * carries the newest generation, so a later save never uses a stale
 * precondition, and a failed storage save cannot undo a saved rename.
 */
export function WorkspaceSettingsDialog({ api, item, canChangeState, canManageTemporaryStorage, canChangePlacement, canShrinkStorage, onChangePlacement, onShrinkStorage, onUpdated, onClose }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspaceId = item?.workspace.id ?? null;
  const generation = useRef(item?.workspace.generation ?? 0);
  generation.current = item?.workspace.generation ?? 0;
  const [draft, setDraft] = useState<Draft>(() => item ? draftFor(item) : { name: "", cpu: "", memory: "", gpu: "", temporaryStorage: "" });
  const [savingSettings, setSavingSettings] = useState(false);
  const [savingStorage, setSavingStorage] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [storageNotice, setStorageNotice] = useState<string | null>(null);

  useEffect(() => {
    if (!item) return;
    generation.current = item.workspace.generation;
    setDraft(draftFor(item));
    setError(null);
    setStorageNotice(null);
  }, [workspaceId]);

  if (!item) return null;
  const workspace = item.workspace;
  const stopped = workspace.state === "stopped";
  const busy = savingSettings || savingStorage;
  const nameLength = codePointLength(draft.name.trim());
  const nameTooLong = nameLength > WORKSPACE_NAME_MAX_CODE_POINTS;
  const nameInvalid = draft.name.trim().length === 0 || nameTooLong;
  const cpu = Number(draft.cpu);
  const memory = Number(draft.memory);
  const gpu = Number(draft.gpu);
  const cpuValid = Number.isInteger(cpu) && cpu >= workspace.pod_requests.cpu_millis;
  const memoryValid = Number.isInteger(memory) && memory >= workspace.pod_requests.memory_mib;
  const gpuValid = Number.isInteger(gpu) && gpu >= 0;
  const settingsChanged = draft.name.trim() !== workspace.name
    || (stopped && (cpu !== workspace.resources.cpu_millis || memory !== workspace.resources.memory_mib || gpu !== workspace.resources.gpu_count));
  const settingsValid = (stopped || workspace.state === "ready") && !nameInvalid && (!stopped || (cpuValid && memoryValid && gpuValid));
  const currentTemporary = workspace.storage_policy.temporary_storage_gib;
  const temporary = Number(draft.temporaryStorage);
  const temporaryValid = Number.isInteger(temporary) && temporary >= 1 && temporary <= 2048;
  const temporaryChanged = temporary !== currentTemporary;
  const temporaryEditable = stopped && canManageTemporaryStorage && !workspace.scratch_volume_binding;

  async function saveSettings() {
    if (!canChangeState || !settingsValid || !settingsChanged || busy) return;
    setSavingSettings(true); setError(null); setStorageNotice(null);
    try {
      const updated = await api.updateWorkspaceSettings(workspace.id, {
        name: draft.name.trim(),
        cpu_millis: stopped ? cpu : workspace.resources.cpu_millis,
        memory_mib: stopped ? memory : workspace.resources.memory_mib,
        gpu_count: stopped ? gpu : workspace.resources.gpu_count,
        expected_generation: generation.current,
      });
      generation.current = updated.workspace.generation;
      onUpdated(updated);
    } catch (errorValue) {
      // The draft stays intact; the caller refreshes authoritative data.
      setError(messageOf(errorValue, t));
    } finally { setSavingSettings(false); }
  }

  async function saveTemporaryStorage() {
    if (!temporaryEditable || !temporaryValid || !temporaryChanged || busy) return;
    setSavingStorage(true); setError(null); setStorageNotice(null);
    try {
      const updated = await api.updateWorkspaceTemporaryStorage(workspace.id, {
        temporary_storage_gib: temporary,
        expected_generation: generation.current,
      });
      generation.current = updated.workspace.generation;
      setDraft((current) => ({ ...current, temporaryStorage: String(updated.workspace.storage_policy.temporary_storage_gib) }));
      onUpdated(updated);
      setStorageNotice(t("settingsStorageSaved"));
    } catch (errorValue) {
      setError(messageOf(errorValue, t));
    } finally { setSavingStorage(false); }
  }

  return (
    <Dialog open onOpenChange={(_, data) => { if (!data.open && !busy) onClose(); }}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody>
          <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={t("cancel")} disabled={busy} onClick={onClose} />}>{t("editParameters")}</DialogTitle>
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
              {!stopped && <Caption1 className={styles.summaryLabel}>{t("workspaceSettingsComputeNeedsStop")}</Caption1>}
              <div className={styles.formGrid}>
                <Field label={t("workspaceCpuMillis")} validationState={stopped && !cpuValid ? "error" : undefined} validationMessage={stopped && !cpuValid ? t("workspaceSettingsCpuMin", { value: workspace.pod_requests.cpu_millis }) : undefined}>
                  <Input type="number" min={workspace.pod_requests.cpu_millis} step={1} value={draft.cpu} disabled={!stopped || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, cpu: data.value }))} contentAfter="m" />
                </Field>
                <Field label={t("workspaceMemoryMiB")} validationState={stopped && !memoryValid ? "error" : undefined} validationMessage={stopped && !memoryValid ? t("workspaceSettingsMemoryMin", { value: workspace.pod_requests.memory_mib }) : undefined}>
                  <Input type="number" min={workspace.pod_requests.memory_mib} step={1} value={draft.memory} disabled={!stopped || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, memory: data.value }))} contentAfter="MiB" />
                </Field>
                <Field label={t("workspaceGpuCount")} validationState={stopped && !gpuValid ? "error" : undefined} validationMessage={stopped && !gpuValid ? t("workspaceSettingsGpuInvalid") : undefined}>
                  <Input type="number" min={0} step={1} value={draft.gpu} disabled={!stopped || !canChangeState || busy} onChange={(_, data) => setDraft((current) => ({ ...current, gpu: data.value }))} />
                </Field>
              </div>
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
              {temporaryEditable ? <div className={styles.formGrid}>
                <Field label={t("workspaceTemporaryStorageGiB")} validationState={!temporaryValid ? "error" : undefined} validationMessage={!temporaryValid ? t("workspaceSettingsTemporaryStorageInvalid") : undefined}>
                  <Input type="number" min={1} max={2048} step={1} value={draft.temporaryStorage} disabled={busy} onChange={(_, data) => setDraft((current) => ({ ...current, temporaryStorage: data.value }))} />
                </Field>
                <div className={`${styles.formActions} ${styles.formWide}`}>
                  <Button appearance="secondary" icon={<SaveRegular />} disabled={!temporaryValid || !temporaryChanged || busy} onClick={() => void saveTemporaryStorage()}>{savingStorage ? t("saving") : t("saveTemporaryStorage")}</Button>
                </div>
              </div> : <Caption1 className={styles.summaryLabel}>{!stopped ? t("workspaceTemporaryStorageNeedsStop") : workspace.scratch_volume_binding ? t("workspaceTemporaryStorageBoundVolume") : t("workspaceTemporaryStorageAdminOnly")}</Caption1>}
              <Caption1 className={styles.summaryLabel}>{t("workspaceShrinkStorageHint")}</Caption1>
              <div className={styles.formActions}>
                {canShrinkStorage && <Button appearance="subtle" icon={<EditRegular />} disabled={busy} onClick={() => onShrinkStorage(item)}>{t("shrinkStorage")}</Button>}
                {stopped && canChangePlacement && <Button appearance="subtle" icon={<LocationRegular />} disabled={busy} onClick={() => onChangePlacement(item)}>{t("changeLocation")}</Button>}
              </div>
              <Caption1 className={styles.summaryLabel}>{t("workspaceLocationChangeHelp")}</Caption1>
            </section>
            {error && <Text role="alert">{error}</Text>}
            {storageNotice && <Text role="status">{storageNotice}</Text>}
            {savingSettings && <Spinner size="tiny" label={t("saving")} />}
          </DialogContent>
          <DialogActions className={styles.dialogActions}>
            <Button appearance="secondary" disabled={busy} onClick={onClose}>{t("cancel")}</Button>
            <Button appearance="primary" icon={<SaveRegular />} disabled={!canChangeState || !settingsValid || !settingsChanged || busy} onClick={() => void saveSettings()}>{savingSettings ? t("saving") : t("saveSettings")}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

function messageOf(error: unknown, t: ReturnType<typeof useI18n>["t"]): string {
  const code = (error as { code?: string } | null)?.code;
  if (code === "workspace_settings_update_conflict" || code === "workspace_temporary_storage_update_conflict") return t("workspaceSettingsConflict");
  if (code === "quota_exceeded") return t("workspaceSettingsQuotaExceeded");
  if (code === "invalid_workspace") return t("workspaceResourcesInvalid");
  return error instanceof Error ? error.message : t("operationFailed");
}
