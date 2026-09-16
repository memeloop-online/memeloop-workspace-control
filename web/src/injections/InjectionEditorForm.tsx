import { useEffect, useMemo, useRef, useState } from "react";
import type { Dispatch, FormEvent, SetStateAction } from "react";
import {
  Button,
  Card,
  Checkbox,
  Combobox,
  Divider,
  Field,
  Input,
  Option,
  Select,
  Text,
  Textarea,
  Tooltip,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { AddRegular, DeleteRegular, DismissRegular, InfoRegular, SaveRegular } from "@fluentui/react-icons";

import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import type { InjectionKind, InjectionScope, WorkspaceTemplate } from "../types";
import { FILE_MODE_PATTERN, changeInjectionKey, changeInjectionKind } from "./editorModel";
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

const useStyles = makeStyles({
  form: {
    display: "grid",
    gap: tokens.spacingVerticalL,
    minWidth: 0,
    padding: tokens.spacingHorizontalL,
  },
  heading: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: tokens.spacingHorizontalM,
  },
  headingTitle: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    gap: tokens.spacingVerticalM,
    [`@media (max-width: 720px)`]: {
      gridTemplateColumns: "1fr",
    },
  },
  wide: {
    gridColumn: "1 / -1",
  },
  label: {
    display: "inline-flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
  },
  info: {
    color: tokens.colorNeutralForeground3,
    cursor: "help",
  },
  note: {
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
  selector: {
    display: "grid",
    gap: tokens.spacingVerticalS,
    gridColumn: "1 / -1",
    padding: tokens.spacingHorizontalM,
    backgroundColor: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
  },
  selectorHeading: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: tokens.spacingHorizontalM,
  },
  selectorRow: {
    display: "grid",
    gridTemplateColumns: "minmax(8rem, 0.8fr) minmax(0, 1fr) auto",
    gap: tokens.spacingHorizontalS,
    alignItems: "end",
    [`@media (max-width: 640px)`]: {
      gridTemplateColumns: "1fr",
    },
  },
  textarea: {
    width: "100%",
    minHeight: "12rem",
    fontFamily: tokens.fontFamilyMonospace,
  },
  checks: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalL,
  },
  actions: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalS,
  },
  templatePicker: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr) auto",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    [`@media (max-width: 560px)`]: { gridTemplateColumns: "1fr" },
  },
});

