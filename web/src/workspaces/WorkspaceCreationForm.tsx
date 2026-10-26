import { useState, type FormEvent } from "react";
import { Button, Card, Caption1, Dropdown, Field, InfoLabel, Input, MessageBar, MessageBarBody, Option, Text } from "@fluentui/react-components";
import { AddRegular } from "@fluentui/react-icons";
import { CredentialReferencePicker } from "../forms/CredentialReferencePicker";
import { nodePoolDisplayName } from "../forms/NodePoolPicker";
import { useI18n } from "../i18n";
import type { AvailableNodePool, Resources, StoredInjection, WorkspaceTemplate } from "../types";
import { useWorkspaceStyles } from "./workspaceStyles";

interface Props {
  name: string;
  templateId: string;
  templates: WorkspaceTemplate[];
  nodePools: AvailableNodePool[];
  nodePoolsStatus: "loading" | "ready" | "failed";
  nodePool: string;
  resourceDraft: Resources | null;
  temporaryStorageGiB: string;
  explicitInjectionRefs: boolean;
  organizationInjections: StoredInjection[];
  userInjections: StoredInjection[];
  organizationRefs: string[];
  userRefs: string[];
  submitting: boolean;
  onNameChange: (value: string) => void;
  onTemplateChange: (id: string) => void;
  onNodePoolChange: (name: string) => void;
  onNodePoolsRetry: () => void;
  onResourceChange: (key: keyof Resources, value: string) => void;
  onTemporaryStorageChange: (value: string) => void;
  onReferenceModeChange: (explicit: boolean) => void;
  onOrganizationRefsChange: (keys: string[]) => void;
  onUserRefsChange: (keys: string[]) => void;
  onSubmit: (event: FormEvent) => void;
}

export function WorkspaceCreationForm(props: Props) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  const selectedTemplate = props.templates.find((template) => template.id === props.templateId);
  const selectableLocations = props.nodePools.filter((pool) => selectedTemplate?.placement.allowed_node_pools.includes(pool.name));
  const [attempted, setAttempted] = useState(false);
  const nameError = attempted && !props.name.trim() ? t("workspaceNameRequired") : undefined;
  // A template default that is disabled or absent from the public list is not a
  // usable choice: a single remaining location is submitted explicitly, and with
  // several remaining locations the user must pick one.
  const defaultAvailable = selectedTemplate !== undefined && selectableLocations.some((pool) => pool.name === selectedTemplate.placement.default_node_pool);
  const explicitChoiceRequired = selectableLocations.length > 1 && !defaultAvailable;
  // Location choice is only submittable once the public list settled and the
  // template actually intersects it; the create handler re-checks the same rule.
  const locationBlocked = props.nodePoolsStatus !== "ready" || (selectedTemplate !== undefined && selectableLocations.length === 0) || (explicitChoiceRequired && !props.nodePool);
  return <Card className={styles.createCard} appearance="filled-alternative">
    <form className={styles.formGrid} onSubmit={(event) => { setAttempted(true); props.onSubmit(event); }}>
    <Field label={t("name")} required validationState={nameError ? "error" : undefined} validationMessage={nameError}><Input value={props.name} maxLength={120} onChange={(_, data) => props.onNameChange(data.value)} /></Field>
    <Field label={<InfoLabel info={t("templatePersistenceHelp")}>{t("template")}</InfoLabel>} required>
      <Dropdown value={selectedTemplate?.name ?? ""} selectedOptions={props.templateId ? [props.templateId] : []} placeholder={t("chooseTemplate")} onOptionSelect={(_, data) => props.onTemplateChange(data.optionValue ?? "")}>
        {props.templates.map((template) => <Option key={template.id} value={template.id}>{template.name}</Option>)}
      </Dropdown>
    </Field>
    {selectedTemplate && <TemplateSummary template={selectedTemplate} />}
    <div className={styles.formWide}>
      <Field label={<InfoLabel info={t("workspaceRunLocationHelp")}>{t("workspaceRunLocation")}</InfoLabel>}>
        {props.nodePoolsStatus !== "ready" || !selectedTemplate || selectableLocations.length === 0
          ? <Input disabled readOnly value={
              props.nodePoolsStatus === "loading" ? t("loading")
                : props.nodePoolsStatus === "failed" ? ""
                : !selectedTemplate ? t("workspaceRunLocationChooseTemplate")
                : ""
            } placeholder={props.nodePoolsStatus === "failed" || (selectedTemplate && selectableLocations.length === 0) ? "—" : undefined} />
          : selectableLocations.length === 1
            ? <Input disabled readOnly value={selectableLocations[0]?.display_name ?? selectableLocations[0]?.name ?? ""} />
            : <Dropdown
                value={props.nodePool ? nodePoolDisplayName(props.nodePools, props.nodePool) : defaultAvailable ? t("workspaceRunLocationTemplateDefault") : ""}
                selectedOptions={props.nodePool || defaultAvailable ? [props.nodePool] : []}
                placeholder={defaultAvailable ? undefined : t("workspaceRunLocationChoose")}
                onOptionSelect={(_, data) => props.onNodePoolChange(data.optionValue ?? "")}
              >
                {defaultAvailable && <Option value="" text={t("workspaceRunLocationTemplateDefault")}>{t("workspaceRunLocationTemplateDefault")}</Option>}
                {selectableLocations.map((pool) => <Option key={pool.name} value={pool.name} text={pool.display_name}>{pool.display_name}</Option>)}
              </Dropdown>}
      </Field>
      {props.nodePoolsStatus === "ready" && selectedTemplate && selectableLocations.length === 1 && <Caption1 className={styles.meterHint}>{t("workspaceRunLocationFixed")}</Caption1>}
      {props.nodePoolsStatus === "ready" && selectedTemplate && selectableLocations.length === 0 && <MessageBar intent="warning"><MessageBarBody>{t("workspaceRunLocationNone")}</MessageBarBody></MessageBar>}
      {props.nodePoolsStatus === "failed" && <MessageBar intent="error"><MessageBarBody>{t("runLocationsLoadFailed")}</MessageBarBody><Button appearance="subtle" onClick={props.onNodePoolsRetry}>{t("retry")}</Button></MessageBar>}
    </div>
    {selectedTemplate && props.resourceDraft && <ResourceEditor template={selectedTemplate} resources={props.resourceDraft} onChange={props.onResourceChange} />}
    {selectedTemplate && <Field label={t("workspaceTemporaryStorageGiB")} hint={t("workspaceTemporaryStorageHelp")}>
      <Input type="number" min={1} max={2048} step={1} required value={props.temporaryStorageGiB} onChange={(_, data) => props.onTemporaryStorageChange(data.value)} />
    </Field>}
    <Field className={styles.formWide} label={t("injectionReferences")}>
      <Dropdown value={props.explicitInjectionRefs ? t("selectedReferences") : t("allMatching")} selectedOptions={[props.explicitInjectionRefs ? "selected" : "all"]} onOptionSelect={(_, data) => props.onReferenceModeChange(data.optionValue === "selected")}>
        <Option value="all">{t("allMatching")}</Option><Option value="selected">{t("selectedReferences")}</Option>
      </Dropdown>
    </Field>
    {props.explicitInjectionRefs && <div className={styles.formWide}><CredentialReferencePicker organizationItems={props.organizationInjections} userItems={props.userInjections} organizationSelected={props.organizationRefs} userSelected={props.userRefs} onOrganizationSelected={props.onOrganizationRefsChange} onUserSelected={props.onUserRefsChange} /></div>}
    <div className={`${styles.formActions} ${styles.formWide}`}><Button appearance="primary" icon={<AddRegular />} type="submit" disabled={props.submitting || !props.templateId || locationBlocked}>{props.submitting ? t("creating") : t("submitCreate")}</Button></div>
    </form>
  </Card>;
}

