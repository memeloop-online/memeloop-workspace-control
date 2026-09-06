import { useEffect, useId, useRef, type ReactNode } from "react";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel: string;
  busy?: boolean;
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
  danger = false,
  details,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={dialogRef}
      className="confirm-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
      onClose={() => {
        if (open && !busy) onClose();
      }}
    >
      <div className="confirm-dialog-body">
        <span className={danger ? "confirm-dialog-icon danger" : "confirm-dialog-icon"} aria-hidden="true">
          {danger ? "!" : "↻"}
        </span>
        <div>
          <h3 id={titleId}>{title}</h3>
          <p>{description}</p>
          {details && <div className="confirm-dialog-details">{details}</div>}
        </div>
      </div>
      <div className="confirm-dialog-actions">
        <button className="button" type="button" disabled={busy} onClick={onClose}>{cancelLabel}</button>
        <button className={`button ${danger ? "danger" : "primary"}`} type="button" disabled={busy} onClick={onConfirm}>{confirmLabel}</button>
      </div>
    </dialog>
  );
}
