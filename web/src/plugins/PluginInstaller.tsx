import { useEffect, useRef, useState } from "react";
import { useI18n } from "../i18n";
import type { PluginApi } from "./api";
import type { PluginInspection, PluginManifest, PluginSourceKind } from "./types";
import { isGithubRepository, isSha256 } from "./viewModel";

type InstallMethod = "file" | "url" | "github_release";

export function PluginInstaller({ api, updateTarget, onInspected, onClose }: {
  api: PluginApi;
  updateTarget: PluginManifest | null;
  onInspected: (inspection: PluginInspection) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
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

  useEffect(() => { dialog.current?.showModal(); return () => dialog.current?.close(); }, []);

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
    } finally {
      setBusy(false);
    }
  }

  return <dialog ref={dialog} className="plugin-dialog plugin-install-dialog" aria-labelledby="plugin-install-title" onCancel={onClose} onClose={onClose}>
    <div className="plugin-dialog-heading"><div><p className="eyebrow">{updateTarget ? updateTarget.id : t("pluginsTitle")}</p><h2 id="plugin-install-title">{updateTarget ? t("pluginUpdateTitle") : t("pluginInstallTitle")}</h2></div><button type="button" className="dialog-close" aria-label={t("pluginCloseDialog")} onClick={onClose}>×</button></div>
    <div className="install-method-tabs" role="tablist" aria-label={t("pluginInstallMethod")}>
      {(["file", "url", "github_release"] as const).map((value) => <button key={value} type="button" role="tab" aria-selected={method === value} className={method === value ? "active" : ""} onClick={() => setMethod(value)}>{t(methodLabel(value))}</button>)}
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    <form className="plugin-install-form" onSubmit={(event) => { event.preventDefault(); void inspect(); }}>
      {method === "file" ? <>
        <label>{t("pluginManifestFile")}<input type="file" accept="application/json,.json" required onChange={(event) => setManifest(event.target.files?.[0] ?? null)} /><small>{t("pluginManifestFileHelp")}</small></label>
        <label>{t("pluginComponentFile")}<input type="file" onChange={(event) => setComponent(event.target.files?.[0] ?? null)} /><small>{t("pluginComponentFileHelp")}</small></label>
        <label className="wide">{t("pluginAssetFiles")}<input type="file" multiple onChange={(event) => setAssets(Array.from(event.target.files ?? []))} /><small>{t("pluginAssetFilesHelp")}</small></label>
      </> : method === "url" ? <>
        <label className="wide">{t("pluginPackageUrl")}<input type="url" required value={url} onChange={(event) => setUrl(event.target.value)} placeholder="https://plugins.example.com/example.mwc-plugin" /></label>
        <DigestField value={sha256} onChange={setSha256} />
      </> : <>
        <label>{t("pluginGithubRepository")}<input required value={repository} onChange={(event) => setRepository(event.target.value)} placeholder="owner/repository" /></label>
        <label>{t("pluginGithubTag")}<input required value={tag} onChange={(event) => setTag(event.target.value)} placeholder="v1.2.0" /></label>
        <label>{t("pluginGithubAsset")}<input required value={asset} onChange={(event) => setAsset(event.target.value)} placeholder="example.mwc-plugin" /></label>
        <DigestField value={sha256} onChange={setSha256} />
      </>}
      <div className="plugin-dialog-actions wide"><button className="button primary" type="submit" disabled={busy}>{busy ? t("pluginInspecting") : t("pluginReviewInstall")}</button><button className="button" type="button" onClick={onClose}>{t("cancel")}</button></div>
    </form>
  </dialog>;
}

function DigestField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useI18n();
  return <label>{t("pluginExpectedSha256")}<input required minLength={64} maxLength={64} pattern="[0-9a-fA-F]{64}" value={value} onChange={(event) => onChange(event.target.value)} placeholder="64-character SHA-256" /><small>{t("pluginExpectedSha256Help")}</small></label>;
}

function installMethod(source: PluginSourceKind | undefined): InstallMethod {
  if (source === "url" || source === "github_release") return source;
  return "file";
}

function methodLabel(method: InstallMethod): "pluginInstallFile" | "pluginInstallUrl" | "pluginInstallGithub" {
  if (method === "url") return "pluginInstallUrl";
  if (method === "github_release") return "pluginInstallGithub";
  return "pluginInstallFile";
}
