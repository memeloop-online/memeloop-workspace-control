import Form from "@rjsf/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "./i18n";
import { PluginApi } from "./plugins/api";
import { PluginAuthorizationDialog } from "./plugins/PluginAuthorizationDialog";
import { PluginInstaller } from "./plugins/PluginInstaller";
import { PluginSurfaceHost } from "./plugins/PluginSurfaceHost";
import { checkPluginSchema, configurationKey } from "./plugins/schema";
import { safeValidator } from "./plugins/safeValidator";
import { nextConfigurationScope, pluginCatalogState, pluginErrorMessageKey, pluginSourceSummary } from "./plugins/viewModel";
import type {
  PluginConfiguration,
  PluginConfigurationScope,
  PluginInspection,
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
  const [installerTarget, setInstallerTarget] = useState<PluginManifest | null | undefined>(undefined);
  const [inspection, setInspection] = useState<PluginInspection | null>(null);
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

  async function setEnabled(plugin: PluginManifest) {
    try {
      const updated = await api.setEnabled(plugin.id, !plugin.enabled, plugin.package_version);
      setPlugins((values) => values.map((value) => value.id === updated.id ? updated : value));
      setError("");
    } catch (reason) { setError(pluginErrorMessage(reason, t)); }
  }

  async function uninstall(plugin: PluginManifest) {
    if (!confirm(t("pluginUninstallConfirm"))) return;
    try {
      await api.uninstall(plugin.id, plugin.package_version);
      setPlugins((values) => values.filter((value) => value.id !== plugin.id));
      setError("");
    } catch (reason) { setError(pluginErrorMessage(reason, t)); }
  }

  function installed(plugin: PluginManifest) {
    setPlugins((values) => values.some((value) => value.id === plugin.id) ? values.map((value) => value.id === plugin.id ? plugin : value) : [...values, plugin]);
    setInspection(null);
    setInstallerTarget(undefined);
  }

  return <section className="panel-stack plugin-page" aria-labelledby="plugin-page-title">
    <div className="section-heading"><div><p className="eyebrow">PLUGINS</p><h2 id="plugin-page-title">{t("pluginsTitle")}</h2><p className="section-copy">{t("pluginsDescription")}</p></div>{systemAdmin && <button className="button primary" type="button" onClick={() => setInstallerTarget(null)}>{t("pluginInstall")}</button>}</div>
    {catalogState === "error" && <div className="error-banner" role="alert">{error}<button type="button" onClick={() => void load()}>{t("pluginRetry")}</button></div>}
    {catalogState === "loading" ? <div className="empty" role="status">{t("pluginsLoading")}</div> : catalogState === "empty" ? (
      <div className="empty plugin-empty"><strong>{t("pluginsEmpty")}</strong><span>{systemAdmin ? t("pluginsEmptyHint") : t("pluginsEmptyMemberHint")}</span>{systemAdmin && <button className="button primary" type="button" onClick={() => setInstallerTarget(null)}>{t("pluginInstall")}</button>}</div>
    ) : <div className="plugin-grid">{plugins.map((plugin) => <PluginCard key={plugin.id} plugin={plugin} systemAdmin={systemAdmin} onConfigure={() => setSelected(plugin)} onUpdate={() => setInstallerTarget(plugin)} onToggle={() => void setEnabled(plugin)} onUninstall={() => void uninstall(plugin)} />)}</div>}
    <PluginSurfaceHost api={api} plugins={plugins} placement="admin_tab" organizationId={organizationId} />
    {selected && <ConfigurationDialog
      api={api}
      plugin={selected}
      organizationId={organizationId}
      systemAdmin={systemAdmin}
      onClose={() => setSelected(null)}
      onOpenCredentials={onOpenCredentials}
    />}
    {installerTarget !== undefined && <PluginInstaller api={api} updateTarget={installerTarget} onClose={() => setInstallerTarget(undefined)} onInspected={(value) => { setInstallerTarget(undefined); setInspection(value); }} />}
    {inspection && <PluginAuthorizationDialog api={api} inspection={inspection} onClose={() => setInspection(null)} onInstalled={installed} />}
  </section>;
}

