import { useEffect, useRef, useState } from "react";
import { Button, Card, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Text, makeStyles, tokens } from "@fluentui/react-components";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import { currentPluginTheme, parsePluginApiBridgeRequest, parsePluginBridgeRequest, safePluginSessionPath } from "./surfaceBridge";
import type { PluginManifest, PluginSurface, PluginSurfaceSession } from "./types";

const useStyles = makeStyles({
  section: { display: "grid", gap: tokens.spacingVerticalM },
  title: { margin: 0 },
  list: { display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(190px, 1fr))", gap: tokens.spacingHorizontalM },
  button: { display: "grid", justifyItems: "start", gap: tokens.spacingVerticalXXS, height: "auto", padding: tokens.spacingVerticalM, textAlign: "left" },
  secondary: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  frame: { display: "block", width: "min(1100px, 90vw)", height: "min(820px, 82vh)", border: 0, backgroundColor: tokens.colorNeutralBackground1 },
});

export function PluginSurfaceHost({ api, plugins, placement, organizationId = null }: { api: PluginApi; plugins: PluginManifest[]; placement: PluginSurface["placement"]; organizationId?: string | null }) {
  const styles = useStyles();
  const { t } = useI18n();
  const available = plugins.flatMap((plugin) => plugin.enabled && plugin.runtime_status === "loaded" && plugin.approved_contributions.includes("ui_surfaces")
    ? plugin.ui_surfaces.filter((surface) => surface.placement === placement).map((surface) => ({ plugin, surface }))
    : []);
  const [selected, setSelected] = useState<{ plugin: PluginManifest; surface: PluginSurface } | null>(null);
  if (!available.length) return null;
  return <section className={styles.section} aria-labelledby={`plugin-surfaces-${placement}`}><Text as="h3" size={400} weight="semibold" className={styles.title} id={`plugin-surfaces-${placement}`}>{t("pluginPages")}</Text><div className={styles.list}>{available.map(({ plugin, surface }) => <Button key={`${plugin.id}:${surface.id}`} className={styles.button} appearance="secondary" onClick={() => setSelected({ plugin, surface })}><Text weight="semibold">{surface.title}</Text><Text className={styles.secondary}>{plugin.name}</Text></Button>)}</div>{selected && <PluginSurfaceDialog api={api} plugin={selected.plugin} surface={selected.surface} organizationId={organizationId} onClose={() => setSelected(null)} />}</section>;
}

function PluginSurfaceDialog({ api, plugin, surface, organizationId, onClose }: { api: PluginApi; plugin: PluginManifest; surface: PluginSurface; organizationId: string | null; onClose: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  const frame = useRef<HTMLIFrameElement>(null);
  const port = useRef<MessagePort | null>(null);
  const [session, setSession] = useState<PluginSurfaceSession | null>(null);
  const [launchPath, setLaunchPath] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    void api.createSurfaceSession(plugin.id, surface.id).then((value) => {
      const path = safePluginSessionPath(value.launch_url, window.location.origin);
      if (!path) throw new Error(t("pluginSurfaceInvalid"));
      setSession(value); setLaunchPath(path);
    }).catch((reason) => setError(reason instanceof Error ? reason.message : t("pluginRequestFailed")));
    return () => port.current?.close();
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
        const pluginRequest = plugin.approved_contributions.includes("api_routes") ? parsePluginApiBridgeRequest(request.payload, plugin.api_routes) : null;
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

  return <Dialog open onOpenChange={(_, data) => !data.open && onClose()}><DialogSurface><DialogBody><DialogTitle action={<Button appearance="subtle" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</Button>}>{surface.title}</DialogTitle><DialogContent>{error ? <Text role="alert">{error}</Text> : launchPath ? <iframe ref={frame} className={styles.frame} src={launchPath} title={`${plugin.name}: ${surface.title}`} sandbox="allow-forms allow-scripts" referrerPolicy="no-referrer" onLoad={connect} /> : <Text role="status">{t("pluginsLoading")}</Text>}</DialogContent></DialogBody></DialogSurface></Dialog>;
}
