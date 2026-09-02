import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { ApiClient } from "./api";
import { BrandIcon } from "./BrandIcon";
import { UserAvatar } from "./UserAvatar";
import { WorkspacePanel } from "./WorkspacePanel";
import { useI18n } from "./i18n";
import { canManageOrganization as mayManageOrganization, canManageSystem } from "./permissions";
import type { Organization, Principal, WorkspaceResponse } from "./types";

type View = "workspaces" | "injections" | "plugins" | "administration" | "audit" | "settings";

// The workspace list is owned by WorkspacePanel. App only keeps a small first
// page as a preview for views that need a selected workspace or a lightweight
// status hint; it must never turn into an unbounded global workspace load.
const GLOBAL_WORKSPACE_PREVIEW_LIMIT = 30;
const GLOBAL_ORGANIZATION_PREVIEW_LIMIT = 50;

const AdminPanel = lazy(() => import("./OperationsPanel").then(({ AdminPanel: component }) => ({ default: component })));
const AuditPanel = lazy(() => import("./AuditPanel").then(({ AuditPanel: component }) => ({ default: component })));
const InjectionPanel = lazy(() => import("./InjectionPanel").then(({ InjectionPanel: component }) => ({ default: component })));
const PluginPanel = lazy(() => import("./PluginPanel").then(({ PluginPanel: component }) => ({ default: component })));
const SettingsPanel = lazy(() => import("./SettingsPanel").then(({ SettingsPanel: component }) => ({ default: component })));

