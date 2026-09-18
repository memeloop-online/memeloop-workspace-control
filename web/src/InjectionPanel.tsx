import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Badge,
  Button,
  Card,
  CardHeader,
  Divider,
  Tab,
  TabList,
  Text,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { EyeRegular } from "@fluentui/react-icons";

import type { ApiClient } from "./api";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { Page } from "./design-system/Page";
import { WorkspaceCombobox } from "./forms/WorkspaceCombobox";
import { useI18n } from "./i18n";
import { canManageOrganization as mayManageOrganization } from "./permissions";
import { CredentialScopeTabs } from "./shared";
import type {
  InjectionScope,
  Principal,
  ResolvedInjection,
  StoredInjection,
  WorkspaceResponse,
  WorkspaceTemplate,
} from "./types";
import { InjectionEditorForm } from "./injections/InjectionEditorForm";
import { InjectionList } from "./injections/InjectionList";
import { draftFromStored, emptyInjectionDraft, injectionDraftForSave } from "./injections/editorModel";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  workspaces: WorkspaceResponse[];
  onError: (message: string) => void;
}

const useStyles = makeStyles({
  controls: {
    display: "grid",
    gap: tokens.spacingVerticalS,
    justifyItems: "start",
  },
  workspace: {
    width: "100%",
  },
  mobileTabs: {
    display: "none",
    [`@media (max-width: 760px)`]: {
      display: "flex",
      width: "100%",
    },
  },
  layout: {
    display: "grid",
    gridTemplateColumns: "minmax(17rem, 0.8fr) minmax(0, 1.4fr)",
    gap: tokens.spacingVerticalL,
    alignItems: "start",
    [`@media (max-width: 900px)`]: {
      gridTemplateColumns: "minmax(15rem, 0.9fr) minmax(0, 1.2fr)",
    },
    [`@media (max-width: 760px)`]: {
      display: "block",
    },
  },
  list: {
    minWidth: 0,
    [`@media (max-width: 760px)`]: {
      display: "block",
    },
  },
  editor: {
    minWidth: 0,
    [`@media (max-width: 760px)`]: {
      marginTop: tokens.spacingVerticalL,
    },
  },
  mobileHidden: {
    [`@media (max-width: 760px)`]: {
      display: "none",
    },
  },
  preview: {
    display: "grid",
    gap: tokens.spacingVerticalM,
  },
  previewGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(14rem, 1fr))",
    gap: tokens.spacingVerticalS,
  },
  previewItem: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
    minWidth: 0,
    padding: tokens.spacingHorizontalM,
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground2,
  },
  previewWrap: {
    overflowWrap: "anywhere",
  },
  previewMeta: {
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
});

