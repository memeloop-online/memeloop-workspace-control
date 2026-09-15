import { useState } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Tab,
  TabList,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import type { PluginInspection, PluginManifest, PluginSourceKind } from "./types";
import { isGithubRepository, isSha256 } from "./viewModel";

type InstallMethod = "file" | "url" | "github_release";

const useStyles = makeStyles({
  content: { display: "grid", gap: tokens.spacingVerticalM, minWidth: 0 },
  form: { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: tokens.spacingVerticalM, "@media (max-width: 620px)": { gridTemplateColumns: "1fr" } },
  wide: { gridColumn: "1 / -1", "@media (max-width: 620px)": { gridColumn: "auto" } },
  fileInput: { boxSizing: "border-box", width: "100%", minHeight: "32px", padding: tokens.spacingVerticalXS, color: tokens.colorNeutralForeground1, backgroundColor: tokens.colorNeutralBackground1, ...shorthands.border("1px", "solid", tokens.colorNeutralStroke1), ...shorthands.borderRadius(tokens.borderRadiusMedium) },
});

export function PluginInstaller({ api, updateTarget, onInspected, onClose }: { api: PluginApi; updateTarget: PluginManifest | null; onInspected: (inspection: PluginInspection) => void; onClose: () => void }) {
  const styles = useStyles();
  const { t } = useI18n();
  const initialMethod = installMethod(updateTarget?.source_kind);
  const [method, setMethod] = useState<InstallMethod>(initialMethod);
  const [manifest, setManifest] = useState<File | null>(null);
  const [component, setComponent] = useState<File | null>(null);
  const [assets, setAssets] = useState<File[]>([]);
  const [url, setUrl] = useState(updateTarget?.source_details.kind === "url" ? updateTarget.source_details.url : "");
  const [repository, setRepository] = useState(updateTarget?.source_details.kind === "github_release" ? updateTarget.source_details.repository : "");
  const [tag, setTag] = useState(updateTarget?.source_details.kind === "github_release" ? updateTarget.source_details.tag : "");
  const [asset, setAsset] = useState(updateTarget?.source_details.kind === "github_release" ? updateTarget.source_details.asset : "");
  const [sha256, setSha256] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function inspect() {
    setBusy(true); setError("");
    try {
      let inspection: PluginInspection;
      if (method === "file") {
        if (!manifest) throw new Error(t("pluginManifestRequired"));
        inspection = await api.inspectLocalPackage(manifest, component, assets);
      } else {
        if (!isSha256(sha256)) throw new Error(t("pluginShaRequired"));
        if (method === "url") {
          if (!url.trim()) throw new Error(t("pluginUrlRequired"));
          inspection = await api.inspectUrl(url.trim(), sha256.trim());
        } else {
          if (!isGithubRepository(repository) || !tag.trim() || !asset.trim()) throw new Error(t("pluginReleaseRequired"));
          inspection = await api.inspectGithubRelease(repository.trim(), tag.trim(), asset.trim(), sha256.trim());
        }
      }
      if (updateTarget && inspection.manifest.id !== updateTarget.id) throw new Error(t("pluginUpdateIdMismatch"));
      onInspected(inspection);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("pluginRequestFailed"));
    } finally { setBusy(false); }
  }

  return <Dialog open onOpenChange={(_, data) => !data.open && onClose()}>
    <DialogSurface>
      <DialogBody>
        <DialogTitle action={<Button appearance="subtle" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</Button>}>{updateTarget ? t("pluginUpdateTitle") : t("pluginInstallTitle")}</DialogTitle>
        <DialogContent className={styles.content}>
          <TabList selectedValue={method} onTabSelect={(_, data) => setMethod(data.value as InstallMethod)} aria-label={t("pluginInstallMethod")}>
            <Tab value="file">{t("pluginInstallFile")}</Tab><Tab value="url">{t("pluginInstallUrl")}</Tab><Tab value="github_release">{t("pluginInstallGithub")}</Tab>
          </TabList>
          {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
          <form className={styles.form} onSubmit={(event) => { event.preventDefault(); void inspect(); }}>
            {method === "file" ? <>
              <Field label={t("pluginManifestFile")} hint={t("pluginManifestFileHelp")} required><input className={styles.fileInput} type="file" accept="application/json,.json" required onChange={(event) => setManifest(event.target.files?.[0] ?? null)} /></Field>
              <Field label={t("pluginComponentFile")} hint={t("pluginComponentFileHelp")}><input className={styles.fileInput} type="file" onChange={(event) => setComponent(event.target.files?.[0] ?? null)} /></Field>
              <Field className={styles.wide} label={t("pluginAssetFiles")} hint={t("pluginAssetFilesHelp")}><input className={styles.fileInput} type="file" multiple onChange={(event) => setAssets(Array.from(event.target.files ?? []))} /></Field>
            </> : method === "url" ? <>
              <Field className={styles.wide} label={t("pluginPackageUrl")} required><Input type="url" required value={url} onChange={(_, data) => setUrl(data.value)} placeholder="https://plugins.example.com/example.mwc-plugin" /></Field>
              <DigestField value={sha256} onChange={setSha256} wide />
            </> : <>
              <Field label={t("pluginGithubRepository")} required><Input required value={repository} onChange={(_, data) => setRepository(data.value)} placeholder="owner/repository" /></Field>
              <Field label={t("pluginGithubTag")} required><Input required value={tag} onChange={(_, data) => setTag(data.value)} placeholder="v1.2.0" /></Field>
              <Field label={t("pluginGithubAsset")} required><Input required value={asset} onChange={(_, data) => setAsset(data.value)} placeholder="example.mwc-plugin" /></Field>
              <DigestField value={sha256} onChange={setSha256} />
            </>}
            <DialogActions className={styles.wide}><Button appearance="primary" type="submit" disabled={busy}>{busy ? t("pluginInspecting") : t("pluginReviewInstall")}</Button><Button type="button" onClick={onClose}>{t("cancel")}</Button></DialogActions>
          </form>
        </DialogContent>
      </DialogBody>
    </DialogSurface>
  </Dialog>;
}

function DigestField({ value, onChange, wide = false }: { value: string; onChange: (value: string) => void; wide?: boolean }) {
  const styles = useStyles();
  const { t } = useI18n();
  return <Field className={wide ? styles.wide : undefined} label={t("pluginExpectedSha256")} hint={t("pluginExpectedSha256Help")} required><Input required minLength={64} maxLength={64} pattern="[0-9a-fA-F]{64}" value={value} onChange={(_, data) => onChange(data.value)} placeholder="64-character SHA-256" /></Field>;
}

function installMethod(source: PluginSourceKind | undefined): InstallMethod {
  if (source === "url" || source === "github_release") return source;
  return "file";
}
