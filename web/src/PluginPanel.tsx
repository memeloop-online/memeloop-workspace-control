import Form from "@rjsf/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "./i18n";
import { PluginApi } from "./plugins/api";
import { checkPluginSchema, configurationKey } from "./plugins/schema";
import { safeValidator } from "./plugins/safeValidator";
import { nextConfigurationScope, pluginCatalogState, pluginErrorMessageKey } from "./plugins/viewModel";
import type {
  PluginConfiguration,
  PluginConfigurationScope,
  PluginManifest,
} from "./plugins/types";

export function PluginPanel({
  token,
  organizationId,
  systemAdmin,
  onOpenCredentials,
}: {
  token: string;
  organizationId: string;
  systemAdmin: boolean;
  onOpenCredentials: () => void;
}) {
  const { t } = useI18n();
  const api = useMemo(() => new PluginApi(token), [token]);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [selected, setSelected] = useState<PluginManifest | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const catalogState = pluginCatalogState(loading, error, plugins.length);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setPlugins(await api.manifests());
      setError("");
    } catch (reason) {
      setError(messageOf(reason, t("pluginRequestFailed")));
    } finally {
      setLoading(false);
    }
  }, [api, t]);

  useEffect(() => { void load(); }, [load]);

  return <section className="panel-stack plugin-page" aria-labelledby="plugin-page-title">
    <div className="section-heading">
      <div><p className="eyebrow">WASM COMPONENTS</p><h2 id="plugin-page-title">{t("pluginsTitle")}</h2></div>
      <span className="read-only-badge">{t("pluginsReadOnlyCatalog")}</span>
    </div>
    <div className="security-note plugin-notice" role="note">
      <strong>{t("pluginsRestartNotice")}</strong>
      <span>{t("pluginsRestartDetail")}</span>
    </div>
    {catalogState === "error" && <div className="error-banner" role="alert">{error}<button type="button" onClick={() => void load()}>{t("pluginRetry")}</button></div>}
    {catalogState === "loading" ? <div className="empty" role="status">{t("pluginsLoading")}</div> : catalogState === "empty" ? (
      <div className="empty plugin-empty"><strong>{t("pluginsEmpty")}</strong><span>{t("pluginsEmptyHint")}</span></div>
    ) : <div className="plugin-grid">{plugins.map((plugin) => <PluginCard key={plugin.id} plugin={plugin} onConfigure={() => setSelected(plugin)} />)}</div>}
    {selected && <ConfigurationDialog
      api={api}
      plugin={selected}
      organizationId={organizationId}
      systemAdmin={systemAdmin}
      onClose={() => setSelected(null)}
      onOpenCredentials={onOpenCredentials}
    />}
  </section>;
}

function PluginCard({ plugin, onConfigure }: { plugin: PluginManifest; onConfigure: () => void }) {
  const { t } = useI18n();
  const configurable = Boolean(plugin.configuration_schema);
  return <article className={`plugin-card ${plugin.loaded ? "" : "failed"}`}>
    <header>
      <div><h3>{plugin.name || plugin.id}</h3><code>{plugin.id}</code></div>
      <span className={`plugin-load-state ${plugin.loaded ? "loaded" : "failed"}`}>{plugin.loaded ? t("pluginLoaded") : t("pluginLoadFailed")}</span>
    </header>
    {plugin.description && <p>{plugin.description}</p>}
    <dl className="plugin-facts">
      <div><dt>{t("pluginVersion")}</dt><dd>{plugin.version}</dd></div>
      <div><dt>{t("pluginInterfaceVersion")}</dt><dd>{plugin.wit_version}</dd></div>
      <div><dt>{t("pluginSource")}</dt><dd><code>{plugin.source || t("pluginSourceUnknown")}</code></dd></div>
      <div><dt>{t("pluginContributions")}</dt><dd>{plugin.workspace_create_policy ? t("pluginWorkspaceAdmission") : t("pluginNoContributions")}</dd></div>
    </dl>
    <div className="plugin-capabilities" aria-label={t("pluginApprovedContributions")}>
      {plugin.approved_contributions.length ? plugin.approved_contributions.map((contribution) => <span key={contribution}>{contribution}</span>) : <span>{t("pluginNoContributions")}</span>}
    </div>
    <small className="plugin-no-host-capabilities">{t("pluginNoCapabilities")}</small>
    {plugin.denial_codes.length > 0 && <div className="plugin-denial-codes"><strong>{t("pluginDenialCodes")}</strong>{plugin.denial_codes.map((code) => <code key={code}>{code}</code>)}</div>}
    {!plugin.loaded && <div className="plugin-failure" role="status"><code>{plugin.error_code || "plugin_load_failed"}</code>{plugin.error_message && <span>{plugin.error_message}</span>}</div>}
    <footer>
      <button className="button" type="button" disabled={!plugin.loaded || !configurable} onClick={onConfigure}>
        {configurable ? t("pluginConfigure") : t("pluginNoConfiguration")}
      </button>
    </footer>
  </article>;
}

