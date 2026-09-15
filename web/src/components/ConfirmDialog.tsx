import type { ReactNode } from "react";
import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Text,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel: string;
  busy?: boolean;
  confirmDisabled?: boolean;
  danger?: boolean;
  details?: ReactNode;
  onConfirm: () => void;
  onClose: () => void;
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel,
  busy = false,
  confirmDisabled = false,
  danger = false,
  details,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  const styles = useStyles();
  return (
    <Dialog open={open} modalType="alert" onOpenChange={(_, data) => { if (!data.open && !busy) onClose(); }}>
      <DialogSurface>
        <DialogBody>
          <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={cancelLabel} disabled={busy} onClick={onClose} />}>{title}</DialogTitle>
          <DialogContent className={styles.content}>
            <Text as="p" className={styles.description}>{description}</Text>
            {details && <div className={styles.details}>{details}</div>}
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" disabled={busy} onClick={onClose}>{cancelLabel}</Button>
            <Button appearance="primary" className={danger ? styles.danger : undefined} disabled={busy || confirmDisabled} onClick={onConfirm}>{confirmLabel}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

const useStyles = makeStyles({
  content: {
    display: "grid",
    rowGap: tokens.spacingVerticalM,
  },
  description: {
    margin: 0,
    color: tokens.colorNeutralForeground2,
  },
  details: {
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
    backgroundColor: tokens.colorNeutralBackground2,
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    overflowWrap: "anywhere",
  },
  danger: {
    backgroundColor: tokens.colorStatusDangerBackground3,
    color: tokens.colorNeutralForegroundOnBrand,
    ":hover": {
      backgroundColor: tokens.colorStatusDangerBackground3Hover,
      color: tokens.colorNeutralForegroundOnBrand,
    },
    ":active": {
      backgroundColor: tokens.colorStatusDangerBackground3Pressed,
      color: tokens.colorNeutralForegroundOnBrand,
    },
  },
});
