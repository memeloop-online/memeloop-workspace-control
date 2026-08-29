import { useEffect, useId, useMemo, useRef, useState } from "react";

import type { ApiClient } from "../api";
import { useI18n } from "../i18n";
import type { InjectionKind, StoredInjection, WorkspaceTemplate } from "../types";
import { InjectionEditorForm } from "./InjectionEditorForm";
import {
  draftFromStored,
  emptyInjectionDraft,
  injectionDraftForSave,
} from "./editorModel";
import type { InjectionEditorDraft } from "./editorModel";

interface Props {
  api: ApiClient;
  organizationId: string;
  template: WorkspaceTemplate;
  open: boolean;
  returnFocusRef: React.RefObject<HTMLButtonElement | null>;
  onClose: () => void;
  onError: (message: string) => void;
}

export function TemplateInjectionsDialog({
  api,
  organizationId,
  template,
  open,
  returnFocusRef,
  onClose,
  onError,
}: Props) {
  const { t } = useI18n();
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [draft, setDraft] = useState<InjectionEditorDraft>(() => emptyInjectionDraft(template.id));
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");

  const filteredItems = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return items;
    return items.filter((item) => [item.key, item.target, item.kind, kindLabel(item.kind, t)]
      .some((value) => value.toLocaleLowerCase().includes(query)));
  }, [items, search, t]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      requestAnimationFrame(() => dialog.querySelector<HTMLElement>("[data-dialog-autofocus]")?.focus());
      void load();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open, template.id]);

  useEffect(() => {
    resetDraft();
    setSearch("");
  }, [template.id]);

  async function load() {
    setLoading(true);
    try {
      const organizationItems = await api.injections("organization", organizationId);
      setItems(organizationItems.filter((item) => item.template_selector === template.id));
    } catch (error) {
      onError(message(error));
    } finally {
      setLoading(false);
    }
  }

  function resetDraft() {
    setSelectedKey(null);
    setDraft(emptyInjectionDraft(template.id));
  }

  function selectItem(item: StoredInjection) {
    if (selectedKey === item.key) {
      resetDraft();
      return;
    }
    setSelectedKey(item.key);
    setDraft(draftFromStored(item));
  }

  async function save() {
    setSaving(true);
    try {
      const item = injectionDraftForSave(draft, template.id);
      await api.replaceInjection("organization", organizationId, { ...item, locked: draft.locked });
      resetDraft();
      await load();
    } catch (error) {
      onError(error instanceof Error && error.message === "invalid_file_mode" ? t("invalidFileMode") : message(error));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!selectedKey || !confirm(t("deleteCredentialConfirm"))) return;
    setSaving(true);
    try {
      await api.deleteInjection("organization", organizationId, selectedKey);
      resetDraft();
      await load();
    } catch (error) {
      onError(message(error));
    } finally {
      setSaving(false);
    }
  }

  function requestClose() {
    if (saving) return;
    onClose();
    requestAnimationFrame(() => returnFocusRef.current?.focus());
  }

  return (
    <dialog
      ref={dialogRef}
      className="template-injections-dialog"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      aria-busy={loading || saving}
      onCancel={(event) => { event.preventDefault(); requestClose(); }}
      onClose={() => { if (open) onClose(); }}
      onClick={(event) => { if (event.target === event.currentTarget) requestClose(); }}
    >
      <div className="dialog-surface">
        <header className="dialog-heading">
          <div>
            <h3 id={titleId}>{t("manageTemplateEnvironmentFiles")} · {template.name}</h3>
            <p id={descriptionId}>{t("templateEnvironmentDialogHelp")}</p>
          </div>
          <button type="button" className="button" data-dialog-autofocus onClick={requestClose} disabled={saving} aria-label={t("close")}>{t("close")}</button>
        </header>
        <div className="template-injections-layout">
          <div className="injection-list">
            <h3>{t("savedCredentials")}</h3>
            <input className="credential-search" type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchCredentials")} aria-label={t("searchCredentials")} />
            <div className="credential-scroll">
              {loading && <div className="empty compact">{t("loading")}</div>}
              {!loading && filteredItems.length === 0 && <div className="empty compact">{t("noTemplateEnvironmentFiles")}</div>}
              {!loading && filteredItems.map((item) => (
                <button type="button" className={`injection-row${selectedKey === item.key ? " selected" : ""}`} aria-pressed={selectedKey === item.key} key={item.key} onClick={() => selectItem(item)}>
                  <span className="kind-icon">{kindGlyph(item.kind)}</span>
                  <span><strong>{item.key}</strong><small>{item.target}</small></span>
                  <span className="version">v{item.version}{item.locked ? ` · ${t("locked")}` : ""}</span>
                </button>
              ))}
            </div>
          </div>
          <InjectionEditorForm
            draft={draft}
            update={setDraft}
            scope="organization"
            templates={[template]}
            fixedTemplate={template}
            selectedKey={selectedKey}
            saving={saving}
            disabled={loading}
            className="editor-card template-injections-editor"
            onReset={resetDraft}
            onSubmit={save}
            onDelete={remove}
          />
        </div>
      </div>
    </dialog>
  );
}

function kindGlyph(kind: InjectionKind) {
  return kind === "environment_variable" ? "ENV" : kind === "ssh_public_key" ? "SSH" : kind === "secret_file" ? "SEC" : "CFG";
}

function kindLabel(kind: InjectionKind, t: ReturnType<typeof useI18n>["t"]) {
  return kind === "environment_variable" ? t("environmentVariable") : kind === "ssh_public_key" ? t("sshPublicKey") : kind === "secret_file" ? t("credentialFile") : t("configFile");
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "Request failed";
}
