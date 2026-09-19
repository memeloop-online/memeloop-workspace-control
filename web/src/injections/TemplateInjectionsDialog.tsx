import { useEffect, useState } from "react";
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Divider,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { DismissRegular } from "@fluentui/react-icons";

import type { ApiClient } from "../api";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useI18n } from "../i18n";
import type { StoredInjection, WorkspaceTemplate } from "../types";
import { InjectionEditorForm } from "./InjectionEditorForm";
import { InjectionList } from "./InjectionList";
import {
  draftFromStored,
  emptyInjectionDraft,
  hasSubstantiveInjectionChanges,
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

const useStyles = makeStyles({
  surface: {
    width: "min(72rem, calc(100vw - 2rem))",
    maxWidth: "72rem",
  },
  body: {
    minHeight: 0,
  },
  layout: {
    display: "grid",
    gridTemplateColumns: "minmax(17rem, 0.8fr) minmax(0, 1.4fr)",
    gap: tokens.spacingVerticalL,
    alignItems: "start",
    [`@media (max-width: 760px)`]: {
      gridTemplateColumns: "1fr",
    },
  },
  editor: {
    minWidth: 0,
    overflow: "auto",
    maxHeight: "min(65vh, 48rem)",
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
  },
});

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
  const styles = useStyles();
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [draft, setDraft] = useState<InjectionEditorDraft>(() => emptyInjectionDraft(template.id));
  const [baselineDraft, setBaselineDraft] = useState<InjectionEditorDraft>(() => emptyInjectionDraft(template.id));
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    setSelectedKey(null);
    const next = emptyInjectionDraft(template.id);
    setDraft(next);
    setBaselineDraft(next);
    setSearch("");
  }, [template.id]);

  useEffect(() => {
    if (!open) return;
    void load();
  }, [open, template.id, organizationId]);

  async function load() {
    setLoading(true);
    try {
      const organizationItems = await api.injections("organization", organizationId);
      setItems(organizationItems.filter((item) => item.template_selector === template.id));
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setLoading(false);
    }
  }

  function resetDraft() {
    const next = emptyInjectionDraft(template.id);
    setSelectedKey(null);
    setDraft(next);
    setBaselineDraft(next);
  }

  async function selectItem(item: StoredInjection) {
    if (selectedKey === item.key) {
      resetDraft();
      return;
    }
    setSelectedKey(item.key);
    const selected = draftFromStored(item);
    setDraft(selected);
    setBaselineDraft(selected);
    if (item.sensitive || item.kind === "secret_file") return;
    try {
      const value = await api.injectionValue("organization", organizationId, item.key);
      setDraft((current) => current.key === item.key
        ? { ...current, value, storedValueAvailable: true }
        : current);
      setBaselineDraft((current) => current.key === item.key
        ? { ...current, value, storedValueAvailable: true }
        : current);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    }
  }

  async function save() {
    if (!hasSubstantiveInjectionChanges(draft, baselineDraft)) return;
    setSaving(true);
    try {
      const item = injectionDraftForSave(draft, template.id);
      await api.replaceInjection("organization", organizationId, { ...item, locked: draft.locked });
      resetDraft();
      await load();
    } catch (error) {
      onError(error instanceof Error && error.message === "invalid_file_mode" ? t("invalidFileMode") : message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!selectedKey) return;
    setSaving(true);
    try {
      await api.deleteInjection("organization", organizationId, selectedKey);
      resetDraft();
      setConfirmDelete(false);
      await load();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  function close() {
    if (saving) return;
    onClose();
    requestAnimationFrame(() => returnFocusRef.current?.focus());
  }

  return (
    <>
      <Dialog open={open} onOpenChange={(_, data) => { if (!data.open) close(); }}>
        <DialogSurface className={styles.surface} aria-busy={loading || saving}>
          <DialogBody className={styles.body}>
            <DialogTitle action={<Button appearance="subtle" icon={<DismissRegular aria-hidden="true" />} onClick={close}>{t("close")}</Button>}>
              {t("manageTemplateInjections")} · {template.name}
            </DialogTitle>
            <DialogContent>
              <p>{t("templateInjectionDialogHelp")}</p>
              <Divider />
              <div className={styles.layout}>
                <InjectionList items={items} selectedKey={selectedKey} search={search} loading={loading} title={t("savedCredentials")} emptyLabel={t("noTemplateInjections")} onSearchChange={setSearch} onSelect={selectItem} />
                <div className={styles.editor}>
                  <InjectionEditorForm draft={draft} baseline={baselineDraft} update={setDraft} scope="organization" templates={[template]} fixedTemplate={template} selectedKey={selectedKey} saving={saving} disabled={loading} onReset={resetDraft} onSubmit={save} onDelete={() => setConfirmDelete(true)} />
                </div>
              </div>
            </DialogContent>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <ConfirmDialog open={confirmDelete} title={t("delete")} description={t("deleteCredentialConfirm")} confirmLabel={t("delete")} cancelLabel={t("cancel")} busy={saving} danger details={selectedKey && <code>{selectedKey}</code>} onClose={() => setConfirmDelete(false)} onConfirm={() => void remove()} />
    </>
  );
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
