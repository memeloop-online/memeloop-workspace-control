import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type {
  InjectionDraft,
  InjectionKind,
  InjectionScope,
  Principal,
  ResolvedInjection,
  StoredInjection,
  WorkspaceResponse,
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
  const [scope, setScope] = useState<InjectionScope>("user");
  const [workspaceId, setWorkspaceId] = useState(props.workspaces[0]?.workspace.id ?? "");
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [draft, setDraft] = useState<InjectionDraft>({ ...EMPTY_DRAFT });
  const [selectorLabels, setSelectorLabels] = useState("{}");
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
    if (!workspaceId && props.workspaces[0]) {
      setWorkspaceId(props.workspaces[0].workspace.id);
    }
  }, [props.workspaces, workspaceId]);

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
      const labels = JSON.parse(selectorLabels) as unknown;
      if (!labels || Array.isArray(labels) || typeof labels !== "object" || Object.values(labels).some((value) => typeof value !== "string")) {
        throw new Error(t("operationFailed"));
      }
      await props.api.replaceInjection(scope, scopeId, {
        ...draft,
        labels: labels as Record<string, string>,
        locked: scope === "organization" && draft.locked,
      });
      setDraft({ ...EMPTY_DRAFT, value: { ...EMPTY_DRAFT.value } });
      setSelectorLabels("{}");
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
        {(["organization", "user", "workspace"] as InjectionScope[]).map((value) => (
          <button className={scope === value ? "active" : ""} onClick={() => setScope(value)} key={value}>
            {value === "organization" ? t("scopeOrganization") : value === "user" ? t("scopeUser") : t("scopeWorkspace")}
          </button>
        ))}
      </div>
      {scope === "workspace" && (
        <label className="standalone-label">{t("workspaces")}<select value={workspaceId} onChange={(event) => setWorkspaceId(event.target.value)}><option value="">{t("choose")}</option>{props.workspaces.map(({ workspace }) => <option value={workspace.id} key={workspace.id}>{workspace.name} · {workspace.short_id}</option>)}</select></label>
      )}

      <div className="injection-layout">
        <div className="injection-list">
          <h3>{t("savedCredentials")}</h3>
          <input className="credential-search" type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchCredentials")} aria-label={t("searchCredentials")} />
          <div className="credential-scroll">
          {filteredItems.length === 0 && <div className="empty compact">{t("noCredentials")}</div>}
          {filteredItems.map((item) => (
            <button className="injection-row" key={item.key} onClick={() => { setDraft({ ...EMPTY_DRAFT, key: item.key, kind: item.kind, target: item.target, sensitive: item.sensitive, locked: item.locked, file_mode: item.file_mode, owner: item.owner, group: item.group, template_selector: item.template_selector, labels: item.labels }); setSelectorLabels(JSON.stringify(item.labels)); }}>
              <span className="kind-icon">{kindGlyph(item.kind)}</span>
              <span><strong>{item.key}</strong><small>{item.target}</small></span>
              <span className="version">v{item.version}{item.locked ? ` · ${t("locked")}` : ""}</span>
            </button>
          ))}
          </div>
        </div>

        <form className="editor-card" onSubmit={save}>
          <div className="editor-grid">
            <label>{t("key")}<input required value={draft.key} onChange={(e) => changeKey(e.target.value)} /></label>
            <label>{t("type")}<select value={draft.kind} onChange={(e) => changeKind(e.target.value as InjectionKind)}><option value="environment_variable">{t("environmentVariable")}</option><option value="config_file">{t("configFile")}</option><option value="secret_file">{t("credentialFile")}</option><option value="ssh_public_key">{t("sshPublicKey")}</option></select></label>
            <label>{t("encoding")}<select value={draft.value.encoding} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, encoding: e.target.value as "utf8" | "base64" } })}><option value="utf8">{t("multilineUtf8")}</option><option value="base64">{t("base64Binary")}</option></select></label>
            {draft.kind !== "environment_variable" && <label>{t("fileMode")}<input value={draft.file_mode === null ? "" : draft.file_mode.toString(8)} onChange={(e) => setDraft({ ...draft, file_mode: e.target.value ? Number.parseInt(e.target.value, 8) : null })} placeholder="600" /></label>}
            <label className="wide">{t("target")}<input required readOnly={draft.kind === "ssh_public_key"} value={draft.target} onChange={(e) => setDraft({ ...draft, target: e.target.value })} /></label>
            {draft.kind === "ssh_public_key" && <p className="profile-note wide">{t("sshTargetHelp")}</p>}
            {draft.kind !== "environment_variable" && <><label>{t("owner")}<input value={draft.owner ?? ""} onChange={(e) => setDraft({ ...draft, owner: e.target.value || null })} placeholder="workspace" /></label><label>{t("group")}<input value={draft.group ?? ""} onChange={(e) => setDraft({ ...draft, group: e.target.value || null })} placeholder="workspace" /></label></>}
            <label>{t("templateSelector")}<input value={draft.template_selector ?? ""} onChange={(e) => setDraft({ ...draft, template_selector: e.target.value || null })} placeholder={t("templateSelectorHint")} /></label>
            <label>{t("labelSelector")}<input value={selectorLabels} onChange={(e) => setSelectorLabels(e.target.value)} placeholder={'{"access_mode":"public"}'} /></label>
            <label className="wide">{draft.value.encoding === "base64" ? t("valueBase64") : t("valueMultiline")}<textarea rows={15} spellCheck={false} value={draft.value.value} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, value: e.target.value } })} placeholder={draft.value.encoding === "base64" ? t("base64Hint") : t("multilineHint")} /></label>
          </div>
          <div className="check-row">
            <label><input type="checkbox" checked={draft.sensitive} onChange={(e) => setDraft({ ...draft, sensitive: e.target.checked })} />{t("sensitiveValue")}</label>
            {scope === "organization" && <label><input type="checkbox" checked={draft.locked} onChange={(e) => setDraft({ ...draft, locked: e.target.checked })} />{t("locked")}</label>}
          </div>
          <p className="security-note">{t("credentialWriteOnly")}</p>
          <button className="button primary" disabled={saving || !scopeId}>{saving ? t("savingEncrypted") : t("saveEncrypted")}</button>
        </form>
      </div>

      {preview.length > 0 && <div className="preview-card"><h3>{t("resolvedSources")}</h3><div className="preview-grid">{preview.map((item) => <div key={item.key}><strong>{item.key}</strong><span>{item.source === "organization" ? t("fromOrganization") : item.source === "user" ? t("fromUser") : t("fromWorkspace")}</span><small>{item.target}{item.locked ? ` · ${t("locked")}` : ""}</small></div>)}</div></div>}
    </section>
  );
}

function kindGlyph(kind: InjectionKind) {
  return kind === "environment_variable" ? "ENV" : kind === "ssh_public_key" ? "SSH" : kind === "secret_file" ? "SEC" : "CFG";
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
