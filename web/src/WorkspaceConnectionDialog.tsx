import { useState } from "react";
import { Button, Caption1, Dialog, DialogBody, DialogContent, DialogSurface, DialogTitle, Divider, Text, Title3 } from "@fluentui/react-components";
import { CopyRegular, DismissRegular, PlugConnectedRegular } from "@fluentui/react-icons";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type { WorkspaceSshConnection } from "./types";
import { useWorkspaceStyles } from "./workspaces/workspaceStyles";

interface Props {
  api: ApiClient;
  workspaceId: string;
  connection: WorkspaceSshConnection;
}

export function WorkspaceConnectionDialog({ api, workspaceId, connection }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const [open, setOpen] = useState(false);
  const [clientPublicKey, setClientPublicKey] = useState<string | null>(null);
  const [keyLoading, setKeyLoading] = useState(false);
  const [keyUnavailable, setKeyUnavailable] = useState(false);

  async function loadClientPublicKey() {
    if (keyLoading) return;
    setKeyLoading(true);
    setKeyUnavailable(false);
    try {
      const result = await api.workspaceClientPublicKey(workspaceId);
      setClientPublicKey(result.public_key);
    } catch {
      setClientPublicKey(null);
      setKeyUnavailable(true);
    } finally {
      setKeyLoading(false);
    }
  }

  function openDialog() {
    setOpen(true);
    if (!clientPublicKey) void loadClientPublicKey();
  }

  return <>
    <Button appearance="outline" icon={<PlugConnectedRegular />} onClick={openDialog}>{t("openSshConnection")}</Button>
    <Dialog open={open} onOpenChange={(_, data) => setOpen(data.open)}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody className={styles.dialogBody}>
          <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={t("close")} onClick={() => setOpen(false)} />}>{t("sshConnectionTitle")}</DialogTitle>
          <DialogContent className={styles.dialogBody}>
            <Text>{t("sshConnectionIntro")}</Text>
            <dl className={styles.dialogFacts}>
              <Fact label={t("displayName")} value={connection.display_name} styles={styles} />
              <Fact label={t("sshAlias")} value={connection.alias} code styles={styles} />
              <Fact label={t("hostname")} value={connection.hostname} code styles={styles} />
              <Fact label={t("sshPort")} value={String(connection.port)} code styles={styles} />
              <Fact label={t("workspaceUser")} value={connection.user} code styles={styles} />
            </dl>
            <Divider />
            <section className={styles.dialogSection}>
              <Title3>{t("codexAppConnection")}</Title3>
              <Caption1>{t("codexAppAliasHelp")}</Caption1>
              <dl className={styles.dialogFacts}>
                <Fact label={t("displayName")} value={connection.app.display_name} code styles={styles} />
                <Fact label={t("hostname")} value={connection.app.hostname} code styles={styles} />
                <Fact label={t("sshPortOptional")} value={connection.app.ssh_port === null ? t("leaveBlank") : String(connection.app.ssh_port)} code styles={styles} />
              </dl>
            </section>
            <Divider />
            <section className={styles.dialogSection}>
              <Title3>{t("sshConfig")}</Title3>
              <Caption1>{t("sshConfigInstallHelp")}</Caption1>
              <CopyBlock label={t("copySshConfig")} value={connection.config} multiline />
            </section>
            <section className={styles.dialogSection}>
              <Title3>{t("sshCommand")}</Title3>
              <CopyBlock label={t("copy")} value={connection.command} />
            </section>
            <section className={styles.dialogSection}>
              <Title3>{t("workspacePublicKey")}</Title3>
              <Caption1>{t("workspaceClientKeyHelp")}</Caption1>
              {keyLoading && <Text role="status">{t("loadingWorkspacePublicKey")}</Text>}
              {clientPublicKey && <CopyBlock label={t("copyWorkspacePublicKey")} value={clientPublicKey} />}
              {keyUnavailable && <div className={styles.notice}><Text>{t("workspacePublicKeyUnavailable")}</Text><Button appearance="subtle" onClick={() => void loadClientPublicKey()}>{t("retry")}</Button></div>}
            </section>
          </DialogContent>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  </>;
}

function CopyBlock({ label, value, multiline = false }: { label: string; value: string; multiline?: boolean }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try { await navigator.clipboard.writeText(value); setCopied(true); window.setTimeout(() => setCopied(false), 1_200); } catch { /* Clipboard permissions are optional; the value remains selectable. */ }
  };
  return <div className={styles.copyBlock}>
    <pre className={styles.copyValue}><code>{value}</code></pre>
    <Button appearance="secondary" icon={<CopyRegular />} aria-label={`${t("copy")} ${label}`} onClick={() => void copy()}>{copied ? t("copied") : label}</Button>
  </div>;
}

function Fact({ label, value, code = false, styles }: { label: string; value: string; code?: boolean; styles: ReturnType<typeof useWorkspaceStyles> }) {
  return <div className={styles.dialogFact}><Caption1>{label}</Caption1><Text className={code ? styles.code : undefined}>{value}</Text></div>;
}
