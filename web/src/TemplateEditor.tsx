import { useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";

import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import { TemplateInjectionsDialog } from "./injections/TemplateInjectionsDialog";
import {
  TEMPLATE_NUMBER_POLICIES,
  TemplateDraftError,
  emptyTemplateDraft,
  templateDraftFromTemplate,
  templateDraftFromYaml,
  templateDraftToYaml,
} from "./templates/templateDraft";
import type { NumericPolicy, TemplateDraft } from "./templates/templateDraft";
import type { AccessMode, WorkspaceTemplate } from "./types";

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
  const manageButtonRef = useRef<HTMLButtonElement>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<"form" | "yaml">("form");
  const [draft, setDraft] = useState<TemplateDraft>(emptyTemplateDraft);
  const [yamlText, setYamlText] = useState(() => templateDraftToYaml(emptyTemplateDraft()));
  const [saving, setSaving] = useState(false);
  const [managingInjections, setManagingInjections] = useState(false);
  const selected = useMemo(() => templates.find((item) => item.id === selectedId) ?? null, [templates, selectedId]);

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
    if (candidate.clusterAccess && !confirm(t("templateHighRiskConfirm"))) return;
    setSaving(true);
    try {
      const saved = selectedId
        ? await api.replaceTemplate(selectedId, yaml)
        : await api.createTemplate({ organization_id: organizationId, yaml });
      const savedDraft = templateDraftFromTemplate(saved);
      setSelectedId(saved.id);
      setDraft(savedDraft);
      setYamlText(templateDraftToYaml(savedDraft));
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }

  async function toggle(template: WorkspaceTemplate) {
    if (template.enabled && !confirm(`${template.name}: ${t("disableTemplateConfirm")}`)) return;
    try {
      await api.setTemplateEnabled(template.id, !template.enabled);
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    }
  }

  async function remove(template: WorkspaceTemplate) {
    if (!confirm(`${template.name}: ${t("deleteTemplateConfirm")}`)) return;
    setSaving(true);
    try {
      await api.deleteTemplate(template.id);
      startNew();
      await onRefresh();
    } catch (error) {
      onError(errorMessage(error, t));
    } finally {
      setSaving(false);
    }
  }

  return <div className="template-manager">
    <div className="template-manager-toolbar">
      <div><h3>{t("templates")}</h3><small>{selected ? `${t("editingTemplate")} · ${selected.name}` : t("newTemplate")}</small></div>
      <button type="button" className="button" onClick={startNew}>{t("newTemplate")}</button>
    </div>
    <div className="template-manager-layout">
      <div className="template-list selectable-list" role="listbox" aria-label={t("templates")}>
        {templates.map((template) => <button type="button" role="option" aria-selected={selectedId === template.id} className={selectedId === template.id ? "selected" : ""} key={template.id} onClick={() => selectTemplate(template)}>
          <span><strong>{template.name}</strong><small>{t("workspaceUser")}: <code>{template.workspace_user}</code> · {template.access_mode === "internal" ? t("internal") : t("public")}</small></span>
          <span className={template.enabled ? "healthy" : "muted"}>{template.enabled ? t("enabled") : t("disabled")}</span>
        </button>)}
        {templates.length === 0 && <p>{t("noTemplates")}</p>}
      </div>
      <form className="template-editor" onSubmit={save}>
        <div className="editor-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={mode === "form"} className={mode === "form" ? "active" : ""} onClick={() => switchMode("form")}>{t("formMode")}</button>
          <button type="button" role="tab" aria-selected={mode === "yaml"} className={mode === "yaml" ? "active" : ""} onClick={() => switchMode("yaml")}>{t("yamlMode")}</button>
        </div>
        {mode === "yaml" ? (
          <label className="yaml-editor"><Field label={t("templateYaml")} help={t("templateYamlHelp")} /><textarea spellCheck={false} value={yamlText} onChange={(event) => setYamlText(event.target.value)} /></label>
        ) : (
          <div className="template-form">
            <label><Field label={t("templateName")} help={t("templateNameHelp")} /><input required value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></label>
            <label className="wide"><Field label={t("allowedOciImage")} help={t("templateImageHelp")} /><input required value={draft.image} onChange={(event) => setDraft({ ...draft, image: event.target.value })} placeholder="registry/image@sha256:…" /></label>
            <label><Field label={t("accessMode")} help={draft.accessMode === "internal" ? t("internalHelp") : t("publicHelp")} /><select value={draft.accessMode} onChange={(event) => setDraft({ ...draft, accessMode: event.target.value as AccessMode })}><option value="internal">{t("internal")}</option><option value="public">{t("public")}</option></select></label>
            <label><Field label={t("workspaceUser")} help={t("workspaceUserHelp")} /><input required value={draft.user} onChange={(event) => setDraft({ ...draft, user: event.target.value })} placeholder="workspace" /></label>
            <label className="wide"><Field label={t("workspaceHome")} help={t("workspaceHomeHelp")} /><input required value={draft.home} onChange={(event) => setDraft({ ...draft, home: event.target.value })} placeholder="/workspace" /></label>
            <NumberField label={`${t("cpuLimit")} (m)`} value={draft.cpu} policy={TEMPLATE_NUMBER_POLICIES.cpu} update={(cpu) => setDraft({ ...draft, cpu })} />
            <NumberField label={`${t("memoryLimit")} (MiB)`} value={draft.memory} policy={TEMPLATE_NUMBER_POLICIES.memory} update={(memory) => setDraft({ ...draft, memory })} />
            <NumberField label={`${t("cpuRequest")} (m)`} value={draft.requestCpu} policy={TEMPLATE_NUMBER_POLICIES.requestCpu} update={(requestCpu) => setDraft({ ...draft, requestCpu })} />
            <NumberField label={`${t("memoryRequest")} (MiB)`} value={draft.requestMemory} policy={TEMPLATE_NUMBER_POLICIES.requestMemory} update={(requestMemory) => setDraft({ ...draft, requestMemory })} />
            <NumberField label="GPU" value={draft.gpu} policy={TEMPLATE_NUMBER_POLICIES.gpu} update={(gpu) => setDraft({ ...draft, gpu })} />
            <NumberField label={`${t("disk")} (GiB)`} value={draft.disk} policy={TEMPLATE_NUMBER_POLICIES.disk} update={(disk) => setDraft({ ...draft, disk })} />
            <NumberField optional label={`${t("ephemeralRequest")} (MiB)`} help={t("ephemeralHelp")} value={draft.requestEphemeral} policy={TEMPLATE_NUMBER_POLICIES.ephemeral} update={(requestEphemeral) => setDraft({ ...draft, requestEphemeral })} />
            <NumberField optional label={`${t("ephemeralLimit")} (MiB)`} help={t("ephemeralHelp")} value={draft.limitEphemeral} policy={TEMPLATE_NUMBER_POLICIES.ephemeral} update={(limitEphemeral) => setDraft({ ...draft, limitEphemeral })} />
            <Check label={t("preserveHomeOwnership")} help={t("preserveHomeOwnershipHelp")} checked={draft.preserveHome} update={(preserveHome) => setDraft({ ...draft, preserveHome })} />
            <Check label="BuildKit" help={t("buildkitHelp")} checked={draft.buildkit} update={(buildkit) => setDraft({ ...draft, buildkit })} />
            <Check label={t("maintenanceAccess")} help={t("maintenanceAccessHelp")} checked={draft.clusterAccess} disabled={!canGrantClusterAccess} update={(clusterAccess) => setDraft({ ...draft, clusterAccess })} />
            <label className="wide"><Field label={t("requiredNodes")} help={t("nodeListHelp")} /><input value={draft.requiredNodes} onChange={(event) => setDraft({ ...draft, requiredNodes: event.target.value })} placeholder="westlake, haixia" /></label>
            <label className="wide"><Field label={t("preferredNodes")} help={t("nodeListHelp")} /><input value={draft.preferredNodes} onChange={(event) => setDraft({ ...draft, preferredNodes: event.target.value })} /></label>
            <label className="wide"><Field label={t("nodeSelector")} help={t("keyValueLinesHelp")} /><textarea value={draft.nodeSelector} onChange={(event) => setDraft({ ...draft, nodeSelector: event.target.value })} placeholder="k3s-worker-ready=true" /></label>
          </div>
        )}
        <div className="template-injection-actions">
          <button ref={manageButtonRef} type="button" className="button" aria-haspopup="dialog" aria-describedby={!selected ? "template-injections-disabled-help" : undefined} disabled={!selected || saving} onClick={() => setManagingInjections(true)}>{t("manageTemplateEnvironmentFiles")}</button>
          {!selected && <small id="template-injections-disabled-help">{t("saveTemplateBeforeEnvironmentFiles")}</small>}
        </div>
        <div className="form-actions">
          <button className="button primary" disabled={saving}>{saving ? t("saving") : selectedId ? t("saveChanges") : t("createTemplate")}</button>
          {selected && <button type="button" className={selected.enabled ? "button danger" : "button"} disabled={saving} onClick={() => void toggle(selected)}>{selected.enabled ? t("disable") : t("enable")}</button>}
          {selected && !selected.enabled && <button type="button" className="button danger" disabled={saving} onClick={() => void remove(selected)}>{t("deleteTemplate")}</button>}
        </div>
      </form>
    </div>
    {selected && <TemplateInjectionsDialog api={api} organizationId={organizationId} template={selected} open={managingInjections} returnFocusRef={manageButtonRef} onClose={() => setManagingInjections(false)} onError={onError} />}
  </div>;
}

