import { useCallback, useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { ApiClient } from "./api";
import { InjectionPanel } from "./InjectionPanel";
import { OperationsPanel } from "./OperationsPanel";
import { WorkspacePanel } from "./WorkspacePanel";
import type { Organization, Principal, WorkspaceResponse } from "./types";

type View = "workspaces" | "injections" | "system";

export default function App() {
  const [token, setToken] = useState(ApiClient.savedToken());
  const [tokenDraft, setTokenDraft] = useState(token);
  const [principal, setPrincipal] = useState<Principal | null>(null);
  const [organizationId, setOrganizationId] = useState("");
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [workspaces, setWorkspaces] = useState<WorkspaceResponse[]>([]);
  const [view, setView] = useState<View>("workspaces");
  const [loading, setLoading] = useState(Boolean(token));
  const [notice, setNotice] = useState("");
  const [fatal, setFatal] = useState("");
  const api = useMemo(() => new ApiClient(token), [token]);

  const refresh = useCallback(async () => {
    if (!organizationId) return;
    setLoading(true);
    try {
      setWorkspaces(await api.workspaces(organizationId));
    } catch (error) {
      setNotice(message(error));
    } finally {
      setLoading(false);
    }
  }, [api, organizationId]);

  useEffect(() => {
    if (!token) return;
    let active = true;
    setLoading(true);
    Promise.all([api.me(), api.organizations()])
      .then(([value, visibleOrganizations]) => {
        if (!active) return;
        setPrincipal(value);
        setOrganizations(visibleOrganizations);
        setOrganizationId((current) => current || visibleOrganizations[0]?.id || "");
        setFatal("");
      })
      .catch((error) => {
        if (!active) return;
        setFatal(message(error));
        setPrincipal(null);
      })
      .finally(() => active && setLoading(false));
    return () => { active = false; };
  }, [api, token]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 5000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  function login(event: FormEvent) {
    event.preventDefault();
    const value = tokenDraft.trim();
    ApiClient.rememberToken(value);
    setToken(value);
    setFatal("");
  }

  function logout() {
    ApiClient.forgetToken();
    setToken("");
    setTokenDraft("");
    setPrincipal(null);
    setWorkspaces([]);
  }

  if (!token || !principal) {
    return (
      <main className="login-shell">
        <div className="ambient one" /><div className="ambient two" />
        <section className="login-card">
          <div className="brand-mark large"><span>M</span></div>
          <p className="eyebrow">MEMELOOP CONTROL PLANE</p>
          <h1>工作区，从这里开始。</h1>
          <p className="login-copy">管理 Kubernetes 工作区、SSH、Web Shell 与级联配置。API Token 只保存在当前浏览器会话。</p>
          <form onSubmit={login}>
            <label>API Token<input autoFocus type="password" minLength={32} required value={tokenDraft} onChange={(event) => setTokenDraft(event.target.value)} placeholder="至少 32 字节" /></label>
            <button className="button primary full" disabled={loading}>{loading ? "验证中…" : "进入控制台"}</button>
          </form>
          {fatal && <div className="error-banner">{fatal}</div>}
          <div className="trust-row"><span>信封加密</span><span>RBAC</span><span>审计留痕</span></div>
        </section>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><div className="brand-mark"><span>M</span></div><div><strong>Memeloop</strong><small>Workspace Control</small></div></div>
        <nav>
          <Nav active={view === "workspaces"} onClick={() => setView("workspaces")} icon="◇">工作区</Nav>
          <Nav active={view === "injections"} onClick={() => setView("injections")} icon="⌁">Secret 与文件</Nav>
          <Nav active={view === "system"} onClick={() => setView("system")} icon="◉">系统与审计</Nav>
        </nav>
        <div className="sidebar-foot"><span className="live-dot" />API online</div>
      </aside>

      <main className="content">
        <header className="topbar">
          <div>
            <label className="org-picker">组织<select value={organizationId} onChange={(event) => setOrganizationId(event.target.value)}>{organizations.map((organization) => <option key={organization.id} value={organization.id}>{organization.name}</option>)}</select></label>
          </div>
          <div className="user-menu"><div className="avatar">{principal.display_name.slice(0, 1).toUpperCase()}</div><div><strong>{principal.display_name}</strong><small>{principal.system_admin ? "系统管理员" : "组织成员"}</small></div><button onClick={logout}>退出</button></div>
        </header>

        {!organizationId ? <EmptyOrganization systemAdmin={principal.system_admin} /> : view === "workspaces" ? (
          <WorkspacePanel api={api} principal={principal} organizationId={organizationId} workspaces={workspaces} busy={loading} onRefresh={refresh} onError={setNotice} />
        ) : view === "injections" ? (
          <InjectionPanel api={api} principal={principal} organizationId={organizationId} workspaces={workspaces} onError={setNotice} />
        ) : (
          <OperationsPanel api={api} principal={principal} organizationId={organizationId} workspaces={workspaces} onError={setNotice} />
        )}
      </main>
      {notice && <div className="toast">{notice}</div>}
    </div>
  );
}

function Nav({ active, onClick, icon, children }: { active: boolean; onClick: () => void; icon: string; children: string }) {
  return <button className={active ? "active" : ""} onClick={onClick}><span>{icon}</span>{children}</button>;
}

function EmptyOrganization({ systemAdmin }: { systemAdmin: boolean }) {
  return <div className="empty-page"><div className="brand-mark large"><span>+</span></div><h2>尚未加入组织</h2><p>{systemAdmin ? "请先通过组织管理 API 创建组织，再刷新控制台。" : "请联系组织管理员添加你的成员关系。"}</p><a className="button" href="/api/v1/openapi.json" target="_blank">查看 OpenAPI</a></div>;
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "请求失败";
}
