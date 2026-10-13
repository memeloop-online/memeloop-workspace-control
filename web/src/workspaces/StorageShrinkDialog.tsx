import { useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  Caption1,
  Checkbox,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Spinner,
  Text,
} from "@fluentui/react-components";
import { DeleteRegular, DismissRegular } from "@fluentui/react-icons";

import type { ApiClient } from "../api";
import { useI18n } from "../i18n";
import type { StorageShrinkOperation, WorkspaceResponse, WorkspaceRuntime } from "../types";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useWorkspaceStyles } from "./workspaceStyles";
import { formatBytesAsGiB } from "./usageSummaryPresentation";
import {
  shrinkFieldErrorKey,
  storageShrinkInFlight,
  storageShrinkPhaseLabelKey,
  storageShrinkReclaimable,
  validateStorageShrink,
} from "./storageShrink";

const POLL_INTERVAL_MS = 4_000;

interface Props {
  api: ApiClient;
  item: WorkspaceResponse | null;
  runtime?: WorkspaceRuntime;
  onClose: () => void;
  onSettled: () => void;
}

export function StorageShrinkDialog({ api, item, runtime, onClose, onSettled }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspace = item?.workspace;
  const workspaceId = workspace?.id ?? null;
  const [operation, setOperation] = useState<StorageShrinkOperation | null>(null);
  const [loading, setLoading] = useState(false);
  const [homeRaw, setHomeRaw] = useState("");
  const [temporaryRaw, setTemporaryRaw] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [reclaimSource, setReclaimSource] = useState(false);
  const [reclaimConfirming, setReclaimConfirming] = useState(false);
  const [reclaimSubmitting, setReclaimSubmitting] = useState(false);
  const [reclaimError, setReclaimError] = useState<string | null>(null);
  const watchedRef = useRef<string | null>(null);
  const notifiedRef = useRef<string | null>(null);

  useEffect(() => {
    setOperation(null);
    setHomeRaw("");
    setTemporaryRaw("");
    setSubmitError(null);
    setReclaimSource(false);
    setReclaimConfirming(false);
    setReclaimError(null);
    watchedRef.current = null;
    notifiedRef.current = null;
    if (!workspaceId) return;
    let active = true;
    setLoading(true);
    api.storageShrinkOperation(workspaceId)
      .then((latest) => { if (active) setOperation(latest); })
      .catch(() => { if (active) setOperation(null); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [api, workspaceId]);

  const inFlight = storageShrinkInFlight(operation);

  useEffect(() => {
    if (!workspaceId || !inFlight) return;
    let active = true;
    const timer = window.setInterval(() => {
      void api.storageShrinkOperation(workspaceId)
        .then((latest) => { if (active && latest) setOperation(latest); })
        .catch(() => {});
    }, POLL_INTERVAL_MS);
    return () => { active = false; window.clearInterval(timer); };
  }, [api, workspaceId, inFlight, operation?.id]);

  useEffect(() => {
    setReclaimConfirming(false);
    setReclaimError(null);
  }, [operation?.id, operation?.phase]);

  useEffect(() => {
    if (operation && storageShrinkInFlight(operation)) watchedRef.current = operation.id;
    const settledKey = operation ? `${operation.id}:${operation.phase}` : null;
    if (operation && !storageShrinkInFlight(operation) && watchedRef.current === operation.id && notifiedRef.current !== settledKey) {
      notifiedRef.current = settledKey;
      onSettled();
    }
  }, [operation, onSettled]);

  if (!workspace) return null;

  const liveRuntime = workspace.state === "stopped" ? undefined : runtime;
  const homeUsedBytes = liveRuntime?.persistent_storage?.used_bytes ?? null;
  const temporaryUsedBytes = liveRuntime?.temporary_storage?.used_bytes ?? null;
  const validation = validateStorageShrink(
    homeRaw,
    temporaryRaw,
    { currentGiB: workspace.resources.disk_gib, usedBytes: homeUsedBytes },
    { currentGiB: workspace.storage_policy.temporary_storage_gib, usedBytes: temporaryUsedBytes },
  );

  const busy = submitting || loading;
  const canSubmit = !busy && !inFlight && validation.targets !== null;

  async function submit() {
    if (!workspace || !validation.targets || submitting) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const started = await api.startStorageShrink(workspace.id, {
        ...validation.targets,
        expected_generation: workspace.generation,
        reclaim_source: reclaimSource,
      });
      setOperation(started);
    } catch (error) {
      setSubmitError(error instanceof Error ? error.message : t("operationFailed"));
    } finally {
      setSubmitting(false);
    }
  }

  async function confirmReclaim() {
    if (!workspace || !operation || reclaimSubmitting) return;
    setReclaimSubmitting(true);
    setReclaimError(null);
    try {
      setOperation(await api.confirmStorageShrinkReclaim(workspace.id, operation.id));
    } catch (error) {
      setReclaimError(error instanceof Error ? error.message : t("operationFailed"));
    } finally {
      setReclaimSubmitting(false);
    }
  }

  function usageText(usedBytes: number | null): string {
    return usedBytes === null ? t("usageNotObserved") : `${t("shrinkStorageUsed")} ${formatBytesAsGiB(usedBytes)} GiB`;
  }

  const title = t("shrinkStorageTitle");
  const closeButton = (
    <Button appearance="subtle" icon={<DismissRegular />} aria-label={t("cancel")} disabled={submitting} onClick={onClose} />
  );

  if (inFlight && operation) {
    const volumes = [
      { label: t("homeStorage"), volume: operation.home },
      { label: t("temporaryStorage"), volume: operation.temporary },
    ].filter(({ volume }) => volume.target_gib !== null);
    return (
      <Dialog open modalType="alert" onOpenChange={(_, data) => { if (!data.open) onClose(); }}>
        <DialogSurface className={styles.dialogSurface}>
          <DialogBody>
            <DialogTitle action={closeButton}>{title}</DialogTitle>
            <DialogContent className={styles.dialogBody}>
              <div role="status">
                <Spinner size="tiny" label={t(storageShrinkPhaseLabelKey(operation.phase))} labelPosition="after" />
              </div>
              {volumes.length > 0 && (
                <dl className={styles.dialogFacts}>
                  {volumes.map(({ label, volume }) => (
                    <div className={styles.dialogFact} key={label}>
                      <Text weight="semibold">{label}</Text>
                      <Text>
                        {volume.target_gib} GiB
                        {volume.used_bytes !== null && <> · {usageText(volume.used_bytes)}</>}
                        {volume.verified && <> <Badge appearance="tint" color="success">{t("shrinkVolumeVerified")}</Badge></>}
                      </Text>
                    </div>
                  ))}
                </dl>
              )}
              <Caption1>{t("shrinkStorageContinues")}</Caption1>
            </DialogContent>
            <DialogActions className={styles.dialogActions}>
              <Button appearance="secondary" onClick={onClose}>{t("close")}</Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    );
  }

  if (operation?.phase === "reclaimed") {
    return (
      <Dialog open modalType="alert" onOpenChange={(_, data) => { if (!data.open) onClose(); }}>
        <DialogSurface className={styles.dialogSurface}>
          <DialogBody>
            <DialogTitle action={closeButton}>{title}</DialogTitle>
            <DialogContent className={styles.dialogBody}>
              <MessageBar intent="success"><MessageBarBody>{t("shrinkStorageReclaimed")}</MessageBarBody></MessageBar>
            </DialogContent>
            <DialogActions className={styles.dialogActions}>
              <Button appearance="secondary" onClick={() => setOperation(null)}>{t("shrinkStorageAgain")}</Button>
              <Button appearance="primary" onClick={onClose}>{t("close")}</Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    );
  }

  if (operation?.phase === "completed") {
    const reclaimable = storageShrinkReclaimable(operation);
    return (
      <>
      <Dialog open modalType="alert" onOpenChange={(_, data) => { if (!data.open) onClose(); }}>
        <DialogSurface className={styles.dialogSurface}>
          <DialogBody>
            <DialogTitle action={closeButton}>{title}</DialogTitle>
            <DialogContent className={styles.dialogBody}>
              <MessageBar intent="success"><MessageBarBody>{operation.resume ? t("shrinkStorageSucceeded") : t("shrinkStorageSucceededStopped")}</MessageBarBody></MessageBar>
              {operation.sources_retained && (
                <>
                  <Text as="p">{t("shrinkStorageRetained")}</Text>
                  {reclaimable && operation.error && <MessageBar intent="error"><MessageBarBody>{t("shrinkReclaimFailed")} {operation.error}</MessageBarBody></MessageBar>}
                  {reclaimable && reclaimError && <MessageBar intent="error"><MessageBarBody>{reclaimError}</MessageBarBody></MessageBar>}
                </>
              )}
            </DialogContent>
            <DialogActions className={styles.dialogActions}>
              {reclaimable && (
                <Button appearance="secondary" icon={<DeleteRegular />} disabled={reclaimSubmitting} onClick={() => setReclaimConfirming(true)}>
                  {t("shrinkStorageReclaim")}
                </Button>
              )}
              <Button appearance="secondary" onClick={() => setOperation(null)}>{t("shrinkStorageAgain")}</Button>
              <Button appearance="primary" onClick={onClose}>{t("close")}</Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <ConfirmDialog
        open={reclaimConfirming}
        title={t("shrinkStorageReclaim")}
        description={t("shrinkStorageReclaimWarning")}
        details={workspace.name}
        confirmLabel={t("shrinkStorageReclaimConfirm")}
        cancelLabel={t("cancel")}
        busy={reclaimSubmitting}
        danger
        onClose={() => { if (!reclaimSubmitting) setReclaimConfirming(false); }}
        onConfirm={() => void confirmReclaim()}
      />
      </>
    );
  }

  return (
    <Dialog open onOpenChange={(_, data) => { if (!data.open && !submitting) onClose(); }}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody>
          <DialogTitle action={closeButton}>{title}</DialogTitle>
          <DialogContent className={styles.dialogBody}>
            <Text as="p">{t("shrinkStorageHelp")}</Text>
            <MessageBar intent="warning"><MessageBarBody>{t("shrinkStorageDowntime")}</MessageBarBody></MessageBar>
            {operation?.phase === "failed" && (
              <MessageBar intent="error">
                <MessageBarBody>
                  {operation.switched ? t("shrinkRecoveryFailed") : t("shrinkStorageFailed")} {operation.error ?? ""}
                </MessageBarBody>
              </MessageBar>
            )}
            {submitError && <MessageBar intent="error"><MessageBarBody>{submitError}</MessageBarBody></MessageBar>}
            <dl className={styles.dialogFacts}>
              <div className={styles.dialogFact}>
                <Text weight="semibold">{t("homeStorage")}</Text>
                <Text>{t("shrinkStorageCurrent")} {workspace.resources.disk_gib} GiB · {usageText(homeUsedBytes)}</Text>
              </div>
              <div className={styles.dialogFact}>
                <Text weight="semibold">{t("temporaryStorage")}</Text>
                <Text>{t("shrinkStorageCurrent")} {workspace.storage_policy.temporary_storage_gib} GiB · {usageText(temporaryUsedBytes)}</Text>
              </div>
            </dl>
            {(homeUsedBytes === null || temporaryUsedBytes === null) && (
              <Caption1>{t("shrinkStorageUnknownUsage")}</Caption1>
            )}
            <Field
              label={`${t("homeStorage")} · ${t("shrinkStorageTargetGiB")}`}
              validationState={validation.homeError ? "error" : undefined}
              validationMessage={validation.homeError ? t(shrinkFieldErrorKey(validation.homeError)) : undefined}
            >
              <Input
                type="number"
                min={1}
                max={workspace.resources.disk_gib - 1}
                step={1}
                value={homeRaw}
                disabled={busy}
                onChange={(_, data) => setHomeRaw(data.value)}
              />
            </Field>
            <Field
              label={`${t("temporaryStorage")} · ${t("shrinkStorageTargetGiB")}`}
              validationState={validation.temporaryError ? "error" : undefined}
              validationMessage={validation.temporaryError ? t(shrinkFieldErrorKey(validation.temporaryError)) : undefined}
            >
              <Input
                type="number"
                min={1}
                max={workspace.storage_policy.temporary_storage_gib - 1}
                step={1}
                value={temporaryRaw}
                disabled={busy}
                onChange={(_, data) => setTemporaryRaw(data.value)}
              />
            </Field>
            <Caption1>{t("shrinkStorageKeepHint")}</Caption1>
            <Checkbox
              checked={reclaimSource}
              disabled={busy}
              onChange={(_, data) => setReclaimSource(Boolean(data.checked))}
              label={t("shrinkStorageAutoReclaim")}
            />
            {reclaimSource && (
              <MessageBar intent="warning"><MessageBarBody>{t("shrinkStorageAutoReclaimWarning")}</MessageBarBody></MessageBar>
            )}
          </DialogContent>
          <DialogActions className={styles.dialogActions}>
            <Button appearance="secondary" disabled={submitting} onClick={onClose}>{t("cancel")}</Button>
            <Button appearance="primary" disabled={!canSubmit} onClick={() => void submit()}>
              {submitting ? t("shrinkStorageStarting") : t("shrinkStorageSubmit")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
