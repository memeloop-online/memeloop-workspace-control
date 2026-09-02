import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ApiClient } from "./api";
import { WorkspaceCombobox } from "./forms/WorkspaceCombobox";
import { useI18n } from "./i18n";
import { canManageOrganization as mayManageOrganization } from "./permissions";
import type {
  InjectionKind,
  InjectionScope,
  Principal,
  ResolvedInjection,
  StoredInjection,
  WorkspaceResponse,
  WorkspaceTemplate,
} from "./types";
import { InjectionEditorForm } from "./injections/InjectionEditorForm";
import {
  draftFromStored,
  emptyInjectionDraft,
  injectionDraftForSave,
} from "./injections/editorModel";

interface Props {
  api: ApiClient;
  principal: Principal;
  organizationId: string;
  workspaces: WorkspaceResponse[];
  onError: (message: string) => void;
}

export function InjectionPanel(props: Props) {
  const { t } = useI18n();
  const scopeTabsId = useId();
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
  const [saving, setSaving] = useState(false);
  const [search, setSearch] = useState("");
  const [mobilePane, setMobilePane] = useState<"list" | "editor">("list");

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
  const workspaceItems = useMemo(
    () => props.workspaces.filter((item) => !item.workspace.organization_id || item.workspace.organization_id === props.organizationId),
    [props.workspaces, props.organizationId],
  );
  const searchWorkspaces = useCallback(
    (query: string) => props.api
      .workspacesPage(props.organizationId, { limit: 30, search: query.trim() || undefined })
      .then((page) => page.items),
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
      if (injectionLoadRequestRef.current === requestId) props.onError(message(error));
    }
  }, [props.api, props.onError, scope, scopeId, props.organizationId]);

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
    setSearch("");
    setMobilePane("list");
  }, [props.organizationId]);

  useEffect(() => {
    if (workspaceId || workspaceSelectionTouchedRef.current || workspaceItems.length === 0) return;
    setWorkspaceId(workspaceItems[0].workspace.id);
  }, [workspaceId, workspaceItems]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    let active = true;
    props.api.templates(props.organizationId)
      .then((value) => { if (active) setTemplates(value.filter((item) => item.enabled)); })
      .catch((error) => active && props.onError(message(error)));
    return () => { active = false; };
  }, [props.api, props.organizationId]);

  function resetDraft() {
    setSelectedKey(null);
    setDraft(emptyInjectionDraft());
    setMobilePane("list");
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
    setDraft(draftFromStored(item));
    setMobilePane("editor");
  }

  function startNew() {
    setSelectedKey(null);
    setDraft(emptyInjectionDraft());
    setMobilePane("editor");
  }

  function scopeKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>, current: InjectionScope) {
    const currentIndex = scopeValues.indexOf(current);
    let nextIndex = currentIndex;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") nextIndex = (currentIndex + 1) % scopeValues.length;
    else if (event.key === "ArrowLeft" || event.key === "ArrowUp") nextIndex = (currentIndex - 1 + scopeValues.length) % scopeValues.length;
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = scopeValues.length - 1;
    else return;
    event.preventDefault();
    const next = scopeValues[nextIndex];
    changeScope(next);
    requestAnimationFrame(() => document.getElementById(`${scopeTabsId}-${next}`)?.focus());
  }

  async function save() {
    if (!scopeId) return;
    setSaving(true);
    try {
      const item = injectionDraftForSave(draft);
      await props.api.replaceInjection(scope, scopeId, {
        ...item,
        locked: scope === "organization" && draft.locked,
      });
      resetDraft();
      await load();
    } catch (error) {
      props.onError(error instanceof Error && error.message === "invalid_file_mode" ? t("invalidFileMode") : message(error));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!selectedKey || !scopeId || !confirm(t("deleteCredentialConfirm"))) return;
    setSaving(true);
    try {
      await props.api.deleteInjection(scope, scopeId, selectedKey);
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
      const inline = draft.key && scope === "workspace" ? [injectionDraftForSave(draft)] : [];
      setPreview(
        await props.api.previewInjections({
          organization_id: props.organizationId,
          user_id: props.principal.user_id,
          workspace_id: workspaceId || null,
          inline_workspace_injections: inline,
        }),
      );
    } catch (error) {
      props.onError(message(error));
    }
  }

  return (
    <section className="panel-stack">
      <div className="section-heading">
        <div><p className="eyebrow">{t("credentials")}</p><h2>{t("credentialsTitle")}</h2></div>
        <button className="button" onClick={() => void runPreview()}>{t("credentialsPreview")}</button>
      </div>
      <div className="scope-tabs" role="tablist" aria-label={t("credentials")}>
        {scopeValues.map((value) => (
          <button id={`${scopeTabsId}-${value}`} role="tab" aria-selected={scope === value} aria-controls={`${scopeTabsId}-panel`} tabIndex={scope === value ? 0 : -1} className={scope === value ? "active" : ""} onClick={() => changeScope(value)} onKeyDown={(event) => scopeKeyDown(event, value)} key={value}>
            {value === "organization" ? t("scopeOrganization") : value === "user" ? t("scopeUser") : t("scopeWorkspace")}
          </button>
        ))}
      </div>
      <div id={`${scopeTabsId}-panel`} role="tabpanel" aria-labelledby={`${scopeTabsId}-${scope}`} className="injection-scope-panel">
        {scope === "workspace" && (
          <WorkspaceCombobox
            key={props.organizationId}
            items={workspaceItems}
            loadItems={searchWorkspaces}
            selectedId={workspaceId}
            onChange={(id) => {
              workspaceSelectionTouchedRef.current = id === "";
              setWorkspaceId(id);
            }}
          />
        )}
        <div className="mobile-master-detail" aria-label={t("credentials")}>
          <button type="button" aria-pressed={mobilePane === "list"} onClick={() => setMobilePane("list")}>{t("savedCredentials")}</button>
          <button type="button" aria-pressed={mobilePane === "editor"} onClick={() => selectedKey ? setMobilePane("editor") : startNew()}>{selectedKey ? t("editingCredential") : t("newCredential")}</button>
        </div>
        <div className="injection-layout">
        <div className={`injection-list${mobilePane === "list" ? "" : " mobile-pane-hidden"}`}>
          <h3>{t("savedCredentials")}</h3>
          <input className="credential-search" type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchCredentials")} aria-label={t("searchCredentials")} />
          <p className="sr-only" role="status" aria-live="polite">{filteredItems.length === 0 ? t("noCredentials") : `${t("savedCredentials")}: ${filteredItems.length}`}</p>
          <div className="credential-scroll" aria-label={t("savedCredentials")}>
          {filteredItems.length === 0 && <div className="empty compact" role="status">{t("noCredentials")}</div>}
          {filteredItems.map((item) => (
            <button className={`injection-row${selectedKey === item.key ? " selected" : ""}`} aria-pressed={selectedKey === item.key} key={item.key} onClick={() => selectItem(item)}>
              <span className="kind-icon" title={kindLabel(item.kind, t)} aria-label={kindLabel(item.kind, t)}>{kindShortLabel(item.kind, t)}</span>
              <span><strong>{item.key}</strong><small>{item.target}</small></span>
              <span className="version">v{item.version}{item.locked ? ` · ${t("locked")}` : ""}</span>
            </button>
          ))}
          </div>
        </div>

        <div className={`injection-editor-pane${mobilePane === "editor" ? "" : " mobile-pane-hidden"}`}>
          <InjectionEditorForm draft={draft} update={setDraft} scope={scope} templates={templates} selectedKey={selectedKey} saving={saving} disabled={!scopeId} onReset={resetDraft} onSubmit={save} onDelete={remove} />
        </div>
        </div>
      </div>

      {preview.length > 0 && <div className="preview-card"><h3>{t("resolvedSources")}</h3><div className="preview-grid">{preview.map((item) => <div key={item.key}><strong>{item.key}</strong><span>{item.source === "organization" ? t("fromOrganization") : item.source === "user" ? t("fromUser") : t("fromWorkspace")}</span><small>{item.target}{item.locked ? ` · ${t("locked")}` : ""}</small></div>)}</div></div>}
    </section>
  );
}

function kindShortLabel(kind: InjectionKind, t: ReturnType<typeof useI18n>["t"]) {
  return kind === "environment_variable" ? t("kindShortEnvironment") : kind === "ssh_public_key" ? t("kindShortSsh") : kind === "secret_file" ? t("kindShortSecret") : t("kindShortConfig");
}

function kindLabel(kind: InjectionKind, t: ReturnType<typeof useI18n>["t"]) {
  return kind === "environment_variable" ? t("environmentVariable") : kind === "ssh_public_key" ? t("sshPublicKey") : kind === "secret_file" ? t("credentialFile") : t("configFile");
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "操作失败";
}
