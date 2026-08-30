import { useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import type { PluginContribution, PluginInspection, PluginManifest } from "./types";

export function PluginAuthorizationDialog({ api, inspection, onInstalled, onClose }: {
  api: PluginApi;
  inspection: PluginInspection;
  onInstalled: (plugin: PluginManifest) => void;
  onClose: () => void;
}) {
  const { locale, t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const [approved, setApproved] = useState<PluginContribution[]>([]);
  const [acknowledged, setAcknowledged] = useState(false);
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const expired = inspection.expires_at <= Date.now() / 1000;
  const updating = inspection.current_package_version > 0;
  const source = inspection.source_ref;
  const size = useMemo(() => new Intl.NumberFormat(locale, { style: "unit", unit: "megabyte", maximumFractionDigits: 2 }).format(inspection.size_bytes / 1_048_576), [inspection.size_bytes, locale]);

  useEffect(() => { dialog.current?.showModal(); return () => dialog.current?.close(); }, []);

  function toggle(contribution: PluginContribution) {
    setApproved((values) => values.includes(contribution) ? values.filter((value) => value !== contribution) : [...values, contribution]);
    setAcknowledged(false);
  }

  async function install() {
    if (!acknowledged || expired) return;
    setBusy(true); setError("");
    try {
      onInstalled(await api.install(inspection, approved, enabled));
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("pluginRequestFailed"));
    } finally {
      setBusy(false);
    }
  }

  return <dialog ref={dialog} className="plugin-dialog plugin-authorization-dialog" aria-labelledby="plugin-authorize-title" onCancel={onClose} onClose={onClose}>
    <div className="plugin-dialog-heading"><div><p className="eyebrow">{inspection.manifest.id}</p><h2 id="plugin-authorize-title">{updating ? t("pluginAuthorizeUpdate") : t("pluginAuthorizeInstall")}</h2></div><button type="button" className="dialog-close" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</button></div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {expired && <div className="error-banner" role="alert">{t("pluginInspectionExpired")}</div>}
    <section className="plugin-review-summary"><div><h3>{inspection.manifest.name}</h3><p>{inspection.manifest.description}</p></div><dl><div><dt>{t("pluginVersion")}</dt><dd>{inspection.manifest.version}</dd></div>{updating && <div><dt>{t("pluginCurrentPackageVersion")}</dt><dd>#{inspection.current_package_version}</dd></div>}<div><dt>{t("pluginSource")}</dt><dd>{source}</dd></div>{inspection.source_confirmation && <div><dt>{t("pluginSourceConfirmation")}</dt><dd>{t(inspection.source_confirmation === "gitops_mounted" ? "pluginSourceConfigured" : "pluginSourceAdministratorConfirmed")}</dd></div>}<div><dt>{t("pluginPackageSize")}</dt><dd>{size}</dd></div><div><dt>SHA-256</dt><dd><code>{inspection.digest}</code></dd></div></dl></section>
    <fieldset className="permission-review"><legend>{t("pluginPermissionReview")}</legend><p>{t("pluginPermissionReviewHelp")}</p>{inspection.declared_contributions.length ? inspection.declared_contributions.map((contribution) => <label key={contribution} className={`permission-option ${permissionRisk(contribution)}`}><input type="checkbox" checked={approved.includes(contribution)} onChange={() => toggle(contribution)} /><span><strong>{t(contributionTitle(contribution))}</strong><small>{t(contributionHelp(contribution))}</small></span></label>) : <div className="empty compact">{t("pluginNoPermissionsRequested")}</div>}</fieldset>
    <div className="authorization-controls"><label><input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />{t("pluginEnableAfterInstall")}</label><label className="authorization-ack"><input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.target.checked)} />{t("pluginAuthorizationConfirm")}</label></div>
    <div className="plugin-dialog-actions"><button className="button primary" type="button" disabled={!acknowledged || expired || busy} onClick={() => void install()}>{busy ? t("pluginInstalling") : updating ? t("pluginConfirmUpdate") : t("pluginConfirmInstall")}</button><button className="button" type="button" onClick={onClose}>{t("cancel")}</button></div>
  </dialog>;
}

function permissionRisk(contribution: PluginContribution): "low" | "medium" | "high" {
  if (contribution === "configuration") return "low";
  if (contribution === "ui_surfaces") return "medium";
  return "high";
}

function contributionTitle(contribution: PluginContribution) {
  return `pluginContribution_${contribution}` as const;
}

function contributionHelp(contribution: PluginContribution) {
  return `pluginContributionHelp_${contribution}` as const;
}