export function InjectionEditorForm({
  draft,
  update,
  scope,
  templates,
  selectedKey,
  saving,
  disabled = false,
  fixedTemplate,
  className,
  onReset,
  onSubmit,
  onDelete,
}: Props) {
  const { t } = useI18n();
  const styles = useStyles();

  function submit(event: FormEvent) {
    event.preventDefault();
    void onSubmit();
  }

  return (
    <form className={`${styles.form} ${className ?? ""}`} onSubmit={submit}>
      <div className={styles.heading}>
        <div className={styles.headingTitle}>
          <Text size={500} weight="semibold">{selectedKey ? t("editingCredential") : t("newCredential")}</Text>
          {selectedKey && <Text size={200} className={styles.note}>{t("credentialWriteOnly")}</Text>}
        </div>
        {selectedKey && <Button type="button" appearance="subtle" icon={<DismissRegular aria-hidden="true" />} onClick={onReset}>{t("cancel")}</Button>}
      </div>
      <Divider />
      <div className={styles.grid}>
        <Field required label={<FieldLabel label={t("key")} help={t("keyHint")} />}>
          <Input
            required
            readOnly={selectedKey !== null}
            value={draft.key}
            onChange={(event) => {
              const value = event.currentTarget.value;
              update((current) => changeInjectionKey(current, value));
            }}
            placeholder={t("keyHint")}
          />
        </Field>
        <Field label={<FieldLabel label={t("type")} />}>
          <Select value={draft.kind} onChange={(event) => {
            const kind = event.currentTarget.value as InjectionKind;
            update((current) => changeInjectionKind(current, kind));
          }}>
            <option value="environment_variable">{t("environmentVariable")}</option>
            <option value="config_file">{t("configFile")}</option>
            <option value="secret_file">{t("credentialFile")}</option>
            <option value="ssh_public_key">{t("sshPublicKey")}</option>
          </Select>
        </Field>
        <Field label={<FieldLabel label={t("encoding")} help={t("encodingHelp")} />}>
          <Select value={draft.value.encoding} onChange={(event) => update({ ...draft, value: { ...draft.value, encoding: event.currentTarget.value as "utf8" | "base64" } })}>
            <option value="utf8">{t("multilineUtf8")}</option>
            <option value="base64">{t("base64Binary")}</option>
          </Select>
        </Field>
        {draft.kind !== "environment_variable" && (
          <Field label={<FieldLabel label={t("fileMode")} help={t("fileModeHelp")} />}>
            <Input
              inputMode="numeric"
              pattern={FILE_MODE_PATTERN}
              maxLength={4}
              value={draft.fileMode}
              onChange={(event) => update({ ...draft, fileMode: event.currentTarget.value })}
              placeholder={draft.kind === "secret_file" ? "600" : "644"}
            />
          </Field>
        )}
        <Field className={styles.wide} required label={<FieldLabel label={t("target")} help={t("targetHelp")} />}>
          <Input
            required
            readOnly={draft.kind === "ssh_public_key"}
            value={draft.target}
            onChange={(event) => update({ ...draft, target: event.currentTarget.value })}
            placeholder={draft.kind === "environment_variable" ? "GITHUB_TOKEN" : "/workspace/.config/example.yaml"}
          />
        </Field>
        {draft.kind === "ssh_public_key" && <Text className={`${styles.wide} ${styles.note}`}>{t("sshTargetHelp")}</Text>}
        {draft.kind !== "environment_variable" && <>
          <Field label={<FieldLabel label={t("owner")} help={t("ownerHelp")} />}>
            <Input value={draft.owner ?? ""} onChange={(event) => update({ ...draft, owner: event.currentTarget.value || null })} placeholder="workspace" />
          </Field>
          <Field label={<FieldLabel label={t("group")} help={t("groupHelp")} />}>
            <Input value={draft.group ?? ""} onChange={(event) => update({ ...draft, group: event.currentTarget.value || null })} placeholder="workspace" />
          </Field>
        </>}
        {fixedTemplate ? (
          <Field className={styles.wide} label={<FieldLabel label={t("templateSelector")} help={t("templateInjectionSelectorHelp")} />}>
            <Input readOnly value={fixedTemplate.name} />
          </Field>
        ) : <TemplateSelectorAutocomplete draft={draft} update={update} templates={templates} />}
        <SelectorEditor draft={draft} update={update} fixedTemplate={fixedTemplate} />
        <Field className={styles.wide} label={draft.value.encoding === "base64" ? t("valueBase64") : t("valueMultiline")}>
          <Textarea
            className={styles.textarea}
            rows={12}
            resize="vertical"
            spellCheck={false}
            value={draft.value.value}
            onChange={(event) => update({ ...draft, value: { ...draft.value, value: event.currentTarget.value } })}
            placeholder={draft.value.encoding === "base64" ? t("base64Hint") : t("multilineHint")}
          />
        </Field>
      </div>
      <div className={styles.checks}>
        <Checkbox checked={draft.sensitive} onChange={(_, data) => update({ ...draft, sensitive: data.checked === true })} label={<FieldLabel label={t("sensitiveValue")} help={t("sensitiveHelp")} />} />
        {scope === "organization" && <Checkbox checked={draft.locked} onChange={(_, data) => update({ ...draft, locked: data.checked === true })} label={<FieldLabel label={t("locked")} help={t("lockedHelp")} />} />}
      </div>
      <div className={styles.actions}>
        <Button type="submit" appearance="primary" icon={<SaveRegular aria-hidden="true" />} disabled={saving || disabled}>{saving ? t("savingEncrypted") : selectedKey ? t("replaceEncrypted") : t("createEncrypted")}</Button>
        {selectedKey && <Button type="button" appearance="secondary" icon={<DeleteRegular aria-hidden="true" />} disabled={saving || !onDelete} onClick={() => void onDelete?.()}>{t("deleteCredential")}</Button>}
      </div>
    </form>
  );
}

