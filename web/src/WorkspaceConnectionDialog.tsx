import { useRef, useState } from "react";
import { useI18n } from "./i18n";
import { jumpHostKnownHostsEntry, workspaceKnownHostsEntry } from "./sshIdentity";
import type { WorkspaceResponse, WorkspaceSshConnection } from "./types";

interface Props {
  connection: WorkspaceSshConnection;
  workspaceHostKey: WorkspaceResponse["workspace_host_key"];
  jumpHostKey: WorkspaceResponse["jump_host_key"];
}

export function WorkspaceConnectionDialog({ connection, workspaceHostKey, jumpHostKey }: Props) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = `connection-title-${connection.alias}`;
  const jumpKnownHosts = jumpHostKey ? jumpHostKnownHostsEntry(connection, jumpHostKey) : null;
  return <>
    <button className="connection-dialog-trigger" onClick={() => dialog.current?.showModal()}>{t("openSshConnection")}</button>
    <dialog ref={dialog} className="connection-dialog" aria-labelledby={titleId} onClick={(event) => {
      if (event.target === dialog.current) dialog.current.close();
    }}>
      <div className="connection-dialog-content">
        <header><h3 id={titleId}>{t("sshConnectionTitle")}</h3><button className="connection-dialog-close" aria-label={t("close")} onClick={() => dialog.current?.close()}>×</button></header>
        <p className="connection-dialog-intro">{t("sshConnectionIntro")}</p>
        <dl className="connection-facts">
          <div><dt>{t("displayName")}</dt><dd>{connection.display_name}</dd></div>
          <div><dt>{t("sshAlias")}</dt><dd><code>{connection.alias}</code></dd></div>
          <div><dt>{t("hostname")}</dt><dd><code>{connection.hostname}</code></dd></div>
          <div><dt>{t("sshPort")}</dt><dd><code>{connection.port}</code></dd></div>
          <div><dt>{t("workspaceUser")}</dt><dd><code>{connection.user}</code></dd></div>
        </dl>

        <section aria-labelledby={`${titleId}-app`}>
          <h4 id={`${titleId}-app`}>{t("codexAppConnection")}</h4>
          <p>{t("codexAppAliasHelp")}</p>
          <dl className="connection-facts app-fields">
            <div><dt>{t("displayName")}</dt><dd><code>{connection.app.display_name}</code></dd></div>
            <div><dt>{t("hostname")}</dt><dd><code>{connection.app.hostname}</code></dd></div>
            <div><dt>{t("sshPortOptional")}</dt><dd>{connection.app.ssh_port ?? t("leaveBlank")}</dd></div>
          </dl>
        </section>

        <section aria-labelledby={`${titleId}-config`}>
          <h4 id={`${titleId}-config`}>{t("sshConfig")}</h4>
          <p>{t("sshConfigInstallHelp")}</p>
          <CopyBlock label={t("copySshConfig")} value={connection.config} multiline />
        </section>
        <section aria-labelledby={`${titleId}-command`}>
          <h4 id={`${titleId}-command`}>{t("sshCommand")}</h4>
          <CopyBlock label={t("copy")} value={connection.command} />
        </section>
        {(workspaceHostKey || jumpHostKey) && <section aria-labelledby={`${titleId}-keys`} className="host-identity-section">
          <h4 id={`${titleId}-keys`}>{t("hostIdentity")}</h4>
          <p>{t("hostIdentityHelp")}</p>
          {workspaceHostKey && <IdentityBlock title={t("workspaceHost")} fingerprint={workspaceHostKey.fingerprint} knownHosts={workspaceKnownHostsEntry(connection, workspaceHostKey)} />}
          {jumpHostKey && <IdentityBlock title={t("jumpHost")} fingerprint={jumpHostKey.fingerprint} knownHosts={jumpKnownHosts} />}
        </section>}
        <section aria-labelledby={`${titleId}-client-key`} className="client-key-section">
          <h4 id={`${titleId}-client-key`}>{t("workspaceClientKey")}</h4>
          <p>{t("workspaceClientKeyHelp")}</p>
          <CopyBlock label={t("copyClientKeyCommand")} value="cat ~/.ssh/id_ed25519.pub" />
        </section>
      </div>
    </dialog>
  </>;
}

function IdentityBlock({ title, fingerprint, knownHosts }: { title: string; fingerprint: string; knownHosts: string | null }) {
  const { t } = useI18n();
  return <div className="host-identity">
    <strong>{title}</strong>
    <CopyBlock label={t("copyFingerprint")} value={fingerprint} />
    {knownHosts && <CopyBlock label={t("copyKnownHostsEntry")} value={knownHosts} />}
  </div>;
}

function CopyBlock({ label, value, multiline = false }: { label: string; value: string; multiline?: boolean }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void navigator.clipboard.writeText(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1_200);
  };
  return <div className={`connection-copy${multiline ? " multiline" : ""}`}>
    {multiline ? <pre><code>{value}</code></pre> : <code>{value}</code>}
    <button aria-label={`${t("copy")} ${label}`} onClick={copy}>{copied ? t("copied") : label}</button>
  </div>;
}
