import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { FluentProvider } from "@fluentui/react-components";
import { ApiClient } from "./api";
import { AppShell, EmptyOrganization, LoadingView, LoginScreen, type AppNotice, type AppView } from "./design-system/AppShell";
import { darkTheme, lightTheme } from "./design-system/theme";
import { WorkspacePanel } from "./WorkspacePanel";
import { useI18n } from "./i18n";
import { canManageOrganization as mayManageOrganization, canManageSystem } from "./permissions";
import type { Organization, Principal, WorkspaceResponse } from "./types";

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
  const [view, setView] = useState<AppView>(() => viewFromHash(window.location.hash));
  const [loading, setLoading] = useState(Boolean(token));
  const [notice, setNoticeState] = useState<AppNotice | null>(null);
  const noticeSequence = useRef(0);
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

  const reportError = useCallback((errorMessage: string) => {
    const value = errorMessage.trim();
    if (!value) return;
    setNoticeState({ id: ++noticeSequence.current, message: value, intent: "error" });
  }, []);

  const navigate = useCallback((next: AppView) => {
    if (next === viewFromHash(window.location.hash)) {
      setView(next);
      return;
    }
    window.history.pushState(null, "", `#${next}`);
    setView(next);
  }, []);

  useEffect(() => {
    const restoreView = () => setView(viewFromHash(window.location.hash));
    window.addEventListener("popstate", restoreView);
    window.addEventListener("hashchange", restoreView);
    return () => {
      window.removeEventListener("popstate", restoreView);
      window.removeEventListener("hashchange", restoreView);
    };
  }, []);

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
      if (requestGeneration === workspaceRequestGeneration.current) reportError(message(error, t("requestFailed")));
    } finally {
      if (requestGeneration === workspaceRequestGeneration.current) setLoading(false);
    }
  }, [api, organizationId, reportError, t]);

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
        setFatal(isAuthenticationError(error) ? t("loginInvalidToken") : message(error, t("requestFailed")));
        setPrincipal(null);
      })
      .finally(() => active && setLoading(false));
    return () => { active = false; };
  }, [api, token]);

  useEffect(() => {
    // WorkspacePanel owns the workspace page's paginated list and runtime
    // polling. The small preview exists only to seed the credential picker.
    if (view !== "injections") return;
    void refresh();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 10000);
    return () => window.clearInterval(timer);
  }, [refresh, view]);

  useEffect(() => {
    if (!principal) return;
    const allowed = view === "administration" ? canOpenAdministration : canManageGlobalState || canManageOrganizationState;
    if ((view === "administration" || view === "audit" || view === "plugins") && !allowed) {
      navigate("workspaces");
    }
  }, [canManageGlobalState, canManageOrganizationState, canOpenAdministration, navigate, principal, view]);

  function login(event: FormEvent) {
    event.preventDefault();
    const value = tokenDraft.trim();
    ApiClient.rememberToken(value);
    setToken(value);
    setFatal("");
    setNoticeState(null);
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
    setNoticeState(null);
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
      <FluentProvider theme={theme === "dark" ? darkTheme : lightTheme} style={{ minHeight: "100vh" }}>
        <LoginScreen locale={locale} setLocale={setLocale} themeMode={theme} onToggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")} tokenDraft={tokenDraft} setTokenDraft={setTokenDraft} onSubmit={login} loading={loading} fatal={fatal} t={t} />
      </FluentProvider>
    );
  }

  return (
    <FluentProvider theme={theme === "dark" ? darkTheme : lightTheme} style={{ minHeight: "100vh" }}>
      <AppShell view={view} onViewChange={navigate} locale={locale} setLocale={setLocale} themeMode={theme} onToggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")} principal={principal} currentOrganization={currentOrganization} organizationRole={organizationRole} canOpenAdministration={canOpenAdministration} canManageGlobalState={canManageGlobalState} canManageOrganizationState={canManageOrganizationState} onLogout={logout} notice={notice} t={t}>
        <Suspense fallback={<LoadingView label={t("loading")} />}>
          {view === "settings" ? (
            <SettingsPanel api={api} principal={principal} organizations={organizations} organizationId={organizationId} onOrganizationChange={selectOrganization} onProfileChanged={(profile) => setPrincipal((current) => current ? { ...current, ...profile } : current)} onError={reportError} />
          ) : view === "audit" ? (
            <AuditPanel api={api} organizationId={organizationId} systemAdmin={canManageGlobalState} onError={reportError} />
          ) : !organizationId ? <EmptyOrganization systemAdmin={canManageGlobalState} t={t} /> : view === "workspaces" ? (
            <WorkspacePanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} busy={loading} onRefresh={refresh} onError={reportError} />
          ) : view === "injections" ? (
            <InjectionPanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} onError={reportError} />
          ) : view === "plugins" ? (
            <PluginPanel token={token} organizationId={organizationId} systemAdmin={canManageGlobalState} onOpenCredentials={() => navigate("injections")} />
          ) : (
            <AdminPanel api={api} principal={principal} organizationId={organizationId} onError={reportError} onOrganizationsChanged={refreshOrganizations} />
          )}
        </Suspense>
      </AppShell>
    </FluentProvider>
  );
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

function isAuthenticationError(error: unknown): boolean {
  return error instanceof Error && "status" in error && (error.status === 401 || error.status === 403);
}

function viewFromHash(hash: string): AppView {
  const candidate = hash.replace(/^#/, "") as AppView;
  return ["workspaces", "injections", "plugins", "administration", "audit", "settings"].includes(candidate)
    ? candidate
    : "workspaces";
}
