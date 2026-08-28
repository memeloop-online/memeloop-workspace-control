import { useEffect, useMemo, useState } from "react";
import type { Dispatch, FormEvent, SetStateAction } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type { MessageKey } from "./i18n";
import type {
  InjectionDraft,
  InjectionKind,
  InjectionScope,
  Principal,
  ResolvedInjection,
  StoredInjection,
  WorkspaceResponse,
  WorkspaceTemplate,
} from "./types";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  workspaces: WorkspaceResponse[];
  onError: (message: string) => void;
}

const EMPTY_DRAFT: InjectionDraft = {
  key: "",
  kind: "config_file",
  target: "/workspace/config.yaml",
  value: { encoding: "utf8", value: "" },
  sensitive: false,
  locked: false,
  version: 0,
  file_mode: 420,
  owner: null,
  group: null,
  template_selector: null,
  labels: {},
};

export function InjectionPanel(props: Props) {
  const { t } = useI18n();
  const canManageOrganization = props.principal.system_admin || props.principal.memberships.some((membership) => membership.organization_id === props.organizationId && membership.role === "organization_admin");
  const [scope, setScope] = useState<InjectionScope>("user");
  const [workspaceId, setWorkspaceId] = useState(props.workspaces[0]?.workspace.id ?? "");
  const [workspaceQuery, setWorkspaceQuery] = useState(props.workspaces[0] ? workspaceLabel(props.workspaces[0]) : "");
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [draft, setDraft] = useState<InjectionDraft>({ ...EMPTY_DRAFT });
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [preview, setPreview] = useState<ResolvedInjection[]>([]);
  const [saving, setSaving] = useState(false);
  const [search, setSearch] = useState("");

  const filteredItems = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    if (!query) return items;
    return items.filter((item) => [item.key, item.target, item.kind, kindLabel(item.kind, t)]
      .some((value) => value.toLocaleLowerCase().includes(query)));
  }, [items, search, t]);

  const scopeId = useMemo(() => {
    if (scope === "organization") return props.organizationId;
    if (scope === "user") return props.principal.user_id;
    return workspaceId;
  }, [scope, workspaceId, props.organizationId, props.principal.user_id]);

  async function load() {
    if (!scopeId) {
      setItems([]);
      return;
    }
    try {
      setItems(await props.api.injections(scope, scopeId));
    } catch (error) {
      props.onError(message(error));
    }
  }

  useEffect(() => {
    void load();
  }, [scopeId]);

  useEffect(() => {
    let active = true;
    props.api.templates(props.organizationId)
      .then((value) => { if (active) setTemplates(value.filter((item) => item.enabled)); })
      .catch((error) => props.onError(message(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId]);

  useEffect(() => {
    if (!workspaceId && props.workspaces[0]) {
      setWorkspaceId(props.workspaces[0].workspace.id);
      setWorkspaceQuery(workspaceLabel(props.workspaces[0]));
    }
  }, [props.workspaces, workspaceId]);

  function resetDraft() {
    setSelectedKey(null);
    setDraft({ ...EMPTY_DRAFT, value: { ...EMPTY_DRAFT.value }, labels: {} });
  }

  function changeScope(value: InjectionScope) {
    setScope(value);
    resetDraft();
    setPreview([]);
  }

  function selectItem(item: StoredInjection) {
    if (selectedKey === item.key) {
      resetDraft();
      return;
    }
    setSelectedKey(item.key);
    setDraft({
      ...EMPTY_DRAFT,
      key: item.key,
      kind: item.kind,
      target: item.target,
      sensitive: item.sensitive,
      locked: item.locked,
      version: item.version,
      file_mode: item.file_mode,
      owner: item.owner,
      group: item.group,
      template_selector: item.template_selector,
      labels: { ...item.labels },
      value: { encoding: "utf8", value: "" },
    });
  }

  function changeKind(kind: InjectionKind) {
    const target =
      kind === "environment_variable"
        ? "EXAMPLE_VARIABLE"
        : kind === "ssh_public_key"
          ? sshTarget(draft.key)
          : kind === "secret_file"
            ? "/run/secrets/example"
            : "/workspace/config.yaml";
    setDraft((value) => ({
      ...value,
      kind,
      target,
      sensitive: kind === "secret_file",
      file_mode: kind === "environment_variable" ? null : 420,
    }));
  }

  function changeKey(key: string) {
    setDraft((current) => ({
      ...current,
      key,
      target: current.kind === "ssh_public_key" ? sshTarget(key) : current.target,
    }));
  }

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!scopeId) return;
    setSaving(true);
    try {
      await props.api.replaceInjection(scope, scopeId, {
        ...draft,
        locked: scope === "organization" && draft.locked,
      });
      resetDraft();
      await load();
    } catch (error) {
      props.onError(message(error));
    } finally {
      setSaving(false);
    }
  }

  async function runPreview() {
    try {
      setPreview(
        await props.api.previewInjections({
          organization_id: props.organizationId,
          user_id: props.principal.user_id,
          workspace_id: workspaceId || null,
          inline_workspace_injections:
            draft.key && scope === "workspace" ? [draft] : [],
        }),
      );
    } catch (error) {
      props.onError(message(error));
    }
  }

  return (
    <section className="panel-stack">
      <div className="section-heading">
        <div><p className="eyebrow">CREDENTIALS</p><h2>{t("credentialsTitle")}</h2></div>
        <button className="button" onClick={() => void runPreview()}>{t("credentialsPreview")}</button>
      </div>
      <div className="scope-tabs">
        {([...(canManageOrganization ? ["organization" as const] : []), "user", "workspace"] as InjectionScope[]).map((value) => (
          <button className={scope === value ? "active" : ""} onClick={() => changeScope(value)} key={value}>
            {value === "organization" ? t("scopeOrganization") : value === "user" ? t("scopeUser") : t("scopeWorkspace")}
          </button>
        ))}
      </div>
      {scope === "workspace" && (
        <label className="standalone-label">{t("workspaces")}<input type="search" list="credential-workspaces" value={workspaceQuery} onChange={(event) => { const value = event.target.value; setWorkspaceQuery(value); const selected = props.workspaces.find((item) => workspaceLabel(item) === value || item.workspace.id === value); setWorkspaceId(selected?.workspace.id ?? ""); }} placeholder={t("workspaceAutocomplete")} autoComplete="off" /><datalist id="credential-workspaces">{props.workspaces.map((item) => <option value={workspaceLabel(item)} key={item.workspace.id} />)}</datalist></label>
      )}

      <div className="injection-layout">
        <div className="injection-list">
          <h3>{t("savedCredentials")}</h3>
          <input className="credential-search" type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchCredentials")} aria-label={t("searchCredentials")} />
          <div className="credential-scroll">
          {filteredItems.length === 0 && <div className="empty compact">{t("noCredentials")}</div>}
          {filteredItems.map((item) => (
            <button className={`injection-row${selectedKey === item.key ? " selected" : ""}`} aria-pressed={selectedKey === item.key} key={item.key} onClick={() => selectItem(item)}>
              <span className="kind-icon">{kindGlyph(item.kind)}</span>
              <span><strong>{item.key}</strong><small>{item.target}</small></span>
              <span className="version">v{item.version}{item.locked ? ` · ${t("locked")}` : ""}</span>
            </button>
          ))}
          </div>
        </div>

        <form className="editor-card" onSubmit={save}>
          <div className="editor-heading"><strong>{selectedKey ? t("editingCredential") : t("newCredential")}</strong>{selectedKey && <button type="button" className="text-button" onClick={resetDraft}>{t("cancel")}</button>}</div>
          <div className="editor-grid">
            <label><FieldTitle label={t("key")} help={t("keyHint")} /><input required readOnly={selectedKey !== null} value={draft.key} onChange={(e) => changeKey(e.target.value)} placeholder={t("keyHint")} /></label>
            <label><FieldTitle label={t("type")} /><select value={draft.kind} onChange={(e) => changeKind(e.target.value as InjectionKind)}><option value="environment_variable">{t("environmentVariable")}</option><option value="config_file">{t("configFile")}</option><option value="secret_file">{t("credentialFile")}</option><option value="ssh_public_key">{t("sshPublicKey")}</option></select></label>
            <label><FieldTitle label={t("encoding")} help={t("encodingHelp")} /><select value={draft.value.encoding} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, encoding: e.target.value as "utf8" | "base64" } })}><option value="utf8">{t("multilineUtf8")}</option><option value="base64">{t("base64Binary")}</option></select></label>
            {draft.kind !== "environment_variable" && <label><FieldTitle label={t("fileMode")} help={t("fileModeHelp")} /><input value={draft.file_mode === null ? "" : draft.file_mode.toString(8)} onChange={(e) => setDraft({ ...draft, file_mode: e.target.value ? Number.parseInt(e.target.value, 8) : null })} placeholder="600" /></label>}
            <label className="wide"><FieldTitle label={t("target")} help={t("targetHelp")} /><input required readOnly={draft.kind === "ssh_public_key"} value={draft.target} onChange={(e) => setDraft({ ...draft, target: e.target.value })} placeholder={draft.kind === "environment_variable" ? "GITHUB_TOKEN" : "/workspace/.config/example.yaml"} /></label>
            {draft.kind === "ssh_public_key" && <p className="profile-note wide">{t("sshTargetHelp")}</p>}
            {draft.kind !== "environment_variable" && <><label><FieldTitle label={t("owner")} help={t("ownerHelp")} /><input value={draft.owner ?? ""} onChange={(e) => setDraft({ ...draft, owner: e.target.value || null })} placeholder="workspace" /></label><label><FieldTitle label={t("group")} help={t("groupHelp")} /><input value={draft.group ?? ""} onChange={(e) => setDraft({ ...draft, group: e.target.value || null })} placeholder="workspace" /></label></>}
            <label className="wide"><FieldTitle label={t("templateSelector")} help={t("templateSelectorHelp")} /><select value={draft.template_selector ?? ""} onChange={(e) => setDraft({ ...draft, template_selector: e.target.value || null })}><option value="">{t("allTemplates")}</option>{draft.template_selector && !templates.some((template) => template.id === draft.template_selector) && <option value={draft.template_selector}>{draft.template_selector}</option>}{templates.map((template) => <option key={template.id} value={template.id}>{template.name}</option>)}</select></label>
            <div className="selector-editor wide">
              <div className="selector-heading">
                <FieldTitle label={t("labelSelector")} help={t("labelSelectorHelp")} />
                <button type="button" className="text-button" onClick={() => addSelector(draft, setDraft)}>{t("addSelector")}</button>
              </div>
              {Object.entries(draft.labels).length === 0 && <p>{t("noSelector")}</p>}
              {Object.entries(draft.labels).map(([key, value]) => (
                <div className="selector-row" key={key}>
                  <select aria-label={t("selectorField")} value={key} onChange={(event) => renameSelector(draft, setDraft, key, event.target.value)}>
                    {SELECTOR_KEYS.map((option) => <option key={option.key} value={option.key}>{t(option.label)}</option>)}
                  </select>
                  {key === "access_mode" ? (
                    <select aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, setDraft, key, event.target.value)}>
                      <option value="internal">{t("internal")}</option>
                      <option value="public">{t("public")}</option>
                    </select>
                  ) : (
                    <input required aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, setDraft, key, event.target.value)} placeholder={t("selectorValue")} />
                  )}
                  <button type="button" className="text-button danger" onClick={() => removeSelector(draft, setDraft, key)}>{t("removeSelector")}</button>
                </div>
              ))}
            </div>
            <label className="wide">{draft.value.encoding === "base64" ? t("valueBase64") : t("valueMultiline")}<textarea rows={15} spellCheck={false} value={draft.value.value} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, value: e.target.value } })} placeholder={draft.value.encoding === "base64" ? t("base64Hint") : t("multilineHint")} /></label>
          </div>
          <div className="check-row">
            <label title={t("sensitiveHelp")}><input type="checkbox" checked={draft.sensitive} onChange={(e) => setDraft({ ...draft, sensitive: e.target.checked })} />{t("sensitiveValue")}<Help text={t("sensitiveHelp")} /></label>
            {scope === "organization" && <label title={t("lockedHelp")}><input type="checkbox" checked={draft.locked} onChange={(e) => setDraft({ ...draft, locked: e.target.checked })} />{t("locked")}<Help text={t("lockedHelp")} /></label>}
          </div>
          <p className="security-note">{t("credentialWriteOnly")}</p>
          <button className="button primary" disabled={saving || !scopeId}>{saving ? t("savingEncrypted") : selectedKey ? t("replaceEncrypted") : t("createEncrypted")}</button>
        </form>
      </div>

      {preview.length > 0 && <div className="preview-card"><h3>{t("resolvedSources")}</h3><div className="preview-grid">{preview.map((item) => <div key={item.key}><strong>{item.key}</strong><span>{item.source === "organization" ? t("fromOrganization") : item.source === "user" ? t("fromUser") : t("fromWorkspace")}</span><small>{item.target}{item.locked ? ` · ${t("locked")}` : ""}</small></div>)}</div></div>}
    </section>
  );
}