export default function App() {
  const { locale, setLocale, t } = useI18n();
  const [theme, setTheme] = useState<"light" | "dark">(() => {
    const saved = localStorage.getItem("mwc.theme");
    if (saved === "light" || saved === "dark") return saved;
    return matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  });
  const [token, setToken] = useState(ApiClient.savedToken());
  const [tokenDraft, setTokenDraft] = useState(token);
  const [principal, setPrincipal] = useState<Principal | null>(null);
  const [organizationId, setOrganizationId] = useState("");
  const [organizations, setOrganizations] = useState<Organization[]>([]);
  const [workspaces, setWorkspaces] = useState<WorkspaceResponse[]>([]);
  const [workspaceScope, setWorkspaceScope] = useState<{ api: ApiClient; organizationId: string } | null>(null);
  const workspaceRequestGeneration = useRef(0);
  const [view, setView] = useState<View>("workspaces");
  const [loading, setLoading] = useState(Boolean(token));
  const [notice, setNotice] = useState("");
  const [fatal, setFatal] = useState("");
  const api = useMemo(() => new ApiClient(token), [token]);
  const organizationRole = principal?.memberships.find((membership) => membership.organization_id === organizationId)?.role;
  const canManageGlobalState = Boolean(principal && canManageSystem(principal));
  const canManageOrganizationState = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_organization"));
  const canManageMembers = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_members"));
  const canOpenAdministration = canManageGlobalState || canManageOrganizationState || canManageMembers;
  const currentOrganization = organizations.find((organization) => organization.id === organizationId);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem("mwc.theme", theme);
  }, [theme]);

  // Invalidate an in-flight preview before fetching for the new scope. The
  // scope check below also prevents one render of an old organization from
  // leaking into the newly selected organization.
  useEffect(() => {
    workspaceRequestGeneration.current += 1;
    setWorkspaceScope(null);
    setWorkspaces([]);
  }, [api, organizationId]);

  const refresh = useCallback(async () => {
    const requestedOrganizationId = organizationId;
    const requestGeneration = ++workspaceRequestGeneration.current;
    if (!requestedOrganizationId) {
      setWorkspaceScope(null);
      setWorkspaces([]);
      return;
    }
    setLoading(true);
    try {
      const page = await api.workspacesPage(requestedOrganizationId, { limit: GLOBAL_WORKSPACE_PREVIEW_LIMIT });
      if (requestGeneration !== workspaceRequestGeneration.current) return;
      setWorkspaces(page.items);
      setWorkspaceScope({ api, organizationId: requestedOrganizationId });
    } catch (error) {
      if (requestGeneration === workspaceRequestGeneration.current) setNotice(message(error));
    } finally {
      if (requestGeneration === workspaceRequestGeneration.current) setLoading(false);
    }
  }, [api, organizationId]);

  const refreshOrganizations = useCallback(async (preferredOrganizationId?: string) => {
    const [nextPrincipal, organizationPage] = await Promise.all([
      api.me(),
      api.organizationsPage({ limit: GLOBAL_ORGANIZATION_PREVIEW_LIMIT }),
    ]);
    const visibleOrganizations = organizationPage.items;
    setPrincipal(nextPrincipal);
    setOrganizations(visibleOrganizations);
    setOrganizationId((current) => {
      const next = preferredOrganizationId && visibleOrganizations.some((organization) => organization.id === preferredOrganizationId)
        ? preferredOrganizationId
        : visibleOrganizations.some((organization) => organization.id === current)
          ? current
          : visibleOrganizations[0]?.id ?? "";
      if (next) localStorage.setItem("mwc.organization-id", next);
      else localStorage.removeItem("mwc.organization-id");
      return next;
    });
  }, [api]);

  useEffect(() => {
    if (!token) return;
    let active = true;
    setLoading(true);
    Promise.all([api.me(), api.organizationsPage({ limit: GLOBAL_ORGANIZATION_PREVIEW_LIMIT })])
      .then(([value, organizationPage]) => {
        if (!active) return;
        const visibleOrganizations = organizationPage.items;
        setPrincipal(value);
        setOrganizations(visibleOrganizations);
        setOrganizationId((current) => {
          const saved = localStorage.getItem("mwc.organization-id") ?? "";
          if (visibleOrganizations.some((organization) => organization.id === current)) return current;
          if (visibleOrganizations.some((organization) => organization.id === saved)) return saved;
          return visibleOrganizations[0]?.id ?? "";
        });
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
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 10000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 5000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    const allowed = view === "administration" ? canOpenAdministration : canManageGlobalState || canManageOrganizationState;
    if ((view === "administration" || view === "audit" || view === "plugins") && !allowed) {
      setView("workspaces");
    }
  }, [canManageGlobalState, canManageOrganizationState, canOpenAdministration, view]);

  function login(event: FormEvent) {
    event.preventDefault();
    const value = tokenDraft.trim();
    ApiClient.rememberToken(value);
    setToken(value);
    setFatal("");
  }

  function logout() {
    ApiClient.forgetToken();
    workspaceRequestGeneration.current += 1;
    setToken("");
    setTokenDraft("");
    setPrincipal(null);
    setOrganizations([]);
    setOrganizationId("");
    setWorkspaceScope(null);
    setWorkspaces([]);
  }

  const scopedWorkspaces = workspaceScope?.api === api && workspaceScope.organizationId === organizationId
    ? workspaces
    : [];

  function selectOrganization(next: string) {
    // Settings may expose a placeholder option; never transition into an
    // organization-less state from an invalid selection.
    if (!organizations.some((organization) => organization.id === next)) return;
    workspaceRequestGeneration.current += 1;
    setWorkspaceScope(null);
    setWorkspaces([]);
    setOrganizationId(next);
    localStorage.setItem("mwc.organization-id", next);
  }

  if (!token || !principal) {
    return (
      <main className="login-shell">
        <div className="ambient one" aria-hidden="true" /><div className="ambient two" aria-hidden="true" />
        <section className="login-card">
          <BrandIcon className="large" size={54} />
          <p className="eyebrow">MEMELOOP CONTROL PLANE</p>
          <div className="display-controls"><LanguagePicker locale={locale} setLocale={setLocale} /><button type="button" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>{theme === "dark" ? t("themeLight") : t("themeDark")}</button></div>
          <h1>{t("loginTitle")}</h1>
          <p className="login-copy">{t("loginCopy")}</p>
          <form onSubmit={login}>
            <label>{t("token")}<input autoFocus type="password" minLength={32} required value={tokenDraft} onChange={(event) => setTokenDraft(event.target.value)} placeholder={t("tokenPlaceholder")} /></label>
            <button className="button primary full" disabled={loading}>{loading ? t("signingIn") : t("signIn")}</button>
          </form>
          {fatal && <div className="error-banner">{fatal}</div>}
          <div className="trust-row"><span>{t("envelopeEncryption")}</span><span>RBAC</span><span>{t("auditTrail")}</span></div>
        </section>
      </main>
    );
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><BrandIcon /><div><strong>Memeloop</strong><small>Workspace Control</small></div></div>
        <nav>
          <Nav active={view === "workspaces"} onClick={() => setView("workspaces")} icon="◇">{t("workspaces")}</Nav>
          <Nav active={view === "injections"} onClick={() => setView("injections")} icon="⌁">{t("credentials")}</Nav>
          {(canManageGlobalState || canManageOrganizationState) && <Nav active={view === "plugins"} onClick={() => setView("plugins")} icon="⬡">{t("pluginsTitle")}</Nav>}
          {canOpenAdministration && <Nav active={view === "administration"} onClick={() => setView("administration")} icon="◉">{t("administration")}</Nav>}
          {(canManageGlobalState || canManageOrganizationState) && <Nav active={view === "audit"} onClick={() => setView("audit")} icon="≡">{t("audit")}</Nav>}
          <Nav active={view === "settings"} onClick={() => setView("settings")} icon="⚙">{t("settings")}</Nav>
        </nav>
        <div className="sidebar-foot"><span className="live-dot" />{t("apiOnline")}</div>
      </aside>

      <main className="content">
        <header className="topbar">
          <div className="current-organization"><span>{t("currentOrganization")}</span><strong>{currentOrganization?.name ?? t("notEnabled")}</strong></div>
          <div className="topbar-actions"><LanguagePicker locale={locale} setLocale={setLocale} className="utility-select" /><button className="utility-button" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>{theme === "dark" ? t("themeLight") : t("themeDark")}</button><div className="user-menu"><button className="user-menu-trigger" onClick={() => setView("settings")}><UserAvatar displayName={principal.display_name} userId={principal.user_id} avatarUrl={principal.avatar_url} /><span><strong>{principal.display_name}</strong><small>{principal.system_admin ? t("systemAdmin") : organizationRole === "organization_admin" ? t("organizationAdmin") : t("organizationMember")}</small></span></button><button className="logout-button" onClick={logout}>{t("logout")}</button></div></div>
        </header>

        <Suspense fallback={<LoadingPanel label={t("loading")} />}>
          {view === "settings" ? (
            <SettingsPanel api={api} principal={principal} organizations={organizations} organizationId={organizationId} onOrganizationChange={selectOrganization} onProfileChanged={(profile) => setPrincipal((current) => current ? { ...current, ...profile } : current)} onError={setNotice} />
          ) : view === "audit" ? (
            <AuditPanel api={api} organizationId={organizationId} systemAdmin={canManageGlobalState} onError={setNotice} />
          ) : !organizationId ? <EmptyOrganization systemAdmin={canManageGlobalState} /> : view === "workspaces" ? (
            <WorkspacePanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} busy={loading} onRefresh={refresh} onError={setNotice} />
          ) : view === "injections" ? (
            <InjectionPanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} onError={setNotice} />
          ) : view === "plugins" ? (
            <PluginPanel token={token} organizationId={organizationId} systemAdmin={canManageGlobalState} onOpenCredentials={() => setView("injections")} />
          ) : (
            <AdminPanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} onError={setNotice} onOrganizationsChanged={refreshOrganizations} />
          )}
        </Suspense>
      </main>
      {notice && <div className="toast" role="status" aria-live="polite" aria-atomic="true">{notice}</div>}
    </div>
  );
}

