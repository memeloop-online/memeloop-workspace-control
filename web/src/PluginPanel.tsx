import Form from "@rjsf/core";
import {
  Badge,
  Button,
  Card,
  CardFooter,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  MessageBar,
  MessageBarBody,
  Spinner,
  Tab,
  TabList,
  Text,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useI18n } from "./i18n";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { PluginApi } from "./plugins/api";
import { PluginAuthorizationDialog } from "./plugins/PluginAuthorizationDialog";
import { PluginInstaller } from "./plugins/PluginInstaller";
import { PluginSurfaceHost } from "./plugins/PluginSurfaceHost";
import { checkPluginSchema, configurationKey } from "./plugins/schema";
import { safeValidator } from "./plugins/safeValidator";
import { pluginCatalogState, pluginErrorMessageKey, pluginSourceSummary } from "./plugins/viewModel";
import type { PluginConfiguration, PluginConfigurationScope, PluginInspection, PluginManifest } from "./plugins/types";

const useStyles = makeStyles({
  page: { display: "grid", gap: tokens.spacingVerticalL, minWidth: 0 },
  heading: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: tokens.spacingHorizontalM, flexWrap: "wrap" },
  title: { margin: 0, fontSize: tokens.fontSizeBase600, fontWeight: tokens.fontWeightSemibold },
  grid: { display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(320px, 1fr))", gap: tokens.spacingHorizontalL },
  card: { display: "grid", gap: tokens.spacingVerticalM, minWidth: 0, height: "100%", padding: tokens.spacingVerticalL },
  cardHeader: { display: "flex", alignItems: "start", justifyContent: "space-between", gap: tokens.spacingHorizontalM },
  cardTitle: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  id: { color: tokens.colorNeutralForeground3, fontFamily: tokens.fontFamilyMonospace, fontSize: tokens.fontSizeBase200, overflowWrap: "anywhere" },
  status: { flex: "none" },
  description: { margin: 0, color: tokens.colorNeutralForeground2, lineHeight: tokens.lineHeightBase300 },
  facts: { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: tokens.spacingVerticalM, margin: 0 },
  fact: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  factLabel: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  factValue: { margin: 0, overflowWrap: "anywhere", fontSize: tokens.fontSizeBase300 },
  permissionBlock: { display: "grid", gap: tokens.spacingVerticalXS },
  permissionList: { display: "flex", flexWrap: "wrap", gap: tokens.spacingHorizontalXS },
  chip: { maxWidth: "100%" },
  failure: { display: "grid", gap: tokens.spacingVerticalXS, padding: tokens.spacingVerticalS, ...shorthands.borderRadius(tokens.borderRadiusMedium), backgroundColor: tokens.colorPaletteRedBackground2 },
  packageDetails: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  packageList: { display: "grid", gridTemplateColumns: "auto minmax(0, 1fr)", gap: tokens.spacingVerticalXS, margin: tokens.spacingVerticalS, overflowWrap: "anywhere" },
  actions: { display: "flex", flexWrap: "wrap", gap: tokens.spacingHorizontalS, marginTop: "auto" },
  empty: { display: "grid", placeItems: "center", gap: tokens.spacingVerticalS, minHeight: "180px", padding: tokens.spacingVerticalL, color: tokens.colorNeutralForeground2 },
  dialogContent: { display: "grid", gap: tokens.spacingVerticalM, minWidth: 0 },
  rjsf: {
    display: "grid",
    gap: tokens.spacingVerticalM,
    "& .form-group": { display: "grid", gap: tokens.spacingVerticalXS },
    "& label": { color: tokens.colorNeutralForeground1, fontSize: tokens.fontSizeBase300, fontWeight: tokens.fontWeightSemibold },
    "& input, & textarea, & select": { boxSizing: "border-box", width: "100%", minHeight: "32px", paddingInline: tokens.spacingHorizontalS, color: tokens.colorNeutralForeground1, backgroundColor: tokens.colorNeutralBackground1, ...shorthands.border("1px", "solid", tokens.colorNeutralStroke1), ...shorthands.borderRadius(tokens.borderRadiusMedium) },
    "& textarea": { minHeight: "96px", paddingBlock: tokens.spacingVerticalXS },
    "& .field-description, & .help-block": { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200, fontWeight: tokens.fontWeightRegular },
  },
  guidance: { margin: 0, color: tokens.colorNeutralForeground2, lineHeight: tokens.lineHeightBase300 },
  statusList: { display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: tokens.spacingHorizontalM, margin: 0, padding: tokens.spacingVerticalM, ...shorthands.borderRadius(tokens.borderRadiusMedium), backgroundColor: tokens.colorNeutralBackground3, "@media (max-width: 620px)": { gridTemplateColumns: "1fr" } },
  statusItem: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  statusLabel: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  statusValue: { margin: 0, fontSize: tokens.fontSizeBase300 },
});