function NumberField({ label, help, value, policy, optional = false, update }: { label: string; help?: string; value: string; policy: NumericPolicy; optional?: boolean; update: (value: string) => void }) {
  return <label><Field label={label} help={help} /><input required={!optional} type="number" inputMode="numeric" min={policy.min} step={policy.step} max={policy.max} value={value} onChange={(event) => update(event.target.value)} /></label>;
}

function Field({ label, help }: { label: string; help?: string }) {
  return <span className="field-title"><span>{label}</span>{help && <span className="help-tip" title={help} aria-label={help} tabIndex={0}>?</span>}</span>;
}

function Check({ label, help, checked, disabled = false, update }: { label: string; help: string; checked: boolean; disabled?: boolean; update: (value: boolean) => void }) {
  return <label className="check-field"><input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => update(event.target.checked)} /><Field label={label} help={help} /></label>;
}

function errorMessage(error: unknown, t: ReturnType<typeof useI18n>["t"]) {
  if (error instanceof TemplateDraftError) {
    if (error.code === "resource_request_exceeds_limit") return t("resourceRequestExceedsLimit");
    if (error.code === "ephemeral_request_exceeds_limit") return t("ephemeralRequestExceedsLimit");
    return t("invalidTemplateNumber");
  }
  return error instanceof Error ? error.message : "Request failed";
}
