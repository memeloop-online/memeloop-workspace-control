import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Badge, Button, Caption1, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Text } from "@fluentui/react-components";
import { AddRegular, ArrowSyncRegular, CopyRegular, DeleteRegular, DismissRegular, EditRegular, OpenRegular } from "@fluentui/react-icons";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import type { MessageKey } from "./i18n";
import { reserveWebShellWindow } from "./workspaceShell";
import { useWorkspaceStyles } from "./workspaces/workspaceStyles";
import {
  DISPLAY_NAME_MAX_CODE_POINTS,
  mappingUrl,
  parseInternalPort,
  safeBootstrapUrl,
  validateDisplayName,
  validateSubdomain,
  type PortMapping,
  type PortMappingsApi,
} from "./portMappings";

interface Props {
  api: PortMappingsApi;
  workspaceId: string;
  /** Full workspace state: mappings can be browsed and edited while stopped. */
  workspaceState: string;
  onError?: (message: string) => void;
}

/** Draft for the shared add/edit form; text stays until a save succeeds. */
interface MappingDraft {
  /** Null while adding; the row id while editing. */
  id: string | null;
  port: string;
  displayName: string;
  /** Optional globally unique DNS label; blank keeps the automatic/current label. */
  subdomain: string;
}

const EMPTY_DRAFT: MappingDraft = { id: null, port: "", displayName: "", subdomain: "" };