function LoadingPanel({ label }: { label: string }) {
  return <div className="loading-panel" role="status" aria-label={label}>
    <div className="loading-skeleton loading-skeleton-heading" />
    <div className="loading-skeleton-grid"><div className="loading-skeleton" /><div className="loading-skeleton" /><div className="loading-skeleton" /></div>
    <span className="loading-label">{label}</span>
  </div>;
}

function LanguagePicker({ locale, setLocale, className }: { locale: "zh-CN" | "en" | "ru"; setLocale: (locale: "zh-CN" | "en" | "ru") => void; className?: string }) {
  const { t } = useI18n();
  return <label className={`language-picker ${className ?? ""}`}><span>{t("language")}</span><select value={locale} onChange={(event) => setLocale(event.target.value as "zh-CN" | "en" | "ru")}><option value="zh-CN">{t("languageChinese")}</option><option value="en">{t("languageEnglish")}</option><option value="ru">{t("languageRussian")}</option></select></label>;
}

function Nav({ active, onClick, icon, children }: { active: boolean; onClick: () => void; icon: string; children: string }) {
  return <button className={active ? "active" : ""} aria-current={active ? "page" : undefined} onClick={onClick}><span aria-hidden="true">{icon}</span>{children}</button>;
}

function EmptyOrganization({ systemAdmin }: { systemAdmin: boolean }) {
  const { t } = useI18n();
  return <div className="empty-page"><BrandIcon className="large" size={54} /><h2>{t("noOrganization")}</h2><p>{systemAdmin ? t("noOrganizationAdmin") : t("noOrganizationMember")}</p><a className="button" href="/api/v1/openapi.json" target="_blank" rel="noreferrer">{t("viewOpenApi")}</a></div>;
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "请求失败";
}
