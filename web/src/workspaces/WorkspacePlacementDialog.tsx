import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Text,
} from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";

import { useI18n } from "../i18n";
import { NodePoolSelect, nodePoolDisplayName } from "../forms/NodePoolPicker";
import type { AvailableNodePool, WorkspaceResponse } from "../types";
import { useWorkspaceStyles } from "./workspaceStyles";

interface Props {
  item: WorkspaceResponse | null;
  nodePools: AvailableNodePool[];
  busy: boolean;
  onClose: () => void;
  onConfirm: (nodePool: string) => void;
}

/** Lets an administrator move a stopped workspace to another allowed node pool. */
export function WorkspacePlacementDialog({ item, nodePools, busy, onClose, onConfirm }: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const workspace = item?.workspace;
  const [selected, setSelected] = useState("");

  useEffect(() => {
    setSelected(workspace?.node_pool ?? "");
  }, [workspace?.id, workspace?.node_pool]);

  if (!workspace) return null;
  const allowed = workspace.placement.allowed_node_pools;
  const unchanged = selected === workspace.node_pool;

  return (
    <Dialog open onOpenChange={(_, data) => { if (!data.open && !busy) onClose(); }}>
      <DialogSurface className={styles.dialogSurface}>
        <DialogBody>
          <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={t("cancel")} disabled={busy} onClick={onClose} />}>
            {t("changeLocationTitle")}
          </DialogTitle>
          <DialogContent className={styles.dialogBody}>
            <Text as="p">{t("changeLocationHelp")}</Text>
            <dl className={styles.dialogFacts}>
              <div className={styles.dialogFact}>
                <Text weight="semibold">{t("workspaces")}</Text>
                <Text>{workspace.name}</Text>
              </div>
              <div className={styles.dialogFact}>
                <Text weight="semibold">{t("currentNodePool")}</Text>
                <Text>{nodePoolDisplayName(nodePools, workspace.node_pool)}</Text>
              </div>
            </dl>
            <Field label={t("nodePool")} required>
              <NodePoolSelect
                pools={nodePools}
                allowed={allowed}
                value={selected}
                disabled={busy}
                onChange={setSelected}
                aria-label={t("nodePool")}
              />
            </Field>
          </DialogContent>
          <DialogActions className={styles.dialogActions}>
            <Button appearance="secondary" disabled={busy} onClick={onClose}>{t("cancel")}</Button>
            <Button appearance="primary" disabled={busy || unchanged || !selected} onClick={() => onConfirm(selected)}>
              {busy ? t("saving") : t("saveLocation")}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