export function InjectionPanel(props: Props) {
  const { t } = useI18n();
  const styles = useStyles();
  const canManageOrganization = mayManageOrganization(props.principal, props.organizationId, "manage_organization");
  const scopeValues: InjectionScope[] = [...(canManageOrganization ? ["organization" as const] : []), "user", "workspace"];
  const [scope, setScope] = useState<InjectionScope>("user");
  const [workspaceId, setWorkspaceId] = useState(() => props.workspaces.find((item) => !item.workspace.organization_id || item.workspace.organization_id === props.organizationId)?.workspace.id ?? "");
  const workspaceSelectionTouchedRef = useRef(false);
  const previousOrganizationIdRef = useRef(props.organizationId);
  const injectionLoadRequestRef = useRef(0);
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [draft, setDraft] = useState(emptyInjectionDraft);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [preview, setPreview] = useState<ResolvedInjection[]>([]);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewRan, setPreviewRan] = useState(false);
  const [saving, setSaving] = useState(false);
  const [search, setSearch] = useState("");
  const [mobilePane, setMobilePane] = useState<"list" | "editor">("list");
  const [confirmDelete, setConfirmDelete] = useState(false);

  const scopeId = useMemo(() => {
    if (scope === "organization") return props.organizationId;
    if (scope === "user") return props.principal.user_id;
    return workspaceId;
  }, [scope, workspaceId, props.organizationId, props.principal.user_id]);
  const workspaceItems = useMemo(
    () => props.workspaces.filter((item) => !item.workspace.organization_id || item.workspace.organization_id === props.organizationId),
    [props.workspaces, props.organizationId],
  );
  const searchWorkspaces = useCallback(
    (query: string) => props.api.workspacesPage(props.organizationId, { limit: 30, search: query.trim() || undefined }).then((page) => page.items),
    [props.api, props.organizationId],
  );

  const load = useCallback(async () => {
    const requestId = ++injectionLoadRequestRef.current;
    if (!scopeId) {
      setItems([]);
      return;
    }
    try {
      const next = await props.api.injections(scope, scopeId);
      if (injectionLoadRequestRef.current === requestId) setItems(next);
    } catch (error) {
      if (injectionLoadRequestRef.current === requestId) props.onError(message(error, t("operationFailed")));
    }
  }, [props.api, props.onError, scope, scopeId]);

  useEffect(() => {
    if (previousOrganizationIdRef.current === props.organizationId) return;
    previousOrganizationIdRef.current = props.organizationId;
    workspaceSelectionTouchedRef.current = false;
    setScope("user");
    setWorkspaceId("");
    setItems([]);
    setTemplates([]);
    setDraft(emptyInjectionDraft());
    setSelectedKey(null);
    setPreview([]);
    setPreviewRan(false);
    setSearch("");
    setMobilePane("list");
  }, [props.organizationId]);

  useEffect(() => {
    if (workspaceId || workspaceSelectionTouchedRef.current || workspaceItems.length === 0) return;
    setWorkspaceId(workspaceItems[0].workspace.id);
  }, [workspaceId, workspaceItems]);

  useEffect(() => { void load(); }, [load]);

  useEffect(() => {
    let active = true;
    props.api.templates(props.organizationId)
      .then((value) => { if (active) setTemplates(value.filter((item) => item.enabled)); })
      .catch((error) => active && props.onError(message(error, t("operationFailed"))));
    return () => { active = false; };
  }, [props.api, props.organizationId, props.onError]);

  function resetDraft() {
    setSelectedKey(null);
    setDraft(emptyInjectionDraft());
    setMobilePane("list");
  }

  function changeScope(value: InjectionScope) {
    setScope(value);
    resetDraft();
    setPreview([]);
    setPreviewRan(false);
    setSearch("");
  }

  async function selectItem(item: StoredInjection) {
    if (selectedKey === item.key) {
      resetDraft();
      return;
    }
    setSelectedKey(item.key);
    const selected = draftFromStored(item);
    setDraft(selected);
    setMobilePane("editor");
    if (item.sensitive || item.kind === "secret_file" || !scopeId) return;
    try {
      const value = await props.api.injectionValue(scope, scopeId, item.key);
      setDraft((current) => current.key === item.key
        ? { ...current, value, storedValueAvailable: true }
        : current);
    } catch (error) {
      props.onError(message(error, t("operationFailed")));
    }
  }

  function startNew() {
    setSelectedKey(null);
    setDraft(emptyInjectionDraft());
    setMobilePane("editor");
  }

  async function save() {
    if (!scopeId) return;
    setSaving(true);
    try {
      const item = injectionDraftForSave(draft);
      await props.api.replaceInjection(scope, scopeId, { ...item, locked: scope === "organization" && draft.locked });
      resetDraft();
      await load();
    } catch (error) {
      props.onError(error instanceof Error && error.message === "invalid_file_mode" ? t("invalidFileMode") : message(error, t("operationFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!selectedKey || !scopeId) return;
    setSaving(true);
    try {
      await props.api.deleteInjection(scope, scopeId, selectedKey);
      resetDraft();
      setConfirmDelete(false);
      await load();
    } catch (error) {
      props.onError(message(error, t("operationFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function runPreview() {
    if (previewBusy) return;
    setPreviewBusy(true);
    try {
      const inline = draft.key && scope === "workspace" ? [injectionDraftForSave(draft)] : [];
      setPreview(await props.api.previewInjections({ organization_id: props.organizationId, user_id: props.principal.user_id, workspace_id: workspaceId || null, inline_workspace_injections: inline }));
      setPreviewRan(true);
    } catch (error) {
      props.onError(message(error, t("operationFailed")));
    } finally {
      setPreviewBusy(false);
    }
  }

  return (
    <Page title={t("credentialsTitle")} actions={<Button type="button" appearance="secondary" icon={previewBusy ? undefined : <EyeRegular aria-hidden="true" />} disabled={!workspaceId || previewBusy} onClick={() => void runPreview()}>{previewBusy ? t("credentialsPreviewLoading") : t("credentialsPreview")}</Button>}>
      <div className={styles.controls}>
        <CredentialScopeTabs
          scopes={scopeValues}
          selected={scope}
          labels={{ organization: t("scopeOrganization"), user: t("scopeUser"), workspace: t("scopeWorkspace") }}
          ariaLabel={t("credentials")}
          onChange={changeScope}
        />
        <div className={styles.workspace}><WorkspaceCombobox key={props.organizationId} items={workspaceItems} loadItems={searchWorkspaces} selectedId={workspaceId} onChange={(id) => { workspaceSelectionTouchedRef.current = id === ""; setWorkspaceId(id); setPreview([]); setPreviewRan(false); }} /></div>
      </div>
      <TabList className={styles.mobileTabs} selectedValue={mobilePane} onTabSelect={(_, data) => data.value === "editor" ? (selectedKey ? setMobilePane("editor") : startNew()) : setMobilePane("list")} aria-label={t("credentials")}>
        <Tab value="list">{t("mobileTabList")}</Tab>
        <Tab value="editor">{selectedKey ? t("mobileTabEdit") : t("mobileTabNew")}</Tab>
      </TabList>
      <div className={styles.layout}>
        <div className={`${styles.list} ${mobilePane === "editor" ? styles.mobileHidden : ""}`}>
          <InjectionList items={items} selectedKey={selectedKey} search={search} title={t("savedCredentials")} emptyLabel={t("noCredentials")} onSearchChange={setSearch} onSelect={selectItem} />
        </div>
        <Card className={`${styles.editor} ${mobilePane === "list" ? styles.mobileHidden : ""}`} appearance="outline">
          <InjectionEditorForm draft={draft} update={setDraft} scope={scope} templates={templates} selectedKey={selectedKey} saving={saving} disabled={!scopeId} onReset={resetDraft} onSubmit={save} onDelete={() => setConfirmDelete(true)} />
        </Card>
      </div>
      {(preview.length > 0 || previewRan) && (
        <Card className={styles.preview} appearance="outline">
          <CardHeader header={<Text weight="semibold">{t("resolvedSources")}</Text>} description={<Badge appearance="tint">{preview.length}</Badge>} />
          <Divider />
          {preview.length === 0 ? <Text className={styles.previewMeta} role="status">{t("credentialsPreviewEmpty")}</Text> : <div className={styles.previewGrid}>
            {preview.map((item) => <div className={styles.previewItem} key={item.key}><Text className={styles.previewWrap} weight="semibold">{item.key}</Text><Text className={styles.previewMeta}>{item.source === "organization" ? t("fromOrganization") : item.source === "user" ? t("fromUser") : t("fromWorkspace")}</Text><Text className={`${styles.previewMeta} ${styles.previewWrap}`}>{item.target}{item.locked ? ` · ${t("lockedState")}` : ""}</Text></div>)}
          </div>}
        </Card>
      )}
      <ConfirmDialog open={confirmDelete} title={t("delete")} description={t("deleteCredentialConfirm")} confirmLabel={t("delete")} cancelLabel={t("cancel")} busy={saving} danger details={selectedKey && <code>{selectedKey}</code>} onClose={() => setConfirmDelete(false)} onConfirm={() => void remove()} />
    </Page>
  );
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
