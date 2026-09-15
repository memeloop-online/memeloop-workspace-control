import { useMemo, useState } from "react";
import {
  Button,
  Checkbox,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  MessageBar,
  MessageBarBody,
  Text,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import type { PluginContribution, PluginInspection, PluginManifest } from "./types";

const useStyles = makeStyles({
  content: { display: "grid", gap: tokens.spacingVerticalM, minWidth: 0 },
  summary: { display: "grid", gap: tokens.spacingVerticalM, padding: tokens.spacingVerticalM, ...shorthands.borderRadius(tokens.borderRadiusMedium), backgroundColor: tokens.colorNeutralBackground3 },
  summaryText: { display: "grid", gap: tokens.spacingVerticalXS },
  summaryFacts: { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: tokens.spacingVerticalM, margin: 0, "@media (max-width: 620px)": { gridTemplateColumns: "1fr" } },
  fact: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  label: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200 },
  value: { margin: 0, overflowWrap: "anywhere" },
  permissions: { display: "grid", gap: tokens.spacingVerticalS },
  permission: { display: "grid", gridTemplateColumns: "auto minmax(0, 1fr)", gap: tokens.spacingHorizontalS, alignItems: "start", padding: tokens.spacingVerticalS, ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2), ...shorthands.borderRadius(tokens.borderRadiusMedium) },
  permissionText: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  permissionHelp: { color: tokens.colorNeutralForeground2, fontSize: tokens.fontSizeBase200, lineHeight: tokens.lineHeightBase200 },
  controls: { display: "grid", gap: tokens.spacingVerticalS },
  hash: { display: "block", overflowWrap: "anywhere", fontSize: tokens.fontSizeBase200 },
});

export function PluginAuthorizationDialog({ api, inspection, onInstalled, onClose }: { api: PluginApi; inspection: PluginInspection; onInstalled: (plugin: PluginManifest) => void; onClose: () => void }) {
  const styles = useStyles();
  const { locale, t } = useI18n();
  const [approved, setApproved] = useState<PluginContribution[]>([]);
  const [acknowledged, setAcknowledged] = useState(false);
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const expired = inspection.expires_at <= Date.now() / 1000;
  const updating = inspection.current_package_version > 0;
  const size = useMemo(() => new Intl.NumberFormat(locale, { style: "unit", unit: "megabyte", maximumFractionDigits: 2 }).format(inspection.size_bytes / 1_048_576), [inspection.size_bytes, locale]);

  function toggle(contribution: PluginContribution) {
    setApproved((values) => values.includes(contribution) ? values.filter((value) => value !== contribution) : [...values, contribution]);
    setAcknowledged(false);
  }

  async function install() {
    if (!acknowledged || expired) return;
    setBusy(true); setError("");
    try { onInstalled(await api.install(inspection, approved, enabled)); }
    catch (reason) { setError(reason instanceof Error ? reason.message : t("pluginRequestFailed")); }
    finally { setBusy(false); }
  }

  return <Dialog open onOpenChange={(_, data) => !data.open && onClose()}>
    <DialogSurface>
      <DialogBody>
        <DialogTitle action={<Button appearance="subtle" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</Button>}>{updating ? t("pluginAuthorizeUpdate") : t("pluginAuthorizeInstall")}</DialogTitle>
        <DialogContent className={styles.content}>
          {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
          {expired && <MessageBar intent="warning"><MessageBarBody>{t("pluginInspectionExpired")}</MessageBarBody></MessageBar>}
          <section className={styles.summary}>
            <div className={styles.summaryText}><Text size={500} weight="semibold">{inspection.manifest.name}</Text><Text>{inspection.manifest.description}</Text></div>
            <dl className={styles.summaryFacts}><Fact label={t("pluginVersion")} value={inspection.manifest.version} />{updating && <Fact label={t("pluginCurrentPackageVersion")} value={`#${inspection.current_package_version}`} />}<Fact label={t("pluginSource")} value={inspection.source_ref} /><Fact label={t("pluginPackageSize")} value={size} /><Fact label="SHA-256" value={inspection.digest} code /></dl>
          </section>
          <fieldset className={styles.permissions}><legend><Text weight="semibold">{t("pluginPermissionReview")}</Text></legend><Text className={styles.permissionHelp}>{t("pluginPermissionReviewHelp")}</Text>{inspection.declared_contributions.length ? inspection.declared_contributions.map((contribution) => <label key={contribution} className={styles.permission}><Checkbox checked={approved.includes(contribution)} onChange={() => toggle(contribution)} /><span className={styles.permissionText}><Text weight="semibold">{t(contributionTitle(contribution))}</Text><Text className={styles.permissionHelp}>{t(contributionHelp(contribution))}</Text></span></label>) : <Text>{t("pluginNoPermissionsRequested")}</Text>}</fieldset>
          <div className={styles.controls}><Checkbox checked={enabled} onChange={(_, data) => setEnabled(Boolean(data.checked))} label={t("pluginEnableAfterInstall")} /><Checkbox checked={acknowledged} onChange={(_, data) => setAcknowledged(Boolean(data.checked))} label={t("pluginAuthorizationConfirm")} /></div>
          <DialogActions><Button appearance="primary" disabled={!acknowledged || expired || busy} onClick={() => void install()}>{busy ? t("pluginInstalling") : updating ? t("pluginConfirmUpdate") : t("pluginConfirmInstall")}</Button><Button onClick={onClose}>{t("cancel")}</Button></DialogActions>
        </DialogContent>
      </DialogBody>
    </DialogSurface>
  </Dialog>;
}

function Fact({ label, value, code = false }: { label: string; value: string; code?: boolean }) {
  const styles = useStyles();
  return <div className={styles.fact}><dt className={styles.label}>{label}</dt><dd className={`${styles.value}${code ? ` ${styles.hash}` : ""}`}>{code ? <code>{value}</code> : value}</dd></div>;
}

function contributionTitle(contribution: PluginContribution) {
  return `pluginContribution_${contribution}` as const;
}

function contributionHelp(contribution: PluginContribution) {
  return `pluginContributionHelp_${contribution}` as const;
}
