import { useMemo, useState } from "react";
import type { FormEvent } from "react";
import { parse, stringify } from "yaml";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type { AccessMode, WorkspaceTemplate } from "./types";

interface Props {
  api: ApiClient;
  organizationId: string;
  templates: WorkspaceTemplate[];
  canGrantClusterAccess: boolean;
  onRefresh: () => Promise<void>;
  onError: (message: string) => void;
}

interface Draft {
  name: string;
  image: string;
  accessMode: AccessMode;
  cpu: number;
  memory: number;
  gpu: number;
  disk: number;
  requestCpu: number;
  requestMemory: number;
  requestEphemeral: string;
  limitEphemeral: string;
  user: string;
  home: string;
  preserveHome: boolean;
  buildkit: boolean;
  clusterAccess: boolean;
  requiredNodes: string;
  preferredNodes: string;
  nodeSelector: string;
  environment: string;
}

const EMPTY: Draft = {
  name: "", image: "", accessMode: "internal",
  cpu: 2_000, memory: 4_096, gpu: 0, disk: 50,
  requestCpu: 500, requestMemory: 1_024, requestEphemeral: "", limitEphemeral: "",
  user: "workspace", home: "/workspace", preserveHome: false,
  buildkit: false, clusterAccess: false,
  requiredNodes: "", preferredNodes: "", nodeSelector: "", environment: "",
};

