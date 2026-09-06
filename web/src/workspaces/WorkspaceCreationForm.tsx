import type { FormEvent } from "react";
import { CredentialReferencePicker } from "../forms/CredentialReferencePicker";
import { useI18n } from "../i18n";
import type { Resources, StoredInjection, WorkspaceTemplate } from "../types";

interface Props {
  name: string;
  templateId: string;
  templates: WorkspaceTemplate[];
  resourceDraft: Resources | null;
  explicitInjectionRefs: boolean;
  organizationInjections: StoredInjection[];
  userInjections: StoredInjection[];
  organizationRefs: string[];
  userRefs: string[];
  submitting: boolean;
  onNameChange: (value: string) => void;
  onTemplateChange: (id: string) => void;
  onResourceChange: (key: keyof Resources, value: string) => void;
  onReferenceModeChange: (explicit: boolean) => void;
  onOrganizationRefsChange: (keys: string[]) => void;
  onUserRefsChange: (keys: string[]) => void;
  onSubmit: (event: FormEvent) => void;
}

export function WorkspaceCreationForm(props: Props) {
  const { t } = useI18n();
  const selectedTemplate = props.templates.find((template) => template.id === props.templateId);
  return <form className="create-card workspace-create-card" onSubmit={props.onSubmit}>
    <label>{t("name")}<input required value={props.name} onChange={(event) => props.onNameChange(event.target.value)} /></label>
    <label><FieldTitle label={t("template")} help={t("templatePersistenceHelp")} /><select required value={props.templateId} onChange={(event) => props.onTemplateChange(event.target.value)}><option value="">{t("chooseTemplate")}</option>{props.templates.map((template) => <option key={template.id} value={template.id}>{template.name}</option>)}</select></label>
    {selectedTemplate && <TemplateSummary template={selectedTemplate} />}
    {selectedTemplate && props.resourceDraft && <ResourceEditor template={selectedTemplate} resources={props.resourceDraft} onChange={props.onResourceChange} />}
    <label>{t("injectionReferences")}<select value={props.explicitInjectionRefs ? "selected" : "all"} onChange={(event) => props.onReferenceModeChange(event.target.value === "selected")}><option value="all">{t("allMatching")}</option><option value="selected">{t("selectedReferences")}</option></select></label>
    {props.explicitInjectionRefs && <CredentialReferencePicker organizationItems={props.organizationInjections} userItems={props.userInjections} organizationSelected={props.organizationRefs} userSelected={props.userRefs} onOrganizationSelected={props.onOrganizationRefsChange} onUserSelected={props.onUserRefsChange} />}
    <div className="form-actions"><button className="button primary" disabled={props.submitting || !props.templateId}>{props.submitting ? t("creating") : t("submitCreate")}</button></div>
  </form>;
}

function TemplateSummary({ template }: { template: WorkspaceTemplate }) {
  const { t } = useI18n();
  return <dl className="template-summary wide">
    <div><dt>{t("image")}</dt><dd><code>{template.image}</code></dd></div>
    <div><dt><FieldTitle label={t("accessMode")} help={template.access_mode === "internal" ? t("internalHelp") : t("publicHelp")} /></dt><dd>{template.access_mode === "internal" ? t("internal") : t("public")}</dd></div>
    <div><dt>{t("resources")}</dt><dd>{template.resources.cpu_millis}m CPU · {template.resources.memory_mib} MiB · {template.resources.disk_gib} GiB · {template.resources.gpu_count} GPU</dd></div>
    <div><dt>{t("workspaceUser")}</dt><dd><code>{template.workspace_user} · {template.workspace_home}</code></dd></div>
  </dl>;
}

function ResourceEditor({ template, resources, onChange }: { template: WorkspaceTemplate; resources: Resources; onChange: (key: keyof Resources, value: string) => void }) {
  const { t } = useI18n();
  return <fieldset className="workspace-resource-editor wide"><legend><FieldTitle label={t("workspaceResources")} help={t("workspaceResourcesHelp")} /></legend><div className="workspace-resource-fields">
    <label>{t("cpuLimitMillis")}<input type="number" min={template.pod_requests.cpu_millis} step="100" required value={resources.cpu_millis} onChange={(event) => onChange("cpu_millis", event.target.value)} /></label>
    <label>{t("memoryLimitMib")}<input type="number" min={template.pod_requests.memory_mib} step="256" required value={resources.memory_mib} onChange={(event) => onChange("memory_mib", event.target.value)} /></label>
    <label>{t("diskSizeGib")}<input type="number" min="1" step="1" required value={resources.disk_gib} onChange={(event) => onChange("disk_gib", event.target.value)} /></label>
    <label>{t("gpuCount")}<input type="number" min="0" step="1" required value={resources.gpu_count} onChange={(event) => onChange("gpu_count", event.target.value)} /></label>
  </div></fieldset>;
}

function FieldTitle({ label, help }: { label: string; help: string }) {
  return <span className="field-title"><span>{label}</span><span className="help-tip" title={help} aria-label={help} tabIndex={0}>?</span></span>;
}