function TemplateSummary({ template }: { template: WorkspaceTemplate }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  return <dl className={`${styles.templateSummary} ${styles.formWide}`}>
    <div className={styles.summaryItem}><Caption1 className={styles.summaryLabel}>{t("image")}</Caption1><Text className={`${styles.summaryValue} ${styles.code}`}>{template.image}</Text></div>
    <div className={styles.summaryItem}><Caption1 className={styles.summaryLabel}><InfoLabel info={template.access_mode === "internal" ? t("internalHelp") : t("publicHelp")}>{t("accessMode")}</InfoLabel></Caption1><Text>{template.access_mode === "internal" ? t("internal") : t("public")}</Text></div>
    <div className={styles.summaryItem}><Caption1 className={styles.summaryLabel}>{t("resources")}</Caption1><Text>{template.resources.cpu_millis}m CPU · {template.resources.memory_mib} MiB · {template.resources.gpu_count} GPU</Text></div>
    <div className={styles.summaryItem}><Caption1 className={styles.summaryLabel}>{t("storagePolicyTitle")}</Caption1><Text>{template.resources.disk_gib} GiB {t("persistentDisk")} · {template.storage_policy.temporary_storage_gib} GiB {t("temporaryStorage")}</Text></div>
    <div className={styles.summaryItem}><Caption1 className={styles.summaryLabel}>{t("workspaceUser")}</Caption1><Text className={styles.summaryValue}>{template.workspace_user} · {template.workspace_home}</Text></div>
  </dl>;
}

function ResourceEditor({ template, resources, onChange }: { template: WorkspaceTemplate; resources: Resources; onChange: (key: keyof Resources, value: string) => void }) {
  const { t } = useI18n();
  const styles = useWorkspaceStyles();
  return <div className={styles.formWide}><Field label={<InfoLabel info={t("workspaceResourcesHelp")}>{t("workspaceResources")}</InfoLabel>}><div className={styles.formGrid}>
    <Field label={t("cpuLimitMillis")} validationState={resources.cpu_millis < template.pod_requests.cpu_millis ? "error" : undefined} validationMessage={resources.cpu_millis < template.pod_requests.cpu_millis ? t("workspaceResourcesInvalid") : undefined}><Input type="number" min={template.pod_requests.cpu_millis} step={100} required value={String(resources.cpu_millis)} onChange={(event) => onChange("cpu_millis", event.currentTarget.value)} /></Field>
    <Field label={t("memoryLimitMib")} validationState={resources.memory_mib < template.pod_requests.memory_mib ? "error" : undefined} validationMessage={resources.memory_mib < template.pod_requests.memory_mib ? t("workspaceResourcesInvalid") : undefined}><Input type="number" min={template.pod_requests.memory_mib} step={256} required value={String(resources.memory_mib)} onChange={(event) => onChange("memory_mib", event.currentTarget.value)} /></Field>
    <Field label={t("diskSizeGib")} validationState={resources.disk_gib < 1 ? "error" : undefined} validationMessage={resources.disk_gib < 1 ? t("workspaceResourcesInvalid") : undefined}><Input type="number" min={1} step={1} required value={String(resources.disk_gib)} onChange={(event) => onChange("disk_gib", event.currentTarget.value)} /></Field>
    <Field label={t("gpuCount")} validationState={resources.gpu_count < 0 ? "error" : undefined} validationMessage={resources.gpu_count < 0 ? t("workspaceResourcesInvalid") : undefined}><Input type="number" min={0} step={1} required value={String(resources.gpu_count)} onChange={(event) => onChange("gpu_count", event.currentTarget.value)} /></Field>
  </div></Field></div>;
}