function PluginCard({ plugin, systemAdmin, onConfigure, onUpdate, onToggle, onUninstall }: { plugin: PluginManifest; systemAdmin: boolean; onConfigure: () => void; onUpdate: () => void; onToggle: () => void; onUninstall: () => void }) {
  const { t } = useI18n();
  const configurable = Boolean(plugin.configuration_schema) && plugin.approved_contributions.includes("configuration");
  const healthy = plugin.runtime_status !== "error";
  return <article className={`plugin-card ${healthy ? "" : "failed"}`}>
    <header>
      <div><h3>{plugin.name || plugin.id}</h3><code>{plugin.id}</code></div>
      <span className={`plugin-load-state ${plugin.runtime_status}`}>{t(runtimeStatusKey(plugin.runtime_status))}</span>
    </header>
    {plugin.description && <p>{plugin.description}</p>}
    <dl className="plugin-facts">
      <div><dt>{t("pluginVersion")}</dt><dd>{plugin.version}</dd></div>
      <div><dt>{t("pluginSource")}</dt><dd>{pluginSourceSummary(plugin.source_kind, plugin.source_ref, plugin.source_details) || t("pluginSourceUnknown")}</dd></div>
      {plugin.source_confirmation && <div><dt>{t("pluginSourceConfirmation")}</dt><dd>{t(plugin.source_confirmation === "gitops_mounted" ? "pluginSourceConfigured" : "pluginSourceAdministratorConfirmed")}</dd></div>}
      <div><dt>{t("pluginInterfaceVersion")}</dt><dd>{plugin.wit_version}</dd></div>
    </dl>
    <div className="plugin-permissions"><strong>{t("pluginApprovedContributions")}</strong><div>{plugin.approved_contributions.length ? plugin.approved_contributions.map((contribution) => <span key={contribution}>{t(contributionTitle(contribution))}</span>) : <span>{t("pluginNoPermissionsRequested")}</span>}</div></div>
    {plugin.approved_contributions.includes("workspace_create_policy") && plugin.denial_codes.length > 0 && <div className="plugin-denial-codes"><strong>{t("pluginDenialCodes")}</strong>{plugin.denial_codes.map((code) => <code key={code}>{code}</code>)}</div>}
    {plugin.runtime_error_code && <div className="plugin-failure" role="status"><code>{plugin.runtime_error_code}</code><span>{t(runtimeErrorKey(plugin.runtime_error_code))}</span></div>}
    <details className="plugin-package-details"><summary>{t("pluginPackageDetails")}</summary><dl><dt>{t("pluginPackageVersion")}</dt><dd>#{plugin.package_version}</dd><dt>SHA-256</dt><dd><code>{plugin.package_digest}</code></dd></dl></details>
    <footer>
      <button className="button" type="button" disabled={!healthy || !configurable} onClick={onConfigure}>
        {configurable ? t("pluginConfigure") : t("pluginNoConfiguration")}
      </button>
      {systemAdmin && <><button className="button" type="button" onClick={onUpdate}>{t("pluginUpdate")}</button><button className="button" type="button" onClick={onToggle}>{plugin.enabled ? t("pluginDisable") : t("pluginEnable")}</button><button className="button danger" type="button" onClick={onUninstall}>{t("pluginUninstall")}</button></>}
    </footer>
  </article>;
}

function contributionTitle(contribution: string) {
  const known: Record<string, "pluginContribution_workspace_create_policy" | "pluginContribution_configuration" | "pluginContribution_ui_surfaces" | "pluginContribution_api_routes" | "pluginContribution_api_middleware"> = {
    workspace_create_policy: "pluginContribution_workspace_create_policy", configuration: "pluginContribution_configuration", ui_surfaces: "pluginContribution_ui_surfaces", api_routes: "pluginContribution_api_routes", api_middleware: "pluginContribution_api_middleware",
  };
  return known[contribution] ?? "pluginContributions";
}

function runtimeStatusKey(status: PluginManifest["runtime_status"]): "pluginLoaded" | "pluginDisabledState" | "pluginLoadFailed" {
  if (status === "loaded") return "pluginLoaded";
  if (status === "disabled") return "pluginDisabledState";
  return "pluginLoadFailed";
}

function runtimeErrorKey(code: NonNullable<PluginManifest["runtime_error_code"]>): "pluginCompileFailed" | "pluginSchemaInvalid" | "pluginInterfaceIncompatible" {
  if (code === "compile_failed") return "pluginCompileFailed";
  if (code === "schema_invalid") return "pluginSchemaInvalid";
  return "pluginInterfaceIncompatible";
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