function kindGlyph(kind: InjectionKind) {
  return kind === "environment_variable" ? "ENV" : kind === "ssh_public_key" ? "SSH" : kind === "secret_file" ? "SEC" : "CFG";
}

const SELECTOR_KEYS: readonly { key: string; label: MessageKey }[] = [
  { key: "access_mode", label: "selectorAccess" },
  { key: "template_id", label: "selectorTemplate" },
  { key: "image", label: "selectorImage" },
  { key: "owner_id", label: "selectorOwner" },
  { key: "organization_id", label: "selectorOrganization" },
  { key: "workspace_id", label: "selectorWorkspace" },
];

function addSelector(draft: InjectionDraft, update: Dispatch<SetStateAction<InjectionDraft>>) {
  const option = SELECTOR_KEYS.find(({ key }) => !(key in draft.labels));
  if (!option) return;
  update({
    ...draft,
    labels: { ...draft.labels, [option.key]: option.key === "access_mode" ? "internal" : "" },
  });
}

function renameSelector(draft: InjectionDraft, update: Dispatch<SetStateAction<InjectionDraft>>, previous: string, next: string) {
  if (previous === next || next in draft.labels) return;
  const labels = { ...draft.labels };
  const value = labels[previous];
  delete labels[previous];
  labels[next] = next === "access_mode" ? "internal" : value;
  update({ ...draft, labels });
}