export function PluginPanel({ token, organizationId, systemAdmin, onOpenCredentials }: { token: string; organizationId: string; systemAdmin: boolean; onOpenCredentials: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  const api = useMemo(() => new PluginApi(token), [token]);
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [selected, setSelected] = useState<PluginManifest | null>(null);
  const [installerTarget, setInstallerTarget] = useState<PluginManifest | null | undefined>(undefined);
  const [inspection, setInspection] = useState<PluginInspection | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [pendingUninstall, setPendingUninstall] = useState<PluginManifest | null>(null);
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
    try {
      await api.uninstall(plugin.id, plugin.package_version);
      setPlugins((values) => values.filter((value) => value.id !== plugin.id));
      setPendingUninstall(null);
      setError("");
    } catch (reason) { setError(pluginErrorMessage(reason, t)); }
  }

  function installed(plugin: PluginManifest) {
    setPlugins((values) => values.some((value) => value.id === plugin.id) ? values.map((value) => value.id === plugin.id ? plugin : value) : [...values, plugin]);
    setInspection(null);
    setInstallerTarget(undefined);
  }

  return <section className={styles.page} aria-labelledby="plugin-page-title">
      <div className={styles.heading}>
        <h2 id="plugin-page-title" className={styles.title}>{t("pluginsTitle")}</h2>
        {systemAdmin && <Button appearance="primary" onClick={() => setInstallerTarget(null)}>{t("pluginInstall")}</Button>}
      </div>
      {catalogState === "error" && <MessageBar intent="error"><MessageBarBody>{error}<Button appearance="subtle" onClick={() => void load()}>{t("pluginRetry")}</Button></MessageBarBody></MessageBar>}
      {catalogState === "loading" && <Card><div className={styles.empty} role="status"><Spinner size="small" label={t("pluginsLoading")} /></div></Card>}
      {catalogState === "empty" && <Card><div className={styles.empty}><Text weight="semibold">{t("pluginsEmpty")}</Text><Text>{systemAdmin ? t("pluginsEmptyHint") : t("pluginsEmptyMemberHint")}</Text></div></Card>}
      {catalogState === "ready" && <div className={styles.grid}>{plugins.map((plugin) => <PluginCard key={plugin.id} plugin={plugin} systemAdmin={systemAdmin} onConfigure={() => setSelected(plugin)} onUpdate={() => setInstallerTarget(plugin)} onToggle={() => void setEnabled(plugin)} onUninstall={() => setPendingUninstall(plugin)} />)}</div>}
      <PluginSurfaceHost api={api} plugins={plugins} placement="admin_tab" organizationId={organizationId} />
      {selected && <ConfigurationDialog api={api} plugin={selected} organizationId={organizationId} systemAdmin={systemAdmin} onClose={() => setSelected(null)} onOpenCredentials={onOpenCredentials} />}
      {installerTarget !== undefined && <PluginInstaller api={api} updateTarget={installerTarget} onClose={() => setInstallerTarget(undefined)} onInspected={(value) => { setInstallerTarget(undefined); setInspection(value); }} />}
      {inspection && <PluginAuthorizationDialog api={api} inspection={inspection} onClose={() => setInspection(null)} onInstalled={installed} />}
      <ConfirmDialog open={pendingUninstall !== null} title={t("pluginUninstall")} description={t("pluginUninstallConfirm")} confirmLabel={t("pluginUninstall")} cancelLabel={t("cancel")} danger details={pendingUninstall && <strong>{pendingUninstall.name || pendingUninstall.id}</strong>} onClose={() => setPendingUninstall(null)} onConfirm={() => pendingUninstall && void uninstall(pendingUninstall)} />
    </section>;
}

