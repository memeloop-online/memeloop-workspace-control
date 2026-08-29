import { useEffect, useId, useState } from "react";
import type { Dispatch, FormEvent, SetStateAction } from "react";

import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import type { InjectionKind, InjectionScope, WorkspaceTemplate } from "../types";
import {
  FILE_MODE_PATTERN,
  changeInjectionKey,
  changeInjectionKind,
} from "./editorModel";
import type { InjectionEditorDraft } from "./editorModel";

interface Props {
  draft: InjectionEditorDraft;
  update: Dispatch<SetStateAction<InjectionEditorDraft>>;
  scope: InjectionScope;
  templates: WorkspaceTemplate[];
  selectedKey: string | null;
  saving: boolean;
  disabled?: boolean;
  fixedTemplate?: WorkspaceTemplate;
  className?: string;
  onReset: () => void;
  onSubmit: () => void | Promise<void>;
  onDelete?: () => void | Promise<void>;
}

export function InjectionEditorForm({
  draft,
  update,
  scope,
  templates,
  selectedKey,
  saving,
  disabled = false,
  fixedTemplate,
  className = "editor-card",
  onReset,
  onSubmit,
  onDelete,
}: Props) {
  const { t } = useI18n();

  function submit(event: FormEvent) {
    event.preventDefault();
    void onSubmit();
  }

  return (
    <form className={className} onSubmit={submit}>
      <div className="editor-heading">
        <strong>{selectedKey ? t("editingCredential") : t("newCredential")}</strong>
        {selectedKey && <button type="button" className="text-button" onClick={onReset}>{t("cancel")}</button>}
      </div>
      <div className="editor-grid">
        <label>
          <FieldTitle label={t("key")} help={t("keyHint")} />
          <input required readOnly={selectedKey !== null} value={draft.key} onChange={(event) => update((current) => changeInjectionKey(current, event.target.value))} placeholder={t("keyHint")} />
        </label>
        <label>
          <FieldTitle label={t("type")} />
          <select value={draft.kind} onChange={(event) => update((current) => changeInjectionKind(current, event.target.value as InjectionKind))}>
            <option value="environment_variable">{t("environmentVariable")}</option>
            <option value="config_file">{t("configFile")}</option>
            <option value="secret_file">{t("credentialFile")}</option>
            <option value="ssh_public_key">{t("sshPublicKey")}</option>
          </select>
        </label>
        <label>
          <FieldTitle label={t("encoding")} help={t("encodingHelp")} />
          <select value={draft.value.encoding} onChange={(event) => update({ ...draft, value: { ...draft.value, encoding: event.target.value as "utf8" | "base64" } })}>
            <option value="utf8">{t("multilineUtf8")}</option>
            <option value="base64">{t("base64Binary")}</option>
          </select>
        </label>
        {draft.kind !== "environment_variable" && (
          <label>
            <FieldTitle label={t("fileMode")} help={t("fileModeHelp")} />
            <input
              inputMode="numeric"
              pattern={FILE_MODE_PATTERN}
              maxLength={4}
              value={draft.fileMode}
              onChange={(event) => update({ ...draft, fileMode: event.target.value })}
              placeholder={draft.kind === "secret_file" ? "600" : "644"}
            />
          </label>
        )}
        <label className="wide">
          <FieldTitle label={t("target")} help={t("targetHelp")} />
          <input required readOnly={draft.kind === "ssh_public_key"} value={draft.target} onChange={(event) => update({ ...draft, target: event.target.value })} placeholder={draft.kind === "environment_variable" ? "GITHUB_TOKEN" : "/workspace/.config/example.yaml"} />
        </label>
        {draft.kind === "ssh_public_key" && <p className="profile-note wide">{t("sshTargetHelp")}</p>}
        {draft.kind !== "environment_variable" && <>
          <label><FieldTitle label={t("owner")} help={t("ownerHelp")} /><input value={draft.owner ?? ""} onChange={(event) => update({ ...draft, owner: event.target.value || null })} placeholder="workspace" /></label>
          <label><FieldTitle label={t("group")} help={t("groupHelp")} /><input value={draft.group ?? ""} onChange={(event) => update({ ...draft, group: event.target.value || null })} placeholder="workspace" /></label>
        </>}
        {fixedTemplate ? (
          <label className="wide">
            <FieldTitle label={t("templateSelector")} help={t("templateEnvironmentSelectorHelp")} />
            <input readOnly value={fixedTemplate.name} />
          </label>
        ) : <TemplateSelectorAutocomplete draft={draft} update={update} templates={templates} />}
        <SelectorEditor draft={draft} update={update} fixedTemplate={fixedTemplate} />
        <label className="wide">
          {draft.value.encoding === "base64" ? t("valueBase64") : t("valueMultiline")}
          <textarea rows={15} spellCheck={false} value={draft.value.value} onChange={(event) => update({ ...draft, value: { ...draft.value, value: event.target.value } })} placeholder={draft.value.encoding === "base64" ? t("base64Hint") : t("multilineHint")} />
        </label>
      </div>
      <div className="check-row">
        <label title={t("sensitiveHelp")}><input type="checkbox" checked={draft.sensitive} onChange={(event) => update({ ...draft, sensitive: event.target.checked })} />{t("sensitiveValue")}<Help text={t("sensitiveHelp")} /></label>
        {scope === "organization" && <label title={t("lockedHelp")}><input type="checkbox" checked={draft.locked} onChange={(event) => update({ ...draft, locked: event.target.checked })} />{t("locked")}<Help text={t("lockedHelp")} /></label>}
      </div>
      <p className="security-note">{t("credentialWriteOnly")}</p>
      <div className="injection-editor-actions">
        <button className="button primary" disabled={saving || disabled}>{saving ? t("savingEncrypted") : selectedKey ? t("replaceEncrypted") : t("createEncrypted")}</button>
        {selectedKey && <button type="button" className="button danger" disabled={saving || !onDelete} title={onDelete ? undefined : t("deleteCredentialUnavailable")} onClick={() => void onDelete?.()}>{t("deleteCredential")}</button>}
      </div>
    </form>
  );
}

