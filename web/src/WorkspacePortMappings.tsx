import { useCallback, useEffect, useId, useRef, useState } from "react";
import type { FormEvent } from "react";
import { useI18n } from "./i18n";
import type { MessageKey } from "./i18n";
import { reserveWebShellWindow } from "./workspaceShell";
import {
  mappingUrl,
  parseInternalPort,
  type PortMapping,
  type PortMappingsApi,
} from "./portMappings";

interface Props {
  api: PortMappingsApi;
  workspaceId: string;
  workspaceReady: boolean;
  onError?: (message: string) => void;
}

/** Port forwarding controls for a workspace, including stopped workspaces. */
export function WorkspacePortMappings({ api, workspaceId, workspaceReady, onError }: Props) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const loadingRef = useRef(false);
  const [items, setItems] = useState<PortMapping[]>([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [port, setPort] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [dialogOpen, setDialogOpen] = useState(false);

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
    if (dialog.current?.open) return;
    dialog.current?.showModal();
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

  async function remove(item: PortMapping) {
    if (!window.confirm(t("portMappingDeleteConfirm"))) return;
    setDeleting(item.id); setError(null);
    try {
      await api.deletePortMapping(workspaceId, item.id);
      setItems((current) => current.filter((candidate) => candidate.id !== item.id));
    } catch (errorValue) { report(errorValue); }
    finally { setDeleting(null); }
  }

  useEffect(() => {
    if (!dialogOpen || !items.some((item) => item.status === "provisioning")) return;
    const timer = window.setInterval(() => void load(), 2_000);
    return () => window.clearInterval(timer);
  }, [dialogOpen, items, load]);

  return <>
    <button type="button" className="port-mappings-trigger" onClick={openDialog}>{t("portMappings")}</button>
    <dialog ref={dialog} className="port-mappings-dialog" aria-labelledby={titleId} onClose={() => setDialogOpen(false)} onClick={(event) => {
      if (event.target === dialog.current) dialog.current?.close();
    }}>
      <div className="port-mappings-dialog-content">
        <header><div><span className="eyebrow">{t("portMappings")}</span><h3 id={titleId}>{t("portMappings")}</h3></div><div className="port-mappings-header-actions"><button type="button" className="port-mappings-refresh" disabled={loading} onClick={() => void load()}>{loading ? t("portMappingRefreshing") : t("portMappingRefresh")}</button><button type="button" className="port-mappings-close" aria-label={t("close")} onClick={() => dialog.current?.close()}>×</button></div></header>
        <p className="port-mappings-intro">{t("portMappingIntro")}</p>
        <form className="port-mappings-form" onSubmit={(event) => void add(event)}>
          <label>{t("internalPort")}<input disabled={!workspaceReady || saving} type="number" min={1} max={65535} step={1} required value={port} onChange={(event) => setPort(event.target.value)} placeholder={t("portMappingPortPlaceholder")} /></label>
          <label>{t("displayNameOptional")}<input disabled={!workspaceReady || saving} value={displayName} maxLength={80} onChange={(event) => setDisplayName(event.target.value)} placeholder={t("portMappingNamePlaceholder")} /></label>
          <small>{t("portMappingPortHelp")}</small>
          {!workspaceReady && <small className="port-mappings-not-ready" role="status">{t("portMappingWorkspaceNotReady")}</small>}
          <button type="submit" className="button primary" disabled={!workspaceReady || saving}>{saving ? t("portMappingAdding") : t("addMapping")}</button>
        </form>
        {error && <p className="port-mappings-error" role="alert">{error}</p>}
        <section className="port-mappings-list" aria-live="polite">
          {loading && <p>{t("portMappingsLoading")}</p>}
          {!loading && !error && items.length === 0 && <p>{t("noPortMappings")}</p>}
          {!loading && items.map((item) => <PortMappingRow key={item.id} api={api} workspaceId={workspaceId} workspaceReady={workspaceReady} item={item} deleting={deleting === item.id} onDelete={() => void remove(item)} onError={report} />)}
        </section>
      </div>
    </dialog>
  </>;
}

function PortMappingRow({ api, workspaceId, workspaceReady, item, deleting, onDelete, onError }: { api: PortMappingsApi; workspaceId: string; workspaceReady: boolean; item: PortMapping; deleting: boolean; onDelete: () => void; onError: (error: unknown) => void }) {
  const { t } = useI18n();
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
    const target = reserveWebShellWindow();
    setOpening(true);
    try {
      const bootstrap = await api.bootstrapPortMapping(workspaceId, item.id);
      if (target) target.location.href = bootstrap.bootstrap_url;
      else window.location.href = bootstrap.bootstrap_url;
    } catch (errorValue) { target?.close(); onError(errorValue); }
    finally { setOpening(false); }
  }
  return <article className="port-mapping-row">
    <div><strong>{item.display_name || `${t("portLabel")} ${item.internal_port}`}</strong><span>{t("internalPort")} <code>{item.internal_port}</code></span></div>
    <span className={`port-mapping-status ${item.status}`}>{statusLabel(item.status, t)}</span>
    {url ? <code className="port-mapping-url" tabIndex={0}>{url}</code> : <span className="port-mapping-unavailable">{t("portMappingAddressPending")}</span>}
    <div className="port-mapping-actions"><button type="button" disabled={!workspaceReady || !url || opening || item.status !== "ready"} onClick={() => void open()}>{opening ? t("portMappingOpening") : t("open")}</button><button type="button" disabled={!url} onClick={() => void copy()}>{copied ? t("copied") : t("copyLink")}</button><button type="button" className="danger" disabled={deleting} onClick={onDelete}>{deleting ? t("deleting") : t("delete")}</button></div>
  </article>;
}

function statusLabel(status: string, t: (key: MessageKey) => string): string {
  if (status === "ready") return t("portMappingReady");
  if (status === "failed") return t("portMappingFailed");
  if (status === "deleting") return t("deleting");
  return t("portMappingProvisioning");
}