function PluginCard({ plugin, systemAdmin, onConfigure, onUpdate, onToggle, onUninstall }: { plugin: PluginManifest; systemAdmin: boolean; onConfigure: () => void; onUpdate: () => void; onToggle: () => void; onUninstall: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  const configurable = Boolean(plugin.configuration_schema) && plugin.approved_contributions.includes("configuration");
  const healthy = plugin.runtime_status !== "error";
  return <Card className={styles.card} appearance="filled-alternative">
    <div className={styles.cardHeader}><div className={styles.cardTitle}><Text size={500} weight="semibold">{plugin.name || plugin.id}</Text><code className={styles.id}>{plugin.id}</code></div><Badge className={styles.status} appearance="tint" color={statusColor(plugin.runtime_status)}>{t(runtimeStatusKey(plugin.runtime_status))}</Badge></div>
    {plugin.description && <p className={styles.description}>{plugin.description}</p>}
    <dl className={styles.facts}>
      <Fact label={t("pluginVersion")} value={plugin.version} />
      <Fact label={t("pluginSource")} value={pluginSourceSummary(plugin.source_kind, plugin.source_ref, plugin.source_details) || t("pluginSourceUnknown")} />
      {plugin.source_confirmation && <Fact label={t("pluginSourceConfirmation")} value={t(plugin.source_confirmation === "gitops_mounted" ? "pluginSourceConfigured" : "pluginSourceAdministratorConfirmed")} />}
      <Fact label={t("pluginInterfaceVersion")} value={plugin.wit_version} />
    </dl>
    <div className={styles.permissionBlock}><Text className={styles.factLabel} weight="semibold">{t("pluginApprovedContributions")}</Text><div className={styles.permissionList}>{plugin.approved_contributions.length ? plugin.approved_contributions.map((contribution) => <Badge key={contribution} className={styles.chip} appearance="tint" color="informative">{t(contributionTitle(contribution))}</Badge>) : <Text className={styles.factLabel}>{t("pluginNoPermissionsRequested")}</Text>}</div></div>
    {plugin.approved_contributions.includes("workspace_create_policy") && plugin.denial_codes.length > 0 && <div className={styles.permissionBlock}><Text className={styles.factLabel} weight="semibold">{t("pluginDenialCodes")}</Text><div className={styles.permissionList}>{plugin.denial_codes.map((code) => <Badge key={code} className={styles.chip} appearance="tint" color="warning">{code}</Badge>)}</div></div>}
    {plugin.runtime_error_code && <div className={styles.failure} role="status"><code>{plugin.runtime_error_code}</code><Text>{t(runtimeErrorKey(plugin.runtime_error_code))}</Text></div>}
    <details className={styles.packageDetails}><summary>{t("pluginPackageDetails")}</summary><dl className={styles.packageList}><dt>{t("pluginPackageVersion")}</dt><dd>#{plugin.package_version}</dd><dt>SHA-256</dt><dd><code>{plugin.package_digest}</code></dd></dl></details>
    <CardFooter className={styles.actions}>
      <Button disabled={!healthy || !configurable} onClick={onConfigure}>{configurable ? t("pluginConfigure") : t("pluginNoConfiguration")}</Button>
      {systemAdmin && <><Button onClick={onUpdate}>{t("pluginUpdate")}</Button><Button onClick={onToggle}>{plugin.enabled ? t("pluginDisable") : t("pluginEnable")}</Button><Button appearance="primary" onClick={onUninstall}>{t("pluginUninstall")}</Button></>}
    </CardFooter>
  </Card>;
}

function Fact({ label, value }: { label: string; value: string }) {
  const styles = useStyles();
  return <div className={styles.fact}><dt className={styles.factLabel}>{label}</dt><dd className={styles.factValue}>{value}</dd></div>;
}

function statusColor(status: PluginManifest["runtime_status"]): "success" | "warning" | "danger" {
  if (status === "loaded") return "success";
  if (status === "disabled") return "warning";
  return "danger";
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

function ConfigurationDialog({ api, plugin, organizationId, systemAdmin, onClose, onOpenCredentials }: { api: PluginApi; plugin: PluginManifest; organizationId: string; systemAdmin: boolean; onClose: () => void; onOpenCredentials: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  const [scope, setScope] = useState<PluginConfigurationScope>(systemAdmin ? "installation" : "organization");
  const [configurations, setConfigurations] = useState<Record<string, PluginConfiguration>>({});
  const [formData, setFormData] = useState<unknown>(plugin.configuration_default ?? {});
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [confirmDeleteOverride, setConfirmDeleteOverride] = useState(false);
  const schema = useMemo(() => checkPluginSchema(plugin.configuration_schema), [plugin.configuration_schema]);
  const current = configurations[configurationKey(plugin.id, scope)];

  const load = useCallback(async () => {
    setBusy(true);
    try {
      const organization = await api.configuration(plugin.id, organizationId);
      const values: Record<string, PluginConfiguration> = { [configurationKey(plugin.id, "organization")]: organization };
      if (systemAdmin) values[configurationKey(plugin.id, "installation")] = await api.configuration(plugin.id, null);
      setConfigurations(values);
      setError("");
    } catch (reason) { setError(messageOf(reason, t("pluginRequestFailed"))); }
    finally { setBusy(false); }
  }, [api, organizationId, plugin.id, systemAdmin, t]);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => { if (current) setFormData(current.value); }, [current]);

  async function save(value: unknown) {
    if (!current) return;
    setBusy(true); setError(""); setMessage("");
    try {
      const saved = await api.putConfiguration(plugin.id, scope === "organization" ? organizationId : null, { expected_version: current.scope_version, value });
      setConfigurations((values) => ({ ...values, [configurationKey(plugin.id, scope)]: saved }));
      setFormData(saved.value); setMessage(t("pluginConfigurationSaved"));
    } catch (reason) { setError(pluginErrorMessage(reason, t)); }
    finally { setBusy(false); }
  }

  async function removeOverride() {
    if (!current || current.scope_version === 0) return;
    setBusy(true); setError(""); setMessage("");
    try {
      const inherited = await api.deleteConfiguration(plugin.id, scope === "organization" ? organizationId : null, current.scope_version);
      setConfigurations((values) => ({ ...values, [configurationKey(plugin.id, scope)]: inherited }));
      setFormData(inherited.value); setConfirmDeleteOverride(false); setMessage(t("pluginOverrideDeleted"));
    } catch (reason) { setError(pluginErrorMessage(reason, t)); }
    finally { setBusy(false); }
  }

  return <Dialog open onOpenChange={(_, data) => !data.open && onClose()}>
    <DialogSurface>
      <DialogBody>
        <DialogTitle action={<Button appearance="subtle" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</Button>}>{t("pluginConfigurationTitle")}</DialogTitle>
        <DialogContent className={styles.dialogContent}>
          <TabList selectedValue={scope} onTabSelect={(_, data) => setScope(data.value as PluginConfigurationScope)} aria-label={t("pluginConfigurationScope")}>
            {systemAdmin && <Tab value="installation">{t("pluginScopeGlobal")}</Tab>}
            <Tab value="organization">{t("pluginScopeOrganization")}</Tab>
          </TabList>
          <p className={styles.guidance}>{t("pluginSensitiveGuidance")} <Button appearance="subtle" onClick={() => { onClose(); onOpenCredentials(); }}>{t("pluginOpenCredentials")}</Button></p>
          {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
          {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
          {busy && !current && <Spinner size="small" label={t("pluginsLoading")} />}
          {current && <>
            {current.schema_changed && <MessageBar intent="warning"><MessageBarBody>{t("pluginSchemaChanged")}</MessageBarBody></MessageBar>}
            {!current.valid && <MessageBar intent="error"><MessageBarBody>{t("pluginConfigurationInvalid")}</MessageBarBody></MessageBar>}
            <dl className={styles.statusList}><StatusItem label={t("pluginEffectiveSource")} value={sourceLabel(current.source, t)} /><StatusItem label={t("pluginScopeVersion")} value={`v${current.scope_version}`} /><StatusItem label={t("pluginEffectiveVersion")} value={`v${current.effective_version}`} /></dl>
            {!schema.ok ? <MessageBar intent="error"><MessageBarBody>{schema.reason === "sensitive" ? t("pluginSensitiveSchemaRejected") : t("pluginSchemaRejected")}</MessageBarBody></MessageBar> : <Form className={styles.rjsf} key={`${plugin.id}-${scope}-${current.scope_version}`} schema={schema.schema} formData={formData} validator={safeValidator} noHtml5Validate disabled={busy} onChange={({ formData: value }) => setFormData(value)} transformErrors={(errors) => errors.map((entry) => ({ ...entry, message: t("pluginValidationInvalid"), stack: `${entry.property} ${t("pluginValidationInvalid")}` }))} onSubmit={({ formData: value }) => void save(value)}><DialogActions><Button appearance="primary" type="submit" disabled={busy}>{busy ? t("pluginSaving") : t("pluginSaveConfiguration")}</Button><Button type="button" disabled={busy || current.scope_version === 0} onClick={() => setConfirmDeleteOverride(true)}>{t("pluginDeleteOverride")}</Button></DialogActions></Form>}
          </>}
        </DialogContent>
      </DialogBody>
    </DialogSurface>
    <ConfirmDialog open={confirmDeleteOverride} title={t("pluginDeleteOverride")} description={t("pluginDeleteOverrideConfirm")} confirmLabel={t("pluginDeleteOverride")} cancelLabel={t("cancel")} busy={busy} danger onClose={() => setConfirmDeleteOverride(false)} onConfirm={() => void removeOverride()} />
  </Dialog>;
}

function StatusItem({ label, value }: { label: string; value: string }) {
  const styles = useStyles();
  return <div className={styles.statusItem}><dt className={styles.statusLabel}>{label}</dt><dd className={styles.statusValue}>{value}</dd></div>;
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

export type { PluginManifest, PluginConfiguration };