function ConfigurationDialog({ api, plugin, organizationId, systemAdmin, onClose, onOpenCredentials }: {
  api: PluginApi;
  plugin: PluginManifest;
  organizationId: string;
  systemAdmin: boolean;
  onClose: () => void;
  onOpenCredentials: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const [scope, setScope] = useState<PluginConfigurationScope>(systemAdmin ? "installation" : "organization");
  const [configurations, setConfigurations] = useState<Record<string, PluginConfiguration>>({});
  const [formData, setFormData] = useState<unknown>(plugin.configuration_default ?? {});
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const schema = useMemo(() => checkPluginSchema(plugin.configuration_schema), [plugin.configuration_schema]);
  const current = configurations[configurationKey(plugin.id, scope)];

  useEffect(() => {
    dialog.current?.showModal();
    return () => dialog.current?.close();
  }, []);

  const load = useCallback(async () => {
    setBusy(true);
    try {
      const organization = await api.configuration(plugin.id, organizationId);
      const values: Record<string, PluginConfiguration> = { [configurationKey(plugin.id, "organization")]: organization };
      if (systemAdmin) {
        const global = await api.configuration(plugin.id, null);
        Object.assign(values, { [configurationKey(plugin.id, "installation")]: global });
      }
      setConfigurations(values);
      setError("");
    } catch (reason) {
      setError(messageOf(reason, t("pluginRequestFailed")));
    } finally {
      setBusy(false);
    }
  }, [api, organizationId, plugin.id, systemAdmin, t]);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    if (current) setFormData(current.value);
  }, [current, scope]);

  async function save(value: unknown) {
    if (!current) return;
    setBusy(true); setError(""); setMessage("");
    try {
      const saved = await api.putConfiguration(
        plugin.id,
        scope === "organization" ? organizationId : null,
        { expected_version: current.scope_version, value },
      );
      setConfigurations((values) => ({ ...values, [configurationKey(plugin.id, scope)]: saved }));
      setFormData(saved.value);
      setMessage(t("pluginConfigurationSaved"));
    } catch (reason) {
      setError(pluginErrorMessage(reason, t));
    } finally {
      setBusy(false);
    }
  }

  async function removeOverride() {
    if (!current || current.scope_version === 0 || !confirm(t("pluginDeleteOverrideConfirm"))) return;
    setBusy(true); setError(""); setMessage("");
    try {
      const inherited = await api.deleteConfiguration(
        plugin.id,
        scope === "organization" ? organizationId : null,
        current.scope_version,
      );
      setConfigurations((values) => ({ ...values, [configurationKey(plugin.id, scope)]: inherited }));
      setFormData(inherited.value);
      setMessage(t("pluginOverrideDeleted"));
    } catch (reason) {
      setError(pluginErrorMessage(reason, t));
    } finally {
      setBusy(false);
    }
  }

  return <dialog ref={dialog} className="plugin-dialog" aria-labelledby="plugin-dialog-title" onCancel={onClose} onClose={onClose}>
    <div className="plugin-dialog-heading"><div><p className="eyebrow">{plugin.id}</p><h2 id="plugin-dialog-title">{t("pluginConfigurationTitle")}</h2></div><button type="button" className="dialog-close" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</button></div>
    <div className="scope-tabs" role="tablist" aria-label={t("pluginConfigurationScope")} onKeyDown={(event) => {
      const next = nextConfigurationScope(scope, event.key, systemAdmin);
      if (next !== scope) { event.preventDefault(); setScope(next); }
    }}>
      {systemAdmin && <button type="button" role="tab" aria-selected={scope === "installation"} className={scope === "installation" ? "active" : ""} onClick={() => setScope("installation")}>{t("pluginScopeGlobal")}</button>}
      <button type="button" role="tab" aria-selected={scope === "organization"} className={scope === "organization" ? "active" : ""} onClick={() => setScope("organization")}>{t("pluginScopeOrganization")}</button>
    </div>
    <p className="security-note credential-guidance">{t("pluginSensitiveGuidance")} <button type="button" className="text-button" onClick={() => { onClose(); onOpenCredentials(); }}>{t("pluginOpenCredentials")}</button></p>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {message && <div className="success-banner" role="status">{message}</div>}
    {busy && !current ? <div className="empty compact" role="status">{t("pluginsLoading")}</div> : current && <>
      {current.schema_changed && <div className="error-banner" role="alert">{t("pluginSchemaChanged")}</div>}
      {!current.valid && <div className="error-banner" role="alert">{t("pluginConfigurationInvalid")}</div>}
      <dl className="configuration-status">
        <div><dt>{t("pluginEffectiveSource")}</dt><dd>{sourceLabel(current.source, t)}</dd></div>
        <div><dt>{t("pluginScopeVersion")}</dt><dd>v{current.scope_version}</dd></div>
        <div><dt>{t("pluginEffectiveVersion")}</dt><dd>v{current.effective_version}</dd></div>
      </dl>
      {!schema.ok ? <div className="error-banner" role="alert">{schema.reason === "sensitive" ? t("pluginSensitiveSchemaRejected") : t("pluginSchemaRejected")}</div> : (
        <Form
          key={`${plugin.id}-${scope}-${current.scope_version}`}
          schema={schema.schema}
          formData={formData}
          validator={safeValidator}
          noHtml5Validate
          disabled={busy}
          onChange={({ formData: value }) => setFormData(value)}
          transformErrors={(errors) => errors.map((entry) => ({ ...entry, message: t("pluginValidationInvalid"), stack: `${entry.property} ${t("pluginValidationInvalid")}` }))}
          onSubmit={({ formData: value }) => void save(value)}
        >
          <div className="plugin-dialog-actions">
            <button className="button primary" type="submit" disabled={busy}>{busy ? t("pluginSaving") : t("pluginSaveConfiguration")}</button>
            <button className="button danger" type="button" disabled={busy || current.scope_version === 0} onClick={() => void removeOverride()}>{t("pluginDeleteOverride")}</button>
          </div>
        </Form>
      )}
    </>}
  </dialog>;
}

function sourceLabel(source: PluginConfiguration["source"], t: ReturnType<typeof useI18n>["t"]): string {
  if (source === "organization") return t("pluginSourceOrganization");
  if (source === "installation") return t("pluginSourceGlobal");
  return t("pluginSourceDefault");
}

function pluginErrorMessage(reason: unknown, t: ReturnType<typeof useI18n>["t"]): string {
  const code = errorCode(reason);
  if (code) return t(pluginErrorMessageKey(code));
  return messageOf(reason, t("pluginRequestFailed"));
}

function errorCode(reason: unknown): string | undefined {
  return reason instanceof Error && "code" in reason ? String((reason as Error & { code?: string }).code) : undefined;
}

function messageOf(reason: unknown, fallback: string): string {
  return reason instanceof Error ? reason.message : fallback;
}

// Keep the public component independent of the application's shared API types.
export type { PluginManifest, PluginConfiguration };
