import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import { currentPluginTheme, parsePluginApiBridgeRequest, parsePluginBridgeRequest, safePluginSessionPath } from "./surfaceBridge";
import type { PluginManifest, PluginSurface, PluginSurfaceSession } from "./types";

export function PluginSurfaceHost({ api, plugins, placement, organizationId = null }: {
  api: PluginApi;
  plugins: PluginManifest[];
  placement: PluginSurface["placement"];
  organizationId?: string | null;
}) {
  const { t } = useI18n();
  const available = plugins.flatMap((plugin) => plugin.enabled && plugin.runtime_status === "loaded" && plugin.approved_contributions.includes("ui_surfaces")
    ? plugin.ui_surfaces.filter((surface) => surface.placement === placement).map((surface) => ({ plugin, surface }))
    : []);
  const [selected, setSelected] = useState<{ plugin: PluginManifest; surface: PluginSurface } | null>(null);
  if (!available.length) return null;
  return <section className="plugin-surfaces" aria-labelledby={`plugin-surfaces-${placement}`}><div className="card-heading"><h3 id={`plugin-surfaces-${placement}`}>{t("pluginPages")}</h3></div><div className="plugin-surface-list">{available.map(({ plugin, surface }) => <button type="button" key={`${plugin.id}:${surface.id}`} onClick={() => setSelected({ plugin, surface })}><strong>{surface.title}</strong><small>{plugin.name}</small></button>)}</div>{selected && <PluginSurfaceDialog api={api} plugin={selected.plugin} surface={selected.surface} organizationId={organizationId} onClose={() => setSelected(null)} />}</section>;
}

function PluginSurfaceDialog({ api, plugin, surface, organizationId, onClose }: {
  api: PluginApi;
  plugin: PluginManifest;
  surface: PluginSurface;
  organizationId: string | null;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const port = useRef<MessagePort | null>(null);
  const [session, setSession] = useState<PluginSurfaceSession | null>(null);
  const [launchPath, setLaunchPath] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    dialog.current?.showModal();
    void api.createSurfaceSession(plugin.id, surface.id).then((value) => {
      const path = safePluginSessionPath(value.launch_url, window.location.origin);
      if (!path) throw new Error(t("pluginSurfaceInvalid"));
      setSession(value); setLaunchPath(path);
    }).catch((reason) => setError(reason instanceof Error ? reason.message : t("pluginRequestFailed")));
    return () => { port.current?.close(); dialog.current?.close(); };
  }, [api, plugin.id, surface.id, t]);

  function connect() {
    if (!session || !frame.current?.contentWindow) return;
    const channel = new MessageChannel();
    port.current?.close(); port.current = channel.port1;
    const allowed = session.allowed_bridge_methods;
    channel.port1.onmessage = async (event) => {
      const request = parsePluginBridgeRequest(event.data, session.channel_nonce, allowed);
      if (!request) return;
      try {
        if (request.method === "theme.read") {
          channel.port1.postMessage({ request_id: request.request_id, result: { theme: currentPluginTheme(document.documentElement) } });
          return;
        }
        const pluginRequest = plugin.approved_contributions.includes("api_routes")
          ? parsePluginApiBridgeRequest(request.payload, plugin.api_routes)
          : null;
        if (!pluginRequest) {
          channel.port1.postMessage({ request_id: request.request_id, error: { code: "invalid_plugin_api_request", message: t("pluginSurfaceActionUnavailable") } });
          return;
        }
        channel.port1.postMessage({ request_id: request.request_id, result: await api.invokePluginRoute(plugin.id, pluginRequest, organizationId) });
      } catch (reason) {
        channel.port1.postMessage({ request_id: request.request_id, error: { code: "bridge_failed", message: reason instanceof Error ? reason.message : t("pluginRequestFailed") } });
      }
    };
    frame.current.contentWindow.postMessage({ type: "mwc:connect", nonce: session.channel_nonce, methods: allowed.filter((method) => method === "theme.read" || method === "plugin_api.request") }, "*", [channel.port2]);
  }

  return <dialog ref={dialog} className="plugin-surface-dialog" aria-labelledby="plugin-surface-title" onCancel={onClose} onClose={onClose}><div className="plugin-dialog-heading"><div><p className="eyebrow">{plugin.name}</p><h2 id="plugin-surface-title">{surface.title}</h2></div><button type="button" className="dialog-close" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</button></div>{error ? <div className="error-banner" role="alert">{error}</div> : launchPath ? <iframe ref={frame} src={launchPath} title={`${plugin.name}: ${surface.title}`} sandbox="allow-forms allow-scripts" referrerPolicy="no-referrer" onLoad={connect} /> : <div className="empty" role="status">{t("pluginsLoading")}</div>}</dialog>;
}
