import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import type { ApiClient } from "./api";
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
  const [scope, setScope] = useState<InjectionScope>("organization");
  const [workspaceId, setWorkspaceId] = useState(props.workspaces[0]?.workspace.id ?? "");
  const [items, setItems] = useState<StoredInjection[]>([]);
  const [draft, setDraft] = useState<InjectionDraft>({ ...EMPTY_DRAFT });
  const [selectorLabels, setSelectorLabels] = useState("{}");
  const [preview, setPreview] = useState<ResolvedInjection[]>([]);
  const [saving, setSaving] = useState(false);

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
          ? "/workspace/.mwc/injected-key.pub"
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

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!scopeId) return;
    setSaving(true);
    try {
      const labels = JSON.parse(selectorLabels) as unknown;
      if (!labels || Array.isArray(labels) || typeof labels !== "object" || Object.values(labels).some((value) => typeof value !== "string")) {
        throw new Error("标签选择必须是字符串键值组成的 JSON 对象");
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
        <div><p className="eyebrow">INJECTIONS</p><h2>Secret 与文件级联</h2></div>
        <button className="button" onClick={() => void runPreview()}>预览最终来源</button>
      </div>
      <div className="scope-tabs">
        {(["organization", "user", "workspace"] as InjectionScope[]).map((value) => (
          <button className={scope === value ? "active" : ""} onClick={() => setScope(value)} key={value}>
            {value === "organization" ? "组织级" : value === "user" ? "用户级" : "工作区级"}
          </button>
        ))}
      </div>
      {scope === "workspace" && (
        <label className="standalone-label">工作区<select value={workspaceId} onChange={(event) => setWorkspaceId(event.target.value)}><option value="">请选择</option>{props.workspaces.map(({ workspace }) => <option value={workspace.id} key={workspace.id}>{workspace.name} · {workspace.short_id}</option>)}</select></label>
      )}

      <div className="injection-layout">
        <div className="injection-list">
          <h3>已保存条目</h3>
          {items.length === 0 && <div className="empty compact">没有条目</div>}
          {items.map((item) => (
            <button className="injection-row" key={item.key} onClick={() => { setDraft({ ...EMPTY_DRAFT, key: item.key, kind: item.kind, target: item.target, sensitive: item.sensitive, locked: item.locked, file_mode: item.file_mode, owner: item.owner, group: item.group, template_selector: item.template_selector, labels: item.labels }); setSelectorLabels(JSON.stringify(item.labels)); }}>
              <span className="kind-icon">{kindGlyph(item.kind)}</span>
              <span><strong>{item.key}</strong><small>{item.target}</small></span>
              <span className="version">v{item.version}{item.locked ? " · 锁定" : ""}</span>
            </button>
          ))}
        </div>

        <form className="editor-card" onSubmit={save}>
          <div className="editor-grid">
            <label>键<input required value={draft.key} onChange={(e) => setDraft({ ...draft, key: e.target.value })} /></label>
            <label>类型<select value={draft.kind} onChange={(e) => changeKind(e.target.value as InjectionKind)}><option value="environment_variable">环境变量</option><option value="config_file">普通配置文件</option><option value="secret_file">敏感文件</option><option value="ssh_public_key">SSH 公钥</option></select></label>
            <label>编码<select value={draft.value.encoding} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, encoding: e.target.value as "utf8" | "base64" } })}><option value="utf8">多行 UTF-8</option><option value="base64">Base64 二进制</option></select></label>
            {draft.kind !== "environment_variable" && <label>权限（八进制）<input value={draft.file_mode === null ? "" : draft.file_mode.toString(8)} onChange={(e) => setDraft({ ...draft, file_mode: e.target.value ? Number.parseInt(e.target.value, 8) : null })} placeholder="600" /></label>}
            <label className="wide">目标<input required value={draft.target} onChange={(e) => setDraft({ ...draft, target: e.target.value })} /></label>
            {draft.kind !== "environment_variable" && <><label>属主<input value={draft.owner ?? ""} onChange={(e) => setDraft({ ...draft, owner: e.target.value || null })} placeholder="workspace" /></label><label>属组<input value={draft.group ?? ""} onChange={(e) => setDraft({ ...draft, group: e.target.value || null })} placeholder="workspace" /></label></>}
            <label>模板选择<input value={draft.template_selector ?? ""} onChange={(e) => setDraft({ ...draft, template_selector: e.target.value || null })} placeholder="模板 UUID 或 *" /></label>
            <label>标签选择（JSON）<input value={selectorLabels} onChange={(e) => setSelectorLabels(e.target.value)} placeholder={'{"access_mode":"public"}'} /></label>
            <label className="wide">{draft.value.encoding === "base64" ? "值（Base64）" : "值（多行 UTF-8 / JSON / YAML / PEM）"}<textarea rows={15} spellCheck={false} value={draft.value.value} onChange={(e) => setDraft({ ...draft, value: { ...draft.value, value: e.target.value } })} placeholder={draft.value.encoding === "base64" ? "Base64 编码内容" : "保留空行、缩进与末尾换行"} /></label>
          </div>
          <div className="check-row">
            <label><input type="checkbox" checked={draft.sensitive} onChange={(e) => setDraft({ ...draft, sensitive: e.target.checked })} />敏感值</label>
            {scope === "organization" && <label><input type="checkbox" checked={draft.locked} onChange={(e) => setDraft({ ...draft, locked: e.target.checked })} />禁止下层覆盖</label>}
          </div>
          <p className="security-note">保存后值不可读取，只能整体替换。界面与审计仅展示元数据和版本。</p>
          <button className="button primary" disabled={saving || !scopeId}>{saving ? "加密保存中…" : "加密并替换"}</button>
        </form>
      </div>

      {preview.length > 0 && <div className="preview-card"><h3>最终解析来源</h3><div className="preview-grid">{preview.map((item) => <div key={item.key}><strong>{item.key}</strong><span>{sourceLabel(item.source)}</span><small>{item.target}{item.locked ? " · locked" : ""}</small></div>)}</div></div>}
    </section>
  );
}

function kindGlyph(kind: InjectionKind) {
  return kind === "environment_variable" ? "ENV" : kind === "ssh_public_key" ? "SSH" : kind === "secret_file" ? "SEC" : "CFG";
}

function sourceLabel(scope: InjectionScope) {
  return scope === "organization" ? "来自组织" : scope === "user" ? "来自用户" : "来自工作区";
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "操作失败";
}