/** Port forwarding controls rendered for a ready or stopped workspace. */
export function WorkspacePortMappings({ api, workspaceId, workspaceState, onError }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspaceReady = workspaceState === "ready";
  const workspaceStopped = workspaceState === "stopped";
  const loadingRef = useRef(false);
  const mutationRevision = useRef(0);
  const [items, setItems] = useState<PortMapping[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<MappingDraft>(EMPTY_DRAFT);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<PortMapping | null>(null);
  const nameValidation = validateDisplayName(draft.displayName);
  const subdomainValidation = validateSubdomain(draft.subdomain);
  const editing = draft.id !== null;
  const busy = saving || deleting !== null;

  const report = useCallback((errorValue: unknown) => {
    const failure = errorValue as (Error & { code?: string }) | null;
    const code = failure?.code;
    // Validation and conflict failures stay inline; they are not global errors.
    if (code === "port_mapping_port_conflict") { setError(t("portMappingConflictUseEdit")); return; }
    if (code === "invalid_port_mapping_display_name") { setError(t("portMappingNameTooLong")); return; }
    if (code === "invalid_port_mapping_port") { setError(t("portMappingInvalidPort")); return; }
    // Subdomain failures keep the draft inline so the user can pick a new value.
    if (code === "subdomain_conflict") { setError(t("portMappingSubdomainConflict")); return; }
    if (code === "invalid_port_mapping_subdomain") { setError(t("portMappingSubdomainInvalid")); return; }
    const message = failure instanceof Error ? failure.message : t("portMappingRequestFailed");
    setError(message);
    onError?.(message);
  }, [onError, t]);

  const load = useCallback(async () => {
    // Polling and a user-triggered refresh should never queue an unbounded set
    // of requests while the control plane is slow.
    if (loadingRef.current) return;
    loadingRef.current = true;
    const revision = mutationRevision.current;
    setLoading(true);
    try {
      const fresh = await api.portMappings(workspaceId);
      // A save or delete that landed while this fetch was in flight is newer;
      // the next poll reconciles, so a stale snapshot must not overwrite it.
      if (mutationRevision.current === revision) setItems(fresh);
    }
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

  function showValidation(messageKey: MessageKey) {
    setError(t(messageKey));

  }

  async function save(event: FormEvent) {
    event.preventDefault();
    const internalPort = parseInternalPort(draft.port);
    if (internalPort === null) return showValidation("portMappingInvalidPort");
    if (nameValidation.tooLong) return showValidation("portMappingNameTooLong");
    if (!subdomainValidation.valid) return showValidation("portMappingSubdomainInvalid");
    const duplicate = items.some((item) => item.internal_port === internalPort && item.id !== draft.id);
    if (duplicate) return showValidation(editing ? "portMappingConflictEditOther" : "portMappingConflictUseEdit");
    if (!editing && !workspaceReady) return showValidation("portMappingAddNeedsReady");
    mutationRevision.current += 1;
    setSaving(true); setError(null);
    try {
      const displayName = nameValidation.value ? nameValidation.value : null;
      if (editing) {
        const updated = await api.updatePortMapping(workspaceId, draft.id as string, {
          internal_port: internalPort,
          display_name: displayName,
          // A blank field keeps the current label; the prepopulated value is
          // sent back unchanged unless the user actually edits it.
          ...(subdomainValidation.value ? { subdomain: subdomainValidation.value } : {}),
        });
        setItems((current) => current.map((item) => item.id === updated.id ? updated : item));
      } else {
        const created = await api.createPortMapping(workspaceId, {
          internal_port: internalPort,
          ...(displayName ? { display_name: displayName } : {}),
          // Omitted on purpose when blank: the backend assigns the short
          // automatic app-<8hex> label.
          ...(subdomainValidation.value ? { subdomain: subdomainValidation.value } : {}),
        });
        setItems((current) => [...current.filter((item) => item.id !== created.id), created]);
      }
      setDraft(EMPTY_DRAFT);
    } catch (errorValue) {
      // Keep the user's draft untouched so a failed save never loses typing.
      report(errorValue);
    }
    finally { mutationRevision.current += 1; setSaving(false); }
  }

  function startEdit(item: PortMapping) {
    setError(null);
    // Prepopulate the persisted label so users see exactly what will change.
    setDraft({ id: item.id, port: String(item.internal_port), displayName: item.display_name ?? "", subdomain: item.subdomain });
  }

  async function remove() {
    const item = pendingDelete;
    if (!item) return;
    mutationRevision.current += 1;
    setDeleting(item.id); setError(null);
    try {
      await api.deletePortMapping(workspaceId, item.id);
      setItems((current) => current.filter((candidate) => candidate.id !== item.id));
      if (draft.id === item.id) setDraft(EMPTY_DRAFT);
      setPendingDelete(null);
    } catch (errorValue) { report(errorValue); }
    finally { mutationRevision.current += 1; setDeleting(null); }
  }

  useEffect(() => {
    if (!dialogOpen || !items.some((item) => item.status === "provisioning")) return;
    const timer = window.setInterval(() => void load(), 2_000);
    return () => window.clearInterval(timer);
  }, [dialogOpen, items, load]);

  return <>
    <Button type="button" appearance="outline" icon={<AddRegular />} onClick={openDialog}>{t("portMappings")}</Button>
    <Dialog open={dialogOpen} onOpenChange={(_, data) => { if (!busy || data.open) setDialogOpen(data.open); }}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody className={styles.dialogBody}>
          <DialogTitle action={<div className={styles.toolbarGroup}><Button appearance="subtle" icon={<ArrowSyncRegular />} disabled={loading || busy} onClick={() => void load()}>{loading ? t("portMappingRefreshing") : t("portMappingRefresh")}</Button><Button appearance="subtle" icon={<DismissRegular />} aria-label={t("close")} disabled={busy} onClick={() => setDialogOpen(false)} /></div>}>{t("portMappings")}</DialogTitle>
          <DialogContent className={styles.dialogBody}>
            <Text>{t("portMappingIntro")}</Text>
            <form className={styles.formGrid} onSubmit={(event) => void save(event)}>
              <Field label={t("internalPort")} required hint={t("portMappingPortHelp")}><Input aria-label={t("internalPort")} disabled={(!editing && !workspaceReady) || busy} type="number" min={1} max={65535} step={1} required value={draft.port} onChange={(_, data) => setDraft((current) => ({ ...current, port: data.value }))} placeholder={t("portMappingPortPlaceholder")} /></Field>
              <Field label={t("displayNameOptional")} hint={<span aria-live="polite">{t("portMappingNameCounter", { count: nameValidation.length, max: DISPLAY_NAME_MAX_CODE_POINTS })}</span>} validationState={nameValidation.tooLong ? "error" : undefined} validationMessage={nameValidation.tooLong ? t("portMappingNameTooLong") : undefined}>
                <Input disabled={(!editing && !workspaceReady) || busy} value={draft.displayName} onChange={(_, data) => setDraft((current) => ({ ...current, displayName: data.value }))} placeholder={t("portMappingNamePlaceholder")} />
              </Field>
              <Field className={styles.formWide} label={t("portMappingSubdomainOptional")} hint={`${t("portMappingSubdomainHelp")} ${editing ? t("portMappingSubdomainChangeHelp") : t("portMappingSubdomainAutoHelp")}`} validationState={subdomainValidation.valid ? undefined : "error"} validationMessage={subdomainValidation.valid ? undefined : t("portMappingSubdomainInvalid")}>
                <Input disabled={(!editing && !workspaceReady) || busy} value={draft.subdomain} onChange={(_, data) => setDraft((current) => ({ ...current, subdomain: data.value }))} placeholder={t("portMappingSubdomainPlaceholder")} />
              </Field>
              {workspaceStopped && <Caption1 className={styles.formWide} role="status">{t("portMappingStoppedHelp")}</Caption1>}
              {!workspaceReady && !workspaceStopped && <Caption1 className={styles.formWide} role="status">{t("portMappingWorkspaceNotReady")}</Caption1>}
              <div className={`${styles.formActions} ${styles.formWide}`}>
                <Button type="submit" appearance="primary" icon={editing ? undefined : <AddRegular />} disabled={(!editing && !workspaceReady) || busy || nameValidation.tooLong || !subdomainValidation.valid}>{saving ? (editing ? t("saving") : t("portMappingAdding")) : (editing ? t("saveMapping") : t("addMapping"))}</Button>
                {editing && <Button type="button" appearance="secondary" disabled={busy} onClick={() => { setDraft(EMPTY_DRAFT); setError(null); }}>{t("cancel")}</Button>}
              </div>
            </form>
            {error && <Text role="alert">{error}</Text>}
            <section aria-label={t("portMappingAgentHintTitle")}>
              <Caption1 block>{t("portMappingAgentHintIntro")}</Caption1>
              <Caption1 block><code>MWC_PORT_MAPPINGS_FILE</code>{t("portMappingAgentHintFile")}</Caption1>
              <Caption1 block><code>MWC_PORT_MAPPINGS_JSON</code>{t("portMappingAgentHintJson")}</Caption1>
              <Caption1 block>{t("portMappingAgentHintStale")}</Caption1>
              <Caption1 block>{t("portMappingAgentHintAuth")}</Caption1>
              <AgentHintCommand />
            </section>
            <section className={styles.portList} aria-live="polite">
              {loading && items.length === 0 && <Text>{t("portMappingsLoading")}</Text>}
              {!loading && items.length === 0 && !error && <Text>{t("noPortMappings")}</Text>}
              {items.map((item) => <PortMappingRow key={item.id} api={api} workspaceId={workspaceId} workspaceReady={workspaceReady} item={item} busy={busy} deleting={deleting === item.id} editing={draft.id === item.id} onEdit={() => startEdit(item)} onDelete={() => setPendingDelete(item)} onError={report} />)}
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

/** Copyable one-liner agents can run inside the workspace to read the live mappings file. */
function AgentHintCommand() {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const [copied, setCopied] = useState(false);
  const command = 'cat "$MWC_PORT_MAPPINGS_FILE"';
  async function copy() {
    try { await navigator.clipboard.writeText(command); setCopied(true); window.setTimeout(() => setCopied(false), 1_200); }
    catch { /* Clipboard permissions are optional; the command remains selectable. */ }
  }
  return <div className={styles.toolbarGroup}><code className={styles.code}>{command}</code><Button type="button" appearance="subtle" size="small" icon={<CopyRegular />} onClick={() => void copy()}>{copied ? t("copied") : t("portMappingCopyCommand")}</Button></div>;
}

function PortMappingRow({ api, workspaceId, workspaceReady, item, busy, deleting, editing, onEdit, onDelete, onError }: { api: PortMappingsApi; workspaceId: string; workspaceReady: boolean; item: PortMapping; busy: boolean; deleting: boolean; editing: boolean; onEdit: () => void; onDelete: () => void; onError: (error: unknown) => void }) {
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
  return <article className={styles.portRow} aria-current={editing ? "true" : undefined}>
    <div className={styles.portMain}><Text weight="semibold">{item.display_name || `${t("portLabel")} ${item.internal_port}`}</Text><Caption1>{t("internalPort")} <span className={styles.code}>{item.internal_port}</span></Caption1>{url ? <Text className={styles.mappingUrl}>{url}</Text> : <Caption1>{t("portMappingAddressPending")}</Caption1>}</div>
    <div className={styles.portActions}><Badge appearance="tint" color={item.status === "ready" ? "success" : item.status === "failed" ? "danger" : "informative"}>{statusLabel(item.status, t)}</Badge><Button type="button" appearance="secondary" icon={<OpenRegular />} disabled={!workspaceReady || !url || opening || item.status !== "ready"} onClick={() => void open()}>{opening ? t("portMappingOpening") : t("open")}</Button><Button type="button" appearance="subtle" icon={<CopyRegular />} disabled={!url} onClick={() => void copy()}>{copied ? t("copied") : t("copyLink")}</Button>{!item.managed && <Button type="button" appearance="subtle" icon={<EditRegular />} disabled={busy || editing} onClick={onEdit}>{editing ? t("editing") : t("edit")}</Button>}{!item.managed && <Button type="button" appearance="subtle" icon={<DeleteRegular />} className={styles.dangerButton} disabled={deleting || busy} onClick={onDelete}>{deleting ? t("deleting") : t("delete")}</Button>}</div>
  </article>;
}

function statusLabel(status: string, t: (key: MessageKey) => string): string {
  if (status === "ready") return t("portMappingReady");
  if (status === "stopped") return t("stateStopped");
  if (status === "failed") return t("portMappingFailed");
  if (status === "deleting") return t("deleting");
  return t("portMappingProvisioning");
}
