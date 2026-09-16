import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Badge, Button, Caption1, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Text } from "@fluentui/react-components";
import { AddRegular, ArrowSyncRegular, CopyRegular, DeleteRegular, DismissRegular, OpenRegular } from "@fluentui/react-icons";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import type { MessageKey } from "./i18n";
import { reserveWebShellWindow } from "./workspaceShell";
import { useWorkspaceStyles } from "./workspaces/workspaceStyles";
import {
  mappingUrl,
  parseInternalPort,
  safeBootstrapUrl,
  type PortMapping,
  type PortMappingsApi,
} from "./portMappings";

interface Props {
  api: PortMappingsApi;
  workspaceId: string;
  workspaceReady: boolean;
  onError?: (message: string) => void;
}

/** Port forwarding controls rendered only for an active workspace. */
export function WorkspacePortMappings({ api, workspaceId, workspaceReady, onError }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const loadingRef = useRef(false);
  const [items, setItems] = useState<PortMapping[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [port, setPort] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<PortMapping | null>(null);

  const report = useCallback((errorValue: unknown) => {
    const message = errorValue instanceof Error ? errorValue.message : t("portMappingRequestFailed");
    setError(message);
    onError?.(message);
  }, [onError, t]);

  const load = useCallback(async () => {
    // Polling and a user-triggered refresh should never queue an unbounded set
    // of requests while the control plane is slow.
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    setError(null);
    try { setItems(await api.portMappings(workspaceId)); }
    catch (errorValue) { report(errorValue); }
    finally {
      loadingRef.current = false;
      setLoading(false);
    }
  }, [api, report, workspaceId]);

  function openDialog() {
    if (dialogOpen) return;
    setDialogOpen(true);
    void load();
  }

  async function add(event: FormEvent) {
    event.preventDefault();
    if (!workspaceReady) {
      setError(t("portMappingWorkspaceNotReady"));
      return;
    }
    const internalPort = parseInternalPort(port);
    if (internalPort === null) {
      setError(t("portMappingInvalidPort"));
      return;
    }
    setSaving(true); setError(null);
    try {
      const created = await api.createPortMapping(workspaceId, {
        internal_port: internalPort,
        ...(displayName.trim() ? { display_name: displayName.trim() } : {}),
      });
      setItems((current) => [...current.filter((item) => item.id !== created.id), created]);
      setPort(""); setDisplayName("");
    } catch (errorValue) { report(errorValue); }
    finally { setSaving(false); }
  }

  async function remove() {
    const item = pendingDelete;
    if (!item) return;
    setDeleting(item.id); setError(null);
    try {
      await api.deletePortMapping(workspaceId, item.id);
      setItems((current) => current.filter((candidate) => candidate.id !== item.id));
      setPendingDelete(null);
    } catch (errorValue) { report(errorValue); }
    finally { setDeleting(null); }
  }

  useEffect(() => {
    if (!dialogOpen || !items.some((item) => item.status === "provisioning")) return;
    const timer = window.setInterval(() => void load(), 2_000);
    return () => window.clearInterval(timer);
  }, [dialogOpen, items, load]);

  return <>
    <Button type="button" appearance="outline" icon={<AddRegular />} onClick={openDialog}>{t("portMappings")}</Button>
    <Dialog open={dialogOpen} onOpenChange={(_, data) => setDialogOpen(data.open)}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody className={styles.dialogBody}>
          <DialogTitle action={<div className={styles.toolbarGroup}><Button appearance="subtle" icon={<ArrowSyncRegular />} disabled={loading} onClick={() => void load()}>{loading ? t("portMappingRefreshing") : t("portMappingRefresh")}</Button><Button appearance="subtle" icon={<DismissRegular />} aria-label={t("close")} onClick={() => setDialogOpen(false)} /></div>}>{t("portMappings")}</DialogTitle>
          <DialogContent className={styles.dialogBody}>
            <Text>{t("portMappingIntro")}</Text>
            <form className={styles.formGrid} onSubmit={(event) => void add(event)}>
              <Field label={t("internalPort")} required><Input disabled={!workspaceReady || saving} type="number" min={1} max={65535} step={1} required value={port} onChange={(_, data) => setPort(data.value)} placeholder={t("portMappingPortPlaceholder")} /></Field>
              <Field label={t("displayNameOptional")}><Input disabled={!workspaceReady || saving} value={displayName} maxLength={80} onChange={(_, data) => setDisplayName(data.value)} placeholder={t("portMappingNamePlaceholder")} /></Field>
              <Caption1 className={styles.formWide}>{t("portMappingPortHelp")}</Caption1>
              {!workspaceReady && <Caption1 className={styles.formWide} role="status">{t("portMappingWorkspaceNotReady")}</Caption1>}
              <div className={`${styles.formActions} ${styles.formWide}`}><Button type="submit" appearance="primary" icon={<AddRegular />} disabled={!workspaceReady || saving}>{saving ? t("portMappingAdding") : t("addMapping")}</Button></div>
            </form>
            {error && <Text role="alert">{error}</Text>}
            <section className={styles.portList} aria-live="polite">
              {loading && <Text>{t("portMappingsLoading")}</Text>}
              {!loading && !error && items.length === 0 && <Text>{t("noPortMappings")}</Text>}
              {!loading && items.map((item) => <PortMappingRow key={item.id} api={api} workspaceId={workspaceId} workspaceReady={workspaceReady} item={item} deleting={deleting === item.id} onDelete={() => setPendingDelete(item)} onError={report} />)}
            </section>
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
    <ConfirmDialog
      open={pendingDelete !== null}
      title={t("delete")}
      description={t("portMappingDeleteConfirm")}
      confirmLabel={deleting !== null ? t("deleting") : t("delete")}
      cancelLabel={t("cancel")}
      busy={deleting !== null}
      danger
      details={pendingDelete && <code>{pendingDelete.display_name || String(pendingDelete.internal_port)}</code>}
      onClose={() => setPendingDelete(null)}
      onConfirm={() => void remove()}
    />
  </>;
}

function PortMappingRow({ api, workspaceId, workspaceReady, item, deleting, onDelete, onError }: { api: PortMappingsApi; workspaceId: string; workspaceReady: boolean; item: PortMapping; deleting: boolean; onDelete: () => void; onError: (error: unknown) => void }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const [copied, setCopied] = useState(false);
  const [opening, setOpening] = useState(false);
  const url = mappingUrl(item);
  async function copy() {
    if (!url) return;
    try { await navigator.clipboard.writeText(url); setCopied(true); window.setTimeout(() => setCopied(false), 1_200); }
    catch { /* Clipboard permissions are optional; the URL remains selectable. */ }
  }
  async function open() {
    if (!workspaceReady || !url || item.status !== "ready" || opening) return;
    const target = reserveWebShellWindow(undefined, t("connectionPreparing"));
    setOpening(true);
    try {
      const bootstrap = await api.bootstrapPortMapping(workspaceId, item.id);
      const destination = safeBootstrapUrl(bootstrap.bootstrap_url);
      if (!destination) throw new Error(t("portMappingUnsafeBootstrapUrl"));
      if (target) target.location.replace(destination);
      else window.location.assign(destination);
    } catch (errorValue) { target?.close(); onError(errorValue); }
    finally { setOpening(false); }
  }
  return <article className={styles.portRow}>
    <div className={styles.portMain}><Text weight="semibold">{item.display_name || `${t("portLabel")} ${item.internal_port}`}</Text><Caption1>{t("internalPort")} <span className={styles.code}>{item.internal_port}</span></Caption1>{url ? <Text className={styles.code}>{url}</Text> : <Caption1>{t("portMappingAddressPending")}</Caption1>}</div>
    <div className={styles.portActions}><Badge appearance="tint" color={item.status === "ready" ? "success" : item.status === "failed" ? "danger" : "informative"}>{statusLabel(item.status, t)}</Badge><Button type="button" appearance="secondary" icon={<OpenRegular />} disabled={!workspaceReady || !url || opening || item.status !== "ready"} onClick={() => void open()}>{opening ? t("portMappingOpening") : t("open")}</Button><Button type="button" appearance="subtle" icon={<CopyRegular />} disabled={!url} onClick={() => void copy()}>{copied ? t("copied") : t("copyLink")}</Button>{!item.managed && <Button type="button" appearance="subtle" icon={<DeleteRegular />} className={styles.dangerButton} disabled={deleting} onClick={onDelete}>{deleting ? t("deleting") : t("delete")}</Button>}</div>
  </article>;
}

function statusLabel(status: string, t: (key: MessageKey) => string): string {
  if (status === "ready") return t("portMappingReady");
  if (status === "failed") return t("portMappingFailed");
  if (status === "deleting") return t("deleting");
  return t("portMappingProvisioning");
}
