import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  Button,
  Checkbox,
  DataGrid,
  DataGridBody,
  DataGridCell,
  DataGridHeader,
  DataGridHeaderCell,
  DataGridRow,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Spinner,
  Text,
  Textarea,
} from "@fluentui/react-components";
import type { TableColumnDefinition } from "@fluentui/react-components";
import { AddRegular } from "@fluentui/react-icons";

import type { ApiClient } from "../api";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useI18n } from "../i18n";
import type { MessageKey } from "../i18n";
import type { NodePool, NodePoolPlacement, PutNodePoolInput } from "../types";
import { AdminCard, SaveButton, useAdminStyles } from "./fluentAdmin";

/** The backend-managed fallback location can be edited but never deleted. */
const DEFAULT_LOCATION = "default";

interface Props {
  api: ApiClient;
  onError: (message: string) => void;
}

interface LocationDraft {
  /** Empty only while creating; the stable ID is immutable afterwards. */
  name: string;
  creating: boolean;
  displayName: string;
  enabled: boolean;
  requiredHosts: string;
  preferredHosts: string;
  selector: string;
}

/** Self-contained administrator card for the run location (node pool) list. */
export function RunLocationsCard({ api, onError }: Props) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const [pools, setPools] = useState<NodePool[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [busyName, setBusyName] = useState<string | null>(null);
  const [draft, setDraft] = useState<LocationDraft | null>(null);
  const [draftErrors, setDraftErrors] = useState<Partial<Record<"name" | "displayName" | "hosts" | "selector", MessageKey>>>({});
  const [dialogBusy, setDialogBusy] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<NodePool | null>(null);
  const generation = useRef(0);

  useEffect(() => {
    let active = true;
    const requestGeneration = ++generation.current;
    setLoadFailed(false);
    void api.adminNodePools()
      .then((value) => { if (active && generation.current === requestGeneration) setPools(value); })
      .catch(() => { if (active && generation.current === requestGeneration) { setPools(null); setLoadFailed(true); } });
    return () => { active = false; };
  }, [api, reload]);

  async function refresh() {
    try {
      setPools(await api.adminNodePools());
      setLoadFailed(false);
    } catch (error) {
      onError(errorMessage(error, t("requestFailed")));
    }
  }

  function openCreate() {
    setDraftErrors({});
    setDraft({ name: "", creating: true, displayName: "", enabled: true, requiredHosts: "", preferredHosts: "", selector: "" });
  }

  function openEdit(pool: NodePool) {
    setDraftErrors({});
    setDraft({
      name: pool.name,
      creating: false,
      displayName: pool.display_name,
      enabled: pool.enabled,
      requiredHosts: pool.placement.required_hosts.join("\n"),
      preferredHosts: pool.placement.preferred_hosts.join("\n"),
      selector: formatSelector(pool.placement.selector),
    });
  }

  async function saveDraft(event: FormEvent) {
    event.preventDefault();
    if (!draft || dialogBusy) return;
    const parsed = parseDraft(draft);
    if ("errors" in parsed) {
      setDraftErrors(parsed.errors);
      return;
    }
    setDraftErrors({});
    setDialogBusy(true);
    try {
      await api.putNodePool(draft.name, parsed.input);
      setDraft(null);
      await refresh();
    } catch (error) {
      onError(errorMessage(error, t("requestFailed")));
    } finally {
      setDialogBusy(false);
    }
  }

  async function toggleEnabled(pool: NodePool) {
    if (busyName) return;
    setBusyName(pool.name);
    try {
      await api.putNodePool(pool.name, { display_name: pool.display_name, placement: pool.placement, enabled: !pool.enabled });
      await refresh();
    } catch (error) {
      onError(errorMessage(error, t("requestFailed")));
    } finally {
      setBusyName(null);
    }
  }

  async function confirmDelete() {
    if (!deleteTarget || dialogBusy) return;
    setDialogBusy(true);
    try {
      await api.deleteNodePool(deleteTarget.name);
      setDeleteTarget(null);
      await refresh();
    } catch (error) {
      // Backend conflicts (still referenced, or the default location) stay visible.
      onError(errorMessage(error, t("requestFailed")));
    } finally {
      setDialogBusy(false);
    }
  }

  return <AdminCard
    title={t("runLocations")}
    description={<>{t("runLocationsHelp")}</>}
    action={<Button icon={<AddRegular />} disabled={pools === null} onClick={openCreate}>{t("createRunLocation")}</Button>}
  >
    {pools === null && !loadFailed && <Spinner size="tiny" label={t("runLocationsLoading")} />}
    {loadFailed && <MessageBar intent="error"><MessageBarBody>{t("runLocationsLoadFailed")}</MessageBarBody><Button appearance="subtle" onClick={() => setReload((value) => value + 1)}>{t("retry")}</Button></MessageBar>}
    {pools !== null && pools.length === 0 && <Text className={styles.muted}>{t("noRunLocations")}</Text>}
    {pools !== null && pools.length > 0 && <div className={styles.staticTable} aria-busy={busyName !== null}>
      <DataGrid items={pools} columns={columns(t, busyName, openEdit, (pool) => void toggleEnabled(pool), setDeleteTarget)}>
        <DataGridHeader><DataGridRow<NodePool>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader>
        <DataGridBody<NodePool>>{({ item }) => <DataGridRow<NodePool>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody>
      </DataGrid>
    </div>}
    <Dialog open={draft !== null} onOpenChange={(_, data) => { if (!data.open && !dialogBusy) setDraft(null); }}>
      <DialogSurface className={styles.dialogSurface}>
        <form onSubmit={(event) => void saveDraft(event)}>
          <DialogBody>
            <DialogTitle>{draft?.creating ? t("createRunLocation") : t("editRunLocation")}</DialogTitle>
            <DialogContent className={styles.dialogBody}>
              {draft && <>
                <Field label={t("runLocationName")} hint={draft.creating ? t("runLocationNameHelp") : undefined} required={draft.creating} validationState={draftErrors.name ? "error" : undefined} validationMessage={draftErrors.name ? t(draftErrors.name) : undefined}>
                  <Input value={draft.name} disabled={!draft.creating || dialogBusy} maxLength={63} placeholder="gpu-west" onChange={(_, data) => setDraft({ ...draft, name: data.value })} />
                </Field>
                <Field label={t("displayName")} required validationState={draftErrors.displayName ? "error" : undefined} validationMessage={draftErrors.displayName ? t(draftErrors.displayName) : undefined}>
                  <Input value={draft.displayName} disabled={dialogBusy} maxLength={80} onChange={(_, data) => setDraft({ ...draft, displayName: data.value })} />
                </Field>
                <Field label={t("runLocationRequiredHosts")} hint={t("runLocationRequiredHostsHelp")} validationState={draftErrors.hosts ? "error" : undefined} validationMessage={draftErrors.hosts ? t(draftErrors.hosts) : undefined}>
                  <Textarea value={draft.requiredHosts} disabled={dialogBusy} rows={3} onChange={(_, data) => setDraft({ ...draft, requiredHosts: data.value })} />
                </Field>
                <Field label={t("runLocationPreferredHosts")} hint={t("runLocationPreferredHostsHelp")} validationState={draftErrors.hosts ? "error" : undefined} validationMessage={draftErrors.hosts ? t(draftErrors.hosts) : undefined}>
                  <Textarea value={draft.preferredHosts} disabled={dialogBusy} rows={3} onChange={(_, data) => setDraft({ ...draft, preferredHosts: data.value })} />
                </Field>
                <details {...(draft.selector.trim() ? { open: true } : {})}>
                  <summary style={{ cursor: "pointer" }}>{t("runLocationSelectorAdvanced")}</summary>
                  <Field label={t("runLocationSelector")} hint={t("runLocationSelectorHelp")} validationState={draftErrors.selector ? "error" : undefined} validationMessage={draftErrors.selector ? t(draftErrors.selector) : undefined}>
                    <Textarea className={styles.code} value={draft.selector} disabled={dialogBusy} rows={3} placeholder="accelerator=gpu" onChange={(_, data) => setDraft({ ...draft, selector: data.value })} />
                  </Field>
                </details>
                <Field hint={t("runLocationEnabledHelp")}>
                  <Checkbox checked={draft.enabled} disabled={dialogBusy} label={`${t("runLocationEnabled")} · ${draft.enabled ? t("enabled") : t("disabled")}`} onChange={(_, data) => setDraft({ ...draft, enabled: data.checked === true })} />
                </Field>
              </>}
            </DialogContent>
            <DialogActions>
              <Button appearance="secondary" disabled={dialogBusy} onClick={() => setDraft(null)}>{t("cancel")}</Button>
              <SaveButton type="submit" disabled={dialogBusy}>{dialogBusy ? t("saving") : t("saveLocation")}</SaveButton>
            </DialogActions>
          </DialogBody>
        </form>
      </DialogSurface>
    </Dialog>
    <ConfirmDialog
      open={deleteTarget !== null}
      title={t("deleteRunLocation")}
      description={t("deleteRunLocationConfirm")}
      details={deleteTarget && <strong>{deleteTarget.display_name}</strong>}
      confirmLabel={t("deleteRunLocation")}
      cancelLabel={t("cancel")}
      busy={dialogBusy}
      danger
      onClose={() => setDeleteTarget(null)}
      onConfirm={() => void confirmDelete()}
    />
  </AdminCard>;
}

type T = (key: MessageKey) => string;

function columns(
  t: T,
  busyName: string | null,
  onEdit: (pool: NodePool) => void,
  onToggle: (pool: NodePool) => void,
  onDelete: (pool: NodePool) => void,
): TableColumnDefinition<NodePool>[] {
  return [
    { columnId: "displayName", compare: (a, b) => a.display_name.localeCompare(b.display_name), renderHeaderCell: () => t("displayName"), renderCell: (pool) => pool.display_name },
    { columnId: "name", compare: (a, b) => a.name.localeCompare(b.name), renderHeaderCell: () => t("runLocationId"), renderCell: (pool) => <Text font="monospace">{pool.name}</Text> },
    { columnId: "rules", compare: () => 0, renderHeaderCell: () => t("runLocationRules"), renderCell: (pool) => rulesSummary(pool.placement, t) },
    { columnId: "state", compare: (a, b) => Number(a.enabled) - Number(b.enabled), renderHeaderCell: () => t("enabled"), renderCell: (pool) => pool.enabled ? t("enabled") : t("disabled") },
    {
      columnId: "actions", compare: () => 0, renderHeaderCell: () => t("actions"), renderCell: (pool) => {
        const busy = busyName === pool.name;
        return <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <Button size="small" disabled={busyName !== null} onClick={() => onEdit(pool)}>{t("editRunLocation")}</Button>
          <Button size="small" disabled={busyName !== null} icon={busy ? <Spinner size="tiny" /> : undefined} onClick={() => onToggle(pool)}>{pool.enabled ? t("pluginDisable") : t("pluginEnable")}</Button>
          {!pool.enabled && pool.name !== DEFAULT_LOCATION && <Button size="small" disabled={busyName !== null} onClick={() => onDelete(pool)}>{t("deleteRunLocation")}</Button>}
        </div>;
      },
    },
  ];
}

function rulesSummary(placement: NodePoolPlacement, t: T): string {
  const parts: string[] = [];
  if (placement.required_hosts.length > 0) parts.push(`${placement.required_hosts.length} ${t("runLocationRulesRequired")}`);
  if (placement.preferred_hosts.length > 0) parts.push(`${placement.preferred_hosts.length} ${t("runLocationRulesPreferred")}`);
  const labels = Object.keys(placement.selector).length;
  if (labels > 0) parts.push(`${labels} ${t("runLocationRulesLabels")}`);
  return parts.length > 0 ? parts.join(" · ") : t("runLocationRulesNone");
}

const NAME_PATTERN = /^[a-z][a-z0-9-]{0,62}$/;
const HOST_PATTERN = /^[A-Za-z0-9._-]{1,63}$/;
const LABEL_KEY_PATTERN = /^[A-Za-z0-9/_\.-]{1,253}$/;
const LABEL_VALUE_PATTERN = /^[A-Za-z0-9._-]{0,63}$/;
// The backend caps display names at 80 UTF-8 bytes and rejects control characters.
const DISPLAY_NAME_MAX_BYTES = 80;
const CONTROL_CHARACTER = /\p{Cc}/u;

function validLocationName(value: string): boolean {
  return NAME_PATTERN.test(value) && /[a-z0-9]$/.test(value);
}

export function formatSelector(selector: Record<string, string>): string {
  return Object.entries(selector).map(([key, value]) => `${key}=${value}`).join("\n");
}

export function parseSelectorLines(value: string): Record<string, string> | null {
  // A null-prototype map keeps constructor, toString, and __proto__ usable as label keys.
  const selector: Record<string, string> = Object.create(null);
  for (const line of value.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const separator = trimmed.indexOf("=");
    if (separator <= 0) return null;
    const key = trimmed.slice(0, separator).trim();
    const label = trimmed.slice(separator + 1).trim();
    if (!LABEL_KEY_PATTERN.test(key) || !LABEL_VALUE_PATTERN.test(label) || Object.hasOwn(selector, key)) return null;
    selector[key] = label;
  }
  return Object.keys(selector).length <= 32 ? selector : null;
}

export function parseHostLines(value: string): string[] | null {
  const hosts: string[] = [];
  for (const line of value.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (!HOST_PATTERN.test(trimmed) || hosts.includes(trimmed)) return null;
    hosts.push(trimmed);
  }
  return hosts.length <= 128 ? hosts : null;
}

function parseDraft(draft: LocationDraft): { input: PutNodePoolInput } | { errors: Partial<Record<"name" | "displayName" | "hosts" | "selector", MessageKey>> } {
  const errors: Partial<Record<"name" | "displayName" | "hosts" | "selector", MessageKey>> = {};
  if (draft.creating && !validLocationName(draft.name)) errors.name = "runLocationNameInvalid";
  const displayName = draft.displayName;
  if (!displayName.trim() || displayName !== displayName.trim() || new TextEncoder().encode(displayName).length > DISPLAY_NAME_MAX_BYTES || CONTROL_CHARACTER.test(displayName)) errors.displayName = "runLocationDisplayNameInvalid";
  const requiredHosts = parseHostLines(draft.requiredHosts);
  const preferredHosts = parseHostLines(draft.preferredHosts);
  if (requiredHosts === null || preferredHosts === null) errors.hosts = "runLocationHostsInvalid";
  const selector = parseSelectorLines(draft.selector);
  if (selector === null) errors.selector = "runLocationSelectorInvalid";
  if (Object.keys(errors).length > 0) return { errors };
  return {
    input: {
      display_name: displayName,
      enabled: draft.enabled,
      placement: { selector: selector ?? {}, required_hosts: requiredHosts ?? [], preferred_hosts: preferredHosts ?? [] },
    },
  };
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
