import { useEffect, useId, useRef, type FormEvent, type ReactNode } from "react";

interface FormDialogProps {
  open: boolean;
  title: string;
  description?: string;
  submitLabel: string;
  cancelLabel: string;
  busy?: boolean;
  danger?: boolean;
  children: ReactNode;
  onSubmit: () => void;
  onClose: () => void;
}

export function FormDialog({ open, title, description, submitLabel, cancelLabel, busy = false, danger = false, children, onSubmit, onClose }: FormDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      requestAnimationFrame(() => dialog.querySelector<HTMLElement>("input, select, textarea")?.focus());
    }
    if (!open && dialog.open) dialog.close();
  }, [open]);

  function submit(event: FormEvent) {
    event.preventDefault();
    onSubmit();
  }

  return <dialog ref={dialogRef} className="confirm-dialog form-dialog" aria-labelledby={titleId} aria-describedby={description ? descriptionId : undefined} onCancel={(event) => { event.preventDefault(); if (!busy) onClose(); }} onClose={() => { if (open && !busy) onClose(); }}>
    <form onSubmit={submit}>
      <div className="confirm-dialog-body form-dialog-body">
        <div>
          <h3 id={titleId}>{title}</h3>
          {description && <p id={descriptionId}>{description}</p>}
          <div className="form-dialog-fields">{children}</div>
        </div>
      </div>
      <div className="confirm-dialog-actions">
        <button className="button" type="button" disabled={busy} onClick={onClose}>{cancelLabel}</button>
        <button className={`button ${danger ? "danger" : "primary"}`} type="submit" disabled={busy}>{submitLabel}</button>
      </div>
    </form>
  </dialog>;
}