function SelectorEditor({ draft, update, fixedTemplate }: { draft: InjectionEditorDraft; update: Dispatch<SetStateAction<InjectionEditorDraft>>; fixedTemplate?: WorkspaceTemplate }) {
  const { t } = useI18n();
  const styles = useStyles();
  const selectorKeys = fixedTemplate ? SELECTOR_KEYS.filter(({ key }) => key !== "template_id") : SELECTOR_KEYS;
  const canAdd = Object.keys(draft.labels).length < selectorKeys.length;
  return (
    <Card className={styles.selector} appearance="filled-alternative">
      <div className={styles.selectorHeading}>
        <FieldLabel label={t("labelSelector")} help={t("labelSelectorHelp")} />
        <Button type="button" appearance="subtle" icon={<AddRegular aria-hidden="true" />} disabled={!canAdd} onClick={() => addSelector(draft, update, selectorKeys)}>{t("addSelector")}</Button>
      </div>
      {Object.entries(draft.labels).length === 0 && <Text className={styles.note}>{t("noSelector")}</Text>}
      {Object.entries(draft.labels).map(([key, value]) => (
        <div className={styles.selectorRow} key={key}>
          <Field label={t("selectorField")}>
            <Select aria-label={t("selectorField")} value={key} onChange={(event) => renameSelector(draft, update, key, event.currentTarget.value)}>
              {selectorKeys.map((option) => <option key={option.key} value={option.key}>{t(option.label)}</option>)}
            </Select>
          </Field>
          <Field label={t("selectorValue")}>
            {key === "access_mode" ? (
              <Select aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, update, key, event.currentTarget.value)}>
                <option value="internal">{t("internal")}</option>
                <option value="public">{t("public")}</option>
              </Select>
            ) : <Input required aria-label={t("selectorValue")} value={value} onChange={(event) => setSelectorValue(draft, update, key, event.currentTarget.value)} placeholder={t("selectorValue")} />}
          </Field>
          <Button type="button" appearance="subtle" icon={<DeleteRegular aria-hidden="true" />} onClick={() => removeSelector(draft, update, key)}>{t("removeSelector")}</Button>
        </div>
      ))}
    </Card>
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
  const selected = templates.find((template) => template.id === draft.template_selector);
  const selectedLabel = selected ? templateLabel(selected) : "";
  const selectedRef = useRef<WorkspaceTemplate | null>(selected ?? null);
  const editingRef = useRef(false);
  const [query, setQuery] = useState(selectedLabel);
  const matches = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return normalized ? templates.filter((template) => templateLabel(template).toLocaleLowerCase().includes(normalized)) : templates;
  }, [query, templates]);
  const styles = useStyles();

  useEffect(() => {
    if (editingRef.current) return;
    setQuery(selectedLabel);
    selectedRef.current = selected ?? null;
    if (templates.length > 0 && draft.template_selector && !selected) {
      update((current) => current.template_selector === draft.template_selector ? { ...current, template_selector: null } : current);
    }
  }, [draft.template_selector, selected, selectedLabel, templates.length, update]);

  function findExact(value: string) {
    const normalized = value.trim().toLocaleLowerCase();
    if (!normalized) return undefined;
    return templates.find((template) => template.id === value.trim() || templateLabel(template).toLocaleLowerCase() === normalized);
  }

  function choose(template: WorkspaceTemplate | null) {
    editingRef.current = false;
    selectedRef.current = template;
    setQuery(template ? templateLabel(template) : "");
    update((current) => ({ ...current, template_selector: template?.id ?? null }));
  }

  function settle() {
    if (!editingRef.current) return;
    const match = findExact(query);
    if (match) {
      choose(match);
      return;
    }
    choose(selectedRef.current);
  }

  return (
    <Field className={styles.wide} label={<FieldLabel label={t("templateSelector")} help={t("templateSelectorHelp")} />}>
      <div className={styles.templatePicker}><Combobox
        freeform
        clearable
        value={query}
        placeholder={t("allTemplates")}
        onChange={(event) => {
          const value = event.currentTarget.value;
          if (!editingRef.current) selectedRef.current = selected ?? null;
          editingRef.current = true;
          setQuery(value);
          const match = findExact(value);
          if (!value.trim()) {
            choose(null);
          } else if (match) {
            selectedRef.current = match;
            update((current) => ({ ...current, template_selector: match.id }));
          }
        }}
        onOptionSelect={(_, data) => {
          const match = templates.find((template) => template.id === data.optionValue);
          choose(match ?? null);
        }}
        onBlur={settle}
        onOpenChange={(_, data) => {
          if (data.open && selectedRef.current && !editingRef.current) {
            editingRef.current = true;
            setQuery("");
          } else if (!data.open) settle();
        }}
        aria-label={t("templateSelector")}
      >
        {matches.map((template) => <Option key={template.id} value={template.id} text={templateLabel(template)}>{template.name}</Option>)}
      </Combobox>{selected && <Button type="button" appearance="subtle" icon={<DismissRegular aria-hidden="true" />} onClick={() => choose(null)}>{t("removeTemplateSelection")}</Button>}</div>
    </Field>
  );
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
  delete labels[previous];
  // Selector values belong to their field. Carrying an access mode such as
  // "internal" into an image or workspace selector silently creates an
  // invalid rule, so changing the field always starts with the right default.
  labels[next] = next === "access_mode" ? "internal" : "";
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

function FieldLabel({ label, help }: { label: string; help?: string }) {
  return <span>{label}{help && <Tooltip content={help} relationship="description"><InfoRegular aria-label={help} /></Tooltip>}</span>;
}