function setSelectorValue(draft: InjectionDraft, update: Dispatch<SetStateAction<InjectionDraft>>, key: string, value: string) {
  update({ ...draft, labels: { ...draft.labels, [key]: value } });
}

function removeSelector(draft: InjectionDraft, update: Dispatch<SetStateAction<InjectionDraft>>, key: string) {
  const labels = { ...draft.labels };
  delete labels[key];
  update({ ...draft, labels });
}

function FieldTitle({ label, help }: { label: string; help?: string }) {
  return <span className="field-title"><span>{label}</span>{help && <Help text={help} />}</span>;
}

function Help({ text }: { text: string }) {
  return <span className="help-tip" title={text} aria-label={text} tabIndex={0}>?</span>;
}

function workspaceLabel(item: WorkspaceResponse) {
  return `${item.workspace.name} · ${item.workspace.short_id}`;
}

function kindLabel(kind: InjectionKind, t: ReturnType<typeof useI18n>["t"]) {
  return kind === "environment_variable" ? t("environmentVariable") : kind === "ssh_public_key" ? t("sshPublicKey") : kind === "secret_file" ? t("credentialFile") : t("configFile");
}

function sshTarget(key: string) {
  const safe = key.trim().replace(/[^A-Za-z0-9._-]+/g, "-") || "injected-key";
  return `/workspace/.mwc/${safe}.pub`;
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "操作失败";
}