export function TemplateEditor({ api, organizationId, templates, canGrantClusterAccess, onRefresh, onError }: Props) {
  const { t } = useI18n();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [mode, setMode] = useState<"form" | "yaml">("form");
  const [draft, setDraft] = useState<Draft>(EMPTY);
  const [yamlText, setYamlText] = useState(toYaml(EMPTY));
  const [saving, setSaving] = useState(false);
  const selected = useMemo(() => templates.find((item) => item.id === selectedId) ?? null, [templates, selectedId]);

  function startNew() {
    setSelectedId(null);
    setDraft(EMPTY);
    setYamlText(toYaml(EMPTY));
    setMode("form");
  }

  function selectTemplate(template: WorkspaceTemplate) {
    const next = fromTemplate(template);
    setSelectedId(template.id);
    setDraft(next);
    setYamlText(template.yaml);
    setMode("form");
  }

  function change(next: Draft) {
    setDraft(next);
    setYamlText(toYaml(next));
  }

  function switchMode(next: "form" | "yaml") {
    if (next === "form" && mode === "yaml") {
      try { setDraft(fromYaml(yamlText)); } catch (error) { onError(message(error)); return; }
    }
    if (next === "yaml" && mode === "form") setYamlText(toYaml(draft));
    setMode(next);
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    const yaml = mode === "yaml" ? yamlText : toYaml(draft);
    let candidate: Draft;
    try { candidate = fromYaml(yaml); } catch (error) { onError(message(error)); return; }
    if (candidate.clusterAccess && !confirm(t("templateHighRiskConfirm"))) return;
    setSaving(true);
    try {
      const saved = selectedId
        ? await api.replaceTemplate(selectedId, yaml)
        : await api.createTemplate({ organization_id: organizationId, yaml });
      setSelectedId(saved.id);
      setDraft(fromTemplate(saved));
      setYamlText(saved.yaml);
      await onRefresh();
    } catch (error) { onError(message(error)); }
    finally { setSaving(false); }
  }

  async function toggle(template: WorkspaceTemplate) {
    if (template.enabled && !confirm(`${template.name}: ${t("disableTemplateConfirm")}`)) return;
    try { await api.setTemplateEnabled(template.id, !template.enabled); await onRefresh(); }
    catch (error) { onError(message(error)); }
  }

  return <div className="template-manager">
    <div className="template-manager-toolbar">
      <div><h3>{t("templates")}</h3><small>{selected ? `${t("editingTemplate")} · ${selected.name}` : t("newTemplate")}</small></div>
      <button type="button" className="button" onClick={startNew}>{t("newTemplate")}</button>
    </div>
    <div className="template-manager-layout">
      <div className="template-list selectable-list" role="listbox" aria-label={t("templates")}>
        {templates.map((template) => <button type="button" role="option" aria-selected={selectedId === template.id} className={selectedId === template.id ? "selected" : ""} key={template.id} onClick={() => selectTemplate(template)}>
          <span><strong>{template.name}</strong><small>{template.workspace_user} · {template.access_mode === "internal" ? t("internal") : t("public")}</small></span>
          <span className={template.enabled ? "healthy" : "muted"}>{template.enabled ? t("enabled") : t("disabled")}</span>
        </button>)}
        {templates.length === 0 && <p>{t("noTemplates")}</p>}
      </div>
      <form className="template-editor" onSubmit={save}>
        <div className="editor-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={mode === "form"} className={mode === "form" ? "active" : ""} onClick={() => switchMode("form")}>{t("formMode")}</button>
          <button type="button" role="tab" aria-selected={mode === "yaml"} className={mode === "yaml" ? "active" : ""} onClick={() => switchMode("yaml")}>{t("yamlMode")}</button>
        </div>
        {mode === "yaml" ? <label className="yaml-editor"><Field label={t("templateYaml")} help={t("templateYamlHelp")} /><textarea spellCheck={false} value={yamlText} onChange={(event) => setYamlText(event.target.value)} /></label> : <div className="template-form">
          <label><Field label={t("templateName")} help={t("templateNameHelp")} /><input required value={draft.name} onChange={(event) => change({ ...draft, name: event.target.value })} /></label>
          <label className="wide"><Field label={t("allowedOciImage")} help={t("templateImageHelp")} /><input required value={draft.image} onChange={(event) => change({ ...draft, image: event.target.value })} placeholder="registry/image@sha256:…" /></label>
          <label><Field label={t("accessMode")} help={draft.accessMode === "internal" ? t("internalHelp") : t("publicHelp")} /><select value={draft.accessMode} onChange={(event) => change({ ...draft, accessMode: event.target.value as AccessMode })}><option value="internal">{t("internal")}</option><option value="public">{t("public")}</option></select></label>
          <label><Field label={t("workspaceUser")} help={t("workspaceUserHelp")} /><input required value={draft.user} onChange={(event) => change({ ...draft, user: event.target.value })} placeholder="workspace" /></label>
          <label className="wide"><Field label={t("workspaceHome")} help={t("workspaceHomeHelp")} /><input required value={draft.home} onChange={(event) => change({ ...draft, home: event.target.value })} placeholder="/workspace" /></label>
          <NumberField label={`${t("cpuLimit")} (m)`} value={draft.cpu} min={100} update={(cpu) => change({ ...draft, cpu })} />
          <NumberField label={`${t("memoryLimit")} (MiB)`} value={draft.memory} min={128} update={(memory) => change({ ...draft, memory })} />
          <NumberField label={`${t("cpuRequest")} (m)`} value={draft.requestCpu} min={1} update={(requestCpu) => change({ ...draft, requestCpu })} />
          <NumberField label={`${t("memoryRequest")} (MiB)`} value={draft.requestMemory} min={1} update={(requestMemory) => change({ ...draft, requestMemory })} />
          <NumberField label="GPU" value={draft.gpu} min={0} update={(gpu) => change({ ...draft, gpu })} />
          <NumberField label={`${t("disk")} (GiB)`} value={draft.disk} min={1} update={(disk) => change({ ...draft, disk })} />
          <label><Field label={`${t("ephemeralRequest")} (MiB)`} help={t("ephemeralHelp")} /><input type="number" min="1" value={draft.requestEphemeral} onChange={(event) => change({ ...draft, requestEphemeral: event.target.value })} /></label>
          <label><Field label={`${t("ephemeralLimit")} (MiB)`} help={t("ephemeralHelp")} /><input type="number" min="1" value={draft.limitEphemeral} onChange={(event) => change({ ...draft, limitEphemeral: event.target.value })} /></label>
          <Check label={t("preserveHomeRoot")} help={t("preserveHomeRootHelp")} checked={draft.preserveHome} update={(preserveHome) => change({ ...draft, preserveHome })} />
          <Check label="BuildKit" help={t("buildkitHelp")} checked={draft.buildkit} update={(buildkit) => change({ ...draft, buildkit })} />
          <Check label={t("maintenanceAccess")} help={t("maintenanceAccessHelp")} checked={draft.clusterAccess} disabled={!canGrantClusterAccess} update={(clusterAccess) => change({ ...draft, clusterAccess })} />
          <label className="wide"><Field label={t("requiredNodes")} help={t("nodeListHelp")} /><input value={draft.requiredNodes} onChange={(event) => change({ ...draft, requiredNodes: event.target.value })} placeholder="westlake, haixia" /></label>
          <label className="wide"><Field label={t("preferredNodes")} help={t("nodeListHelp")} /><input value={draft.preferredNodes} onChange={(event) => change({ ...draft, preferredNodes: event.target.value })} /></label>
          <label className="wide"><Field label={t("nodeSelector")} help={t("keyValueLinesHelp")} /><textarea value={draft.nodeSelector} onChange={(event) => change({ ...draft, nodeSelector: event.target.value })} placeholder="k3s-worker-ready=true" /></label>
          <label className="wide"><Field label={t("environmentVariables")} help={t("keyValueLinesHelp")} /><textarea value={draft.environment} onChange={(event) => change({ ...draft, environment: event.target.value })} placeholder="HOME=/workspace" /></label>
        </div>}
        <div className="form-actions"><button className="button primary" disabled={saving}>{saving ? t("saving") : selectedId ? t("saveChanges") : t("createTemplate")}</button>{selected && <button type="button" className={selected.enabled ? "button danger" : "button"} onClick={() => void toggle(selected)}>{selected.enabled ? t("disable") : t("enable")}</button>}</div>
      </form>
    </div>
  </div>;
}

