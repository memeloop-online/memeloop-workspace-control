import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Caption1,
  Checkbox,
  Field,
  Input,
  Option,
  Select,
  Text,
} from "@fluentui/react-components";

import { useI18n } from "../i18n";
import { useAdminStyles } from "../admin/fluentAdmin";
import { AllowedNodePoolsPicker, NodePoolSelect } from "../forms/NodePoolPicker";
import { TEMPLATE_NUMBER_POLICIES } from "./templateDraft";
import type { NumericPolicy, TemplateDraft } from "./templateDraft";
import type { AccessMode, AvailableNodePool, EgressPolicy } from "../types";

export function TemplateForm({ draft, setDraft, canGrantClusterAccess, nodePools, nodePoolsError, saving, t, styles }: { draft: TemplateDraft; setDraft: (value: TemplateDraft) => void; canGrantClusterAccess: boolean; nodePools: AvailableNodePool[]; nodePoolsError: boolean; saving: boolean; t: ReturnType<typeof useI18n>["t"]; styles: ReturnType<typeof useAdminStyles> }) {
  return <div className={styles.formGrid}>
    <Field label={t("templateName")} hint={t("templateNameHelp")} required><Input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} /></Field>
    <Field className={styles.full} label={t("allowedOciImage")} hint={t("templateImageHelp")} required><Input value={draft.image} onChange={(event) => setDraft({ ...draft, image: event.target.value })} placeholder="registry/image@sha256:…" /></Field>
    <Field label={t("accessMode")} hint={draft.accessMode === "internal" ? t("internalHelp") : t("publicHelp")}><Select value={draft.accessMode} onChange={(event) => setDraft({ ...draft, accessMode: event.target.value as AccessMode })}><Option value="internal">{t("internal")}</Option><Option value="public">{t("public")}</Option></Select></Field>
    <Field label={t("workspaceUser")} hint={t("workspaceUserHelp")} required><Input value={draft.user} onChange={(event) => setDraft({ ...draft, user: event.target.value })} /></Field>
    <Field className={styles.full} label={t("workspaceHome")} hint={t("workspaceHomeHelp")} required><Input value={draft.home} onChange={(event) => setDraft({ ...draft, home: event.target.value })} /></Field>
    <fieldset className={`${styles.full} ${styles.group}`}>
      <legend className={styles.groupLegend}><Text weight="semibold">{t("resources")}</Text></legend>
      <div className={styles.formGrid}>
        <NumberField label={`${t("cpuLimit")} (m)`} value={draft.cpu} policy={TEMPLATE_NUMBER_POLICIES.cpu} update={(cpu) => setDraft({ ...draft, cpu })} />
        <NumberField label={`${t("memoryLimit")} (MiB)`} value={draft.memory} policy={TEMPLATE_NUMBER_POLICIES.memory} update={(memory) => setDraft({ ...draft, memory })} />
        <NumberField label={`${t("cpuRequest")} (m)`} value={draft.requestCpu} policy={TEMPLATE_NUMBER_POLICIES.requestCpu} update={(requestCpu) => setDraft({ ...draft, requestCpu })} />
        <NumberField label={`${t("memoryRequest")} (MiB)`} value={draft.requestMemory} policy={TEMPLATE_NUMBER_POLICIES.requestMemory} update={(requestMemory) => setDraft({ ...draft, requestMemory })} />
        <NumberField label="GPU" value={draft.gpu} policy={TEMPLATE_NUMBER_POLICIES.gpu} update={(gpu) => setDraft({ ...draft, gpu })} />
        <NumberField label={`${t("disk")} (GiB)`} value={draft.disk} policy={TEMPLATE_NUMBER_POLICIES.disk} update={(disk) => setDraft({ ...draft, disk })} />
      </div>
    </fieldset>
    <fieldset className={`${styles.full} ${styles.group}`}>
      <legend className={styles.groupLegend}><Text weight="semibold">{t("storagePolicyTitle")}</Text></legend>
      <Caption1 className={styles.muted}>{t("storagePolicyHelp")}</Caption1>
      <div className={styles.formGrid}>
        <NumberField label={`${t("temporaryStorage")} (GiB)`} help={t("temporaryStorageHelp")} value={draft.storagePolicy.temporary_storage_gib} policy={TEMPLATE_NUMBER_POLICIES.temporaryStorage} update={(temporary_storage_gib) => setDraft({ ...draft, storagePolicy: { temporary_storage_gib } })} />
      </div>
    </fieldset>
    <Field className={styles.full} hint={t("imageBuildServiceHelp")}><Checkbox checked={draft.buildkit} onChange={(_, data) => setDraft({ ...draft, buildkit: Boolean(data.checked) })} label={t("imageBuildService")} /></Field>
    <fieldset className={`${styles.full} ${styles.group}`}>
      <legend className={styles.groupLegend}><Text weight="semibold">{t("placementTitle")}</Text></legend>
      <Caption1 className={styles.muted}>{t("placementHelp")}</Caption1>
      <div className={styles.formGrid}>
        <Field className={styles.full} label={t("allowedNodePools")} hint={nodePoolsError ? t("nodePoolsLoadFailed") : t("allowedNodePoolsHelp")} required>
          <AllowedNodePoolsPicker pools={nodePools} selected={draft.allowedNodePools} disabled={saving} onChange={(allowedNodePools) => setDraft({ ...draft, allowedNodePools, defaultNodePool: allowedNodePools.includes(draft.defaultNodePool) ? draft.defaultNodePool : (allowedNodePools[0] ?? "") })} />
        </Field>
        <Field label={t("defaultNodePool")} hint={t("defaultNodePoolHelp")} required>
          <NodePoolSelect pools={nodePools} allowed={draft.allowedNodePools} value={draft.defaultNodePool} disabled={saving || draft.allowedNodePools.length === 0} onChange={(defaultNodePool) => setDraft({ ...draft, defaultNodePool })} aria-label={t("defaultNodePool")} />
        </Field>
      </div>
    </fieldset>
    <Field className={styles.full} hint={t("maintenanceAccessHelp")}><Checkbox checked={draft.clusterAccess} disabled={!canGrantClusterAccess} onChange={(_, data) => setDraft({ ...draft, clusterAccess: Boolean(data.checked) })} label={t("maintenanceAccess")} /></Field>
    <fieldset className={`${styles.full} ${styles.group}`}>
      <legend className={styles.groupLegend}><Text weight="semibold">{t("browserDesktop")}</Text></legend>
      <div className={styles.formGrid}>
        <Field hint={t("enableBrowserDesktopHelp")}><Checkbox checked={draft.desktopEnabled} onChange={(_, data) => setDraft({ ...draft, desktopEnabled: Boolean(data.checked) })} label={t("enableBrowserDesktop")} /></Field>
        {draft.desktopEnabled && <NumberField label={t("desktopInternalPort")} help={t("desktopInternalPortHelp")} value={draft.desktopPort} policy={TEMPLATE_NUMBER_POLICIES.desktopPort} update={(desktopPort) => setDraft({ ...draft, desktopPort })} />}
        {draft.desktopEnabled && <Field label={t("desktopDisplayName")} hint={t("desktopDisplayNameHelp")}><Input value={draft.desktopDisplayName} maxLength={80} onChange={(event) => setDraft({ ...draft, desktopDisplayName: event.target.value })} placeholder={t("desktopDisplayNamePlaceholder")} /></Field>}
      </div>
    </fieldset>
    <Field label={t("egressPolicy")} hint={t("egressPolicyHelp")}><Select value={draft.egressPolicy} onChange={(event) => setDraft({ ...draft, egressPolicy: event.target.value as EgressPolicy })}><Option value="unrestricted">{t("egressUnrestricted")}</Option><Option value="internet_only">{t("egressInternetOnly")}</Option></Select></Field>
    <div className={styles.full}>
      <Accordion collapsible>
        <AccordionItem value="advanced">
          <AccordionHeader size="large">{t("advancedSettings")}</AccordionHeader>
          <AccordionPanel>
            <fieldset className={styles.group}>
              <legend className={styles.groupLegend}><Text weight="semibold">{t("scheduling")}</Text></legend>
              <Caption1 className={styles.muted}>{t("schedulingHelp")}</Caption1>
              <div className={styles.formGrid}>
                <Field label={t("runtimeClassName")} hint={t("runtimeClassNameHelp")}><Input value={draft.runtimeClassName} onChange={(event) => setDraft({ ...draft, runtimeClassName: event.target.value })} placeholder="gvisor-sandbox" /></Field>
              </div>
            </fieldset>
          </AccordionPanel>
        </AccordionItem>
      </Accordion>
    </div>
  </div>;
}

function NumberField({ label, help, value, policy, optional = false, update }: { label: string; help?: string; value: string; policy: NumericPolicy; optional?: boolean; update: (value: string) => void }) {
  return <Field label={label} hint={help} required={!optional}><Input type="number" inputMode="numeric" min={policy.min} step={policy.step} max={policy.max} value={value} onChange={(event) => update(event.target.value)} /></Field>;
}
