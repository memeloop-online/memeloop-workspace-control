import {
  Button,
  Card,
  Field,
  Tab,
  TabList,
  Text,
  Textarea,
} from "@fluentui/react-components";
import { AddRegular, DeleteRegular, EditRegular, SaveRegular } from "@fluentui/react-icons";
import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";

import type { ApiClient } from "./api";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import { TemplateInjectionsDialog } from "./injections/TemplateInjectionsDialog";
import {
  TemplateDraftError,
  emptyTemplateDraft,
  templateDraftFromTemplate,
  templateDraftFromYaml,
  templateDraftToYaml,
} from "./templates/templateDraft";
import type { TemplateDraft } from "./templates/templateDraft";
import { TemplateForm } from "./templates/TemplateForm";
import type { AvailableNodePool, WorkspaceTemplate } from "./types";
import { AdminToolbar, SaveButton, useAdminStyles } from "./admin/fluentAdmin";

interface Props {
  api: ApiClient;
  organizationId: string;
  templates: WorkspaceTemplate[];
  canGrantClusterAccess: boolean;
  onRefresh: () => Promise<void>;
  onError: (message: string) => void;
}

export function TemplateEditor({ api, organizationId, templates, canGrantClusterAccess, onRefresh, onError }: Props) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const manageButtonRef = useRef<HTMLButtonElement>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<"form" | "yaml">("form");
  const [draft, setDraft] = useState<TemplateDraft>(emptyTemplateDraft);
  const [yamlText, setYamlText] = useState(() => templateDraftToYaml(emptyTemplateDraft()));
  const [saving, setSaving] = useState(false);
  const [managingInjections, setManagingInjections] = useState(false);
  const [pendingSave, setPendingSave] = useState<{ candidate: TemplateDraft; yaml: string } | null>(null);
  const [pendingAction, setPendingAction] = useState<{ kind: "disable" | "delete"; template: WorkspaceTemplate } | null>(null);
  const [nodePools, setNodePools] = useState<AvailableNodePool[]>([]);
  const [nodePoolsError, setNodePoolsError] = useState(false);
  const selected = useMemo(() => templates.find((item) => item.id === selectedId) ?? null, [templates, selectedId]);

  useEffect(() => {
    let active = true;
    setNodePoolsError(false);
    api.nodePools()
      .then((pools) => { if (active) setNodePools(pools); })
      .catch(() => { if (active) { setNodePools([]); setNodePoolsError(true); } });
    return () => { active = false; };
  }, [api]);

  function startNew() {
    const next = emptyTemplateDraft();
    setSelectedId(null);
    setDraft(next);
    setYamlText(templateDraftToYaml(next));
    setMode("form");
    setManagingInjections(false);
  }

  function selectTemplate(template: WorkspaceTemplate) {
    try {
      const next = templateDraftFromTemplate(template);
      setSelectedId(template.id);
      setDraft(next);
      setYamlText(templateDraftToYaml(next));
      setMode("form");
      setManagingInjections(false);
    } catch (error) {
      onError(errorMessage(error, t));
    }
  }

  function switchMode(next: "form" | "yaml") {
    try {
      if (next === "form" && mode === "yaml") setDraft(templateDraftFromYaml(yamlText));
      if (next === "yaml" && mode === "form") setYamlText(templateDraftToYaml(draft));
      setMode(next);
    } catch (error) {
      onError(errorMessage(error, t));
    }
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    let candidate: TemplateDraft;
    let yaml: string;
    try {
      candidate = mode === "yaml" ? templateDraftFromYaml(yamlText) : draft;
      yaml = templateDraftToYaml(candidate);
    } catch (error) {
      onError(errorMessage(error, t));
      return;
    }
    if (candidate.clusterAccess) {
      setPendingSave({ candidate, yaml });
      return;
    }
    await persist(candidate, yaml);
  }

  async function persist(candidate: TemplateDraft, yaml: string) {
    setSaving(true);
    try {
      const saved = selectedId ? await api.replaceTemplate(selectedId, yaml) : await api.createTemplate({ organization_id: organizationId, yaml });
      const savedDraft = templateDraftFromTemplate(saved);
      setSelectedId(saved.id);
      setDraft(savedDraft);
      setYamlText(templateDraftToYaml(savedDraft));
      setPendingSave(null);
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }

  async function toggle(template: WorkspaceTemplate) {
    if (template.enabled) {
      setPendingAction({ kind: "disable", template });
      return;
    }
    await setTemplateEnabled(template, true);
  }

  async function setTemplateEnabled(template: WorkspaceTemplate, enabled: boolean) {
    setSaving(true);
    try {
      await api.setTemplateEnabled(template.id, enabled);
      setPendingAction(null);
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }

  async function remove(template: WorkspaceTemplate) {
    setSaving(true);
    try {
      await api.deleteTemplate(template.id);
      startNew();
      setPendingAction(null);
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }

  return <div className={styles.stack}>
    <AdminToolbar action={<Button icon={<AddRegular />} onClick={startNew}>{t("newTemplate")}</Button>}>
      <div className={styles.stack}>
        <Text weight="semibold" size={500}>{t("templates")}</Text>
        <Text size={300} className={styles.muted}>{selected ? `${t("editingTemplate")} · ${selected.name}` : t("newTemplate")}</Text>
      </div>
    </AdminToolbar>
    <div className={styles.formGrid}>
      <Card appearance="outline" className={styles.list}>
        {templates.map((template) => <Button
          key={template.id}
          appearance={selectedId === template.id ? "primary" : "subtle"}
          aria-pressed={selectedId === template.id}
          className={styles.listButton}
          onClick={() => selectedId === template.id ? startNew() : selectTemplate(template)}
        >
          <span className={styles.stack}><Text weight="semibold">{template.name}</Text><Text className={styles.code} size={200}>{template.image}</Text></span>
          <Text size={200}>{template.enabled ? t("enabled") : t("disabled")}</Text>
        </Button>)}
        {templates.length === 0 && <Text className={styles.empty}>{t("noTemplates")}</Text>}
      </Card>
      <Card appearance="outline" className={styles.card}>
        <form className={styles.stack} onSubmit={(event) => void save(event)}>
          <TabList selectedValue={mode} onTabSelect={(_, data) => switchMode(data.value as "form" | "yaml")}>
            <Tab value="form" icon={<EditRegular />}>{t("formMode")}</Tab>
            <Tab value="yaml">{t("yamlMode")}</Tab>
          </TabList>
          {mode === "yaml" ? <Field label={t("templateYaml")} hint={t("templateYamlHelp")}>
            <Textarea className={styles.yaml} spellCheck={false} value={yamlText} onChange={(event) => setYamlText(event.target.value)} />
          </Field> : <TemplateForm draft={draft} setDraft={setDraft} canGrantClusterAccess={canGrantClusterAccess} nodePools={nodePools} nodePoolsError={nodePoolsError} saving={saving} t={t} styles={styles} />}
          <AdminToolbar action={<div className={styles.actions}>
            <Button ref={manageButtonRef} type="button" disabled={!selected || saving} onClick={() => setManagingInjections(true)}>{t("manageTemplateInjections")}</Button>
            <SaveButton type="submit" icon={<SaveRegular />} disabled={saving}>{saving ? t("saving") : selectedId ? t("saveChanges") : t("createTemplate")}</SaveButton>
            {selected && <Button type="button" appearance="secondary" disabled={saving} onClick={() => void toggle(selected)}>{selected.enabled ? t("disable") : t("enable")}</Button>}
            {selected && !selected.enabled && <Button type="button" appearance="subtle" icon={<DeleteRegular />} disabled={saving} onClick={() => setPendingAction({ kind: "delete", template: selected })}>{t("deleteTemplate")}</Button>}
          </div>}>
            {!selected && <Text size={200} className={styles.muted}>{t("saveTemplateBeforeInjections")}</Text>}
          </AdminToolbar>
        </form>
      </Card>
    </div>
    {selected && <TemplateInjectionsDialog api={api} organizationId={organizationId} template={selected} open={managingInjections} returnFocusRef={manageButtonRef} onClose={() => setManagingInjections(false)} onError={onError} />}
    <ConfirmDialog open={pendingSave !== null} title={selectedId ? t("saveChanges") : t("createTemplate")} description={t("templateHighRiskConfirm")} confirmLabel={selectedId ? t("saveChanges") : t("createTemplate")} cancelLabel={t("cancel")} busy={saving} danger details={<strong>{pendingSave?.candidate.name}</strong>} onClose={() => setPendingSave(null)} onConfirm={() => pendingSave && void persist(pendingSave.candidate, pendingSave.yaml)} />
    <ConfirmDialog open={pendingAction !== null} title={pendingAction?.kind === "delete" ? t("deleteTemplate") : t("disable")} description={pendingAction?.kind === "delete" ? t("deleteTemplateConfirm") : t("disableTemplateConfirm")} confirmLabel={pendingAction?.kind === "delete" ? t("deleteTemplate") : t("disable")} cancelLabel={t("cancel")} busy={saving} danger details={<strong>{pendingAction?.template.name}</strong>} onClose={() => setPendingAction(null)} onConfirm={() => pendingAction && void (pendingAction.kind === "delete" ? remove(pendingAction.template) : setTemplateEnabled(pendingAction.template, false))} />
  </div>;
}

function errorMessage(error: unknown, t: ReturnType<typeof useI18n>["t"]) {
  if (error instanceof TemplateDraftError) {
    if (error.code === "resource_request_exceeds_limit") return t("resourceRequestExceedsLimit");
    if (error.code === "invalid_node_pool_placement") return t("invalidNodePoolPlacement");
    return t("invalidTemplateNumber");
  }
  return error instanceof Error ? error.message : t("requestFailed");
}