function toYaml(draft: Draft): string {
  const spec: Record<string, unknown> = {
    image: draft.image,
    access_mode: draft.accessMode,
    resources: { cpu_millis: draft.cpu, memory_mib: draft.memory, gpu_count: draft.gpu, disk_gib: draft.disk },
    pod_requests: { cpu_millis: draft.requestCpu, memory_mib: draft.requestMemory },
    workspace_user: draft.user,
    workspace_home: draft.home,
    preserve_home_root: draft.preserveHome,
    buildkit: draft.buildkit,
    cluster_access: draft.clusterAccess,
  };
  if (draft.requestEphemeral) (spec.pod_requests as Record<string, unknown>).ephemeral_storage_mib = Number(draft.requestEphemeral);
  if (draft.limitEphemeral) spec.ephemeral_storage_limit_mib = Number(draft.limitEphemeral);
  const required = csv(draft.requiredNodes); if (required.length) spec.required_node_names = required;
  const preferred = csv(draft.preferredNodes); if (preferred.length) spec.preferred_node_names = preferred;
  const selector = pairs(draft.nodeSelector); if (Object.keys(selector).length) spec.node_selector = selector;
  const environment = pairs(draft.environment); if (Object.keys(environment).length) spec.environment = environment;
  return stringify({ apiVersion: "workspace.memeloop.dev/v1", kind: "WorkspaceTemplate", metadata: { name: draft.name }, spec }, { lineWidth: 0 });
}

function fromYaml(yaml: string): Draft {
  const value = parse(yaml) as Record<string, any>;
  if (!value?.metadata?.name || !value?.spec) throw new Error("Invalid WorkspaceTemplate YAML");
  const spec = value.spec;
  return {
    name: String(value.metadata.name), image: String(spec.image ?? ""), accessMode: spec.access_mode === "public" ? "public" : "internal",
    cpu: Number(spec.resources?.cpu_millis ?? 0), memory: Number(spec.resources?.memory_mib ?? 0), gpu: Number(spec.resources?.gpu_count ?? 0), disk: Number(spec.resources?.disk_gib ?? 0),
    requestCpu: Number(spec.pod_requests?.cpu_millis ?? 0), requestMemory: Number(spec.pod_requests?.memory_mib ?? 0),
    requestEphemeral: nullableNumber(spec.pod_requests?.ephemeral_storage_mib), limitEphemeral: nullableNumber(spec.ephemeral_storage_limit_mib),
    user: String(spec.workspace_user ?? ""), home: String(spec.workspace_home ?? ""), preserveHome: Boolean(spec.preserve_home_root),
    buildkit: Boolean(spec.buildkit), clusterAccess: Boolean(spec.cluster_access),
    requiredNodes: (spec.required_node_names ?? []).join(", "), preferredNodes: (spec.preferred_node_names ?? []).join(", "),
    nodeSelector: formatPairs(spec.node_selector), environment: formatPairs(spec.environment),
  };
}

function fromTemplate(template: WorkspaceTemplate): Draft {
  return fromYaml(template.yaml);
}

function csv(value: string) { return value.split(",").map((item) => item.trim()).filter(Boolean); }
function pairs(value: string) { return Object.fromEntries(value.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => { const index = line.indexOf("="); if (index < 1) throw new Error(`Expected KEY=value: ${line}`); return [line.slice(0, index).trim(), line.slice(index + 1)]; })); }
function formatPairs(value: unknown) { return value && typeof value === "object" ? Object.entries(value as Record<string, unknown>).map(([key, item]) => `${key}=${String(item)}`).join("\n") : ""; }
function nullableNumber(value: unknown) { return value === undefined || value === null ? "" : String(value); }
function message(error: unknown) { return error instanceof Error ? error.message : "Request failed"; }

function Field({ label, help }: { label: string; help?: string }) { return <span className="field-title"><span>{label}</span>{help && <span className="help-tip" title={help} aria-label={help} tabIndex={0}>?</span>}</span>; }
function NumberField({ label, value, min, update }: { label: string; value: number; min: number; update: (value: number) => void }) { return <label>{label}<input required type="number" min={min} value={value} onChange={(event) => update(Number(event.target.value))} /></label>; }
function Check({ label, help, checked, disabled = false, update }: { label: string; help: string; checked: boolean; disabled?: boolean; update: (value: boolean) => void }) { return <label className="check-field"><input type="checkbox" checked={checked} disabled={disabled} onChange={(event) => update(event.target.checked)} /><Field label={label} help={help} /></label>; }