function SelectorEditor({ draft, update, fixedTemplate }: { draft: InjectionEditorDraft; update: Dispatch<SetStateAction<InjectionEditorDraft>>; fixedTemplate?: WorkspaceTemplate }) {
  const { t } = useI18n();
  const selectorKeys = fixedTemplate ? SELECTOR_KEYS.filter(({ key }) => key !== "template_id") : SELECTOR_KEYS;
  return (
    <div className="selector-editor wide">
      <div className="selector-heading">
        <FieldTitle label={t("labelSelector")} help={t("labelSelectorHelp")} />
        <button type="button" className="text-button" onClick={() => addSelector(draft, update, selectorKeys)}>{t("addSelector")}</button>
      </div>
      {Object.entries(draft.labels).length === 0 && <p>{t("noSelector")}</p>}
      {Object.entries(draft.labels).map(([key, value]) => (
        <div className="selector-row" key={key}>
          <select aria-label={t("selectorField")} value={key} onChange={(event) => renameSelector(draft, update, key, event.target.value)}>
            {selectorKeys.map((option) => <option key={option.key} value={option.key}>{t(option.label)}</option>)}
          </select>
          {key === "access_mode" ? (
            <select aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, update, key, event.target.value)}>
              <option value="internal">{t("internal")}</option>
              <option value="public">{t("public")}</option>
            </select>
          ) : (
            <input required aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, update, key, event.target.value)} placeholder={t("selectorValue")} />
          )}
          <button type="button" className="text-button danger" onClick={() => removeSelector(draft, update, key)}>{t("removeSelector")}</button>
        </div>
      ))}
    </div>
  );
}

const SELECTOR_KEYS: readonly { key: string; label: MessageKey }[] = [
  { key: "access_mode", label: "selectorAccess" },
  { key: "image", label: "selectorImage" },
  { key: "owner_id", label: "selectorOwner" },
  { key: "organization_id", label: "selectorOrganization" },
  { key: "workspace_id", label: "selectorWorkspace" },
];

function TemplateSelectorAutocomplete({ draft, update, templates }: { draft: InjectionEditorDraft; update: Dispatch<SetStateAction<InjectionEditorDraft>>; templates: WorkspaceTemplate[] }) {
  const { t } = useI18n();
  const listId = useId();
  const selected = templates.find((template) => template.id === draft.template_selector);
  const selectedLabel = selected ? templateLabel(selected) : draft.template_selector ?? "";
  const [query, setQuery] = useState(selectedLabel);

  useEffect(() => setQuery(selectedLabel), [selectedLabel]);

  function change(value: string) {
    setQuery(value);
    const match = templates.find((template) => templateLabel(template) === value || template.id === value);
    if (match || value === "") {
      update((current) => ({ ...current, template_selector: match?.id ?? null }));
    }
  }

  return <label className="wide">
    <FieldTitle label={t("templateSelector")} help={t("templateSelectorHelp")} />
    <input type="search" list={listId} value={query} onChange={(event) => change(event.target.value)} onBlur={() => setQuery(selectedLabel)} placeholder={t("allTemplates")} autoComplete="off" />
    <datalist id={listId}>{templates.map((template) => <option key={template.id} value={templateLabel(template)} />)}</datalist>
  </label>;
}

function templateLabel(template: WorkspaceTemplate) {
  return `${template.name} · ${template.id.slice(0, 8)}`;
}

function addSelector(draft: InjectionEditorDraft, update: Dispatch<SetStateAction<InjectionEditorDraft>>, options: typeof SELECTOR_KEYS) {
  const option = options.find(({ key }) => !(key in draft.labels));
  if (!option) return;
  update({ ...draft, labels: { ...draft.labels, [option.key]: option.key === "access_mode" ? "internal" : "" } });
}

function renameSelector(draft: InjectionEditorDraft, update: Dispatch<SetStateAction<InjectionEditorDraft>>, previous: string, next: string) {
  if (previous === next || next in draft.labels) return;
  const labels = { ...draft.labels };
  const value = labels[previous];
  delete labels[previous];
  labels[next] = next === "access_mode" ? "internal" : value;
  update({ ...draft, labels });
}

function setSelectorValue(draft: InjectionEditorDraft, update: Dispatch<SetStateAction<InjectionEditorDraft>>, key: string, value: string) {
  update({ ...draft, labels: { ...draft.labels, [key]: value } });
}

function removeSelector(draft: InjectionEditorDraft, update: Dispatch<SetStateAction<InjectionEditorDraft>>, key: string) {
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
