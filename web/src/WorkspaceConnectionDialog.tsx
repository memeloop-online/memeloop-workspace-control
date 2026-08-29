import { useRef, useState } from "react";
import { useI18n } from "./i18n";
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
  return <>
    <button className="connection-dialog-trigger" onClick={() => dialog.current?.showModal()}>{t("openSshConnection")}</button>
    <dialog ref={dialog} className="connection-dialog" aria-labelledby={titleId} onClick={(event) => {
      if (event.target === dialog.current) dialog.current.close();
    }}>
      <div className="connection-dialog-content">
        <header><div><span className="eyebrow">SSH</span><h3 id={titleId}>{t("sshConnectionTitle")}</h3></div><button className="connection-dialog-close" aria-label={t("close")} onClick={() => dialog.current?.close()}>×</button></header>
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
        {(workspaceHostKey || jumpHostKey) && <section aria-labelledby={`${titleId}-keys`}>
          <h4 id={`${titleId}-keys`}>{t("hostKeys")}</h4>
          {workspaceHostKey && <CopyBlock label={t("hostKey")} value={`${workspaceHostKey.fingerprint} ${workspaceHostKey.public_key}`} />}
          {jumpHostKey && <CopyBlock label={t("jumpKey")} value={`${jumpHostKey.fingerprint} ${jumpHostKey.public_key}`} />}
        </section>}
      </div>
    </dialog>
  </>;
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
