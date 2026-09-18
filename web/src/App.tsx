import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { FluentProvider } from "@fluentui/react-components";
import { ApiClient } from "./api";
import { AppShell, EmptyOrganization, LoadingView, LoginScreen, type AppNotice, type AppView } from "./design-system/AppShell";
import { darkTheme, lightTheme } from "./design-system/theme";
import { WorkspacePanel } from "./WorkspacePanel";
import { useI18n, type MessageKey } from "./i18n";
import { canManageOrganization as mayManageOrganization, canManageSystem } from "./permissions";
import { principalQueryKey, useOrganizationsQuery, usePrincipalQuery, useWorkspacePreviewQuery } from "./state/appQueries";
import { queryClient, useAppStore } from "./state";
import type { Principal } from "./types";

const ERROR_NOTICE_DEDUPE_MS = 10_000;

const viewTitles: Record<AppView, MessageKey> = {
  workspaces: "workspaces",
  injections: "credentials",
  plugins: "pluginsTitle",
  administration: "administration",
  audit: "audit",
  settings: "settings",
};

const AdminPanel = lazy(() => import("./OperationsPanel").then(({ AdminPanel: component }) => ({ default: component })));
const AuditPanel = lazy(() => import("./AuditPanel").then(({ AuditPanel: component }) => ({ default: component })));
const InjectionPanel = lazy(() => import("./InjectionPanel").then(({ InjectionPanel: component }) => ({ default: component })));
const PluginPanel = lazy(() => import("./PluginPanel").then(({ PluginPanel: component }) => ({ default: component })));
const SettingsPanel = lazy(() => import("./SettingsPanel").then(({ SettingsPanel: component }) => ({ default: component })));

export default function App() {
  const { locale, setLocale, t } = useI18n();
  const token = useAppStore((state) => state.token);
  const organizationId = useAppStore((state) => state.organizationId);
  const theme = useAppStore((state) => state.theme);
  const view = useAppStore((state) => state.view);
  const setToken = useAppStore((state) => state.setToken);
  const setOrganizationId = useAppStore((state) => state.setOrganizationId);
  const setTheme = useAppStore((state) => state.setTheme);
  const setView = useAppStore((state) => state.setView);
  const [tokenDraft, setTokenDraft] = useState(token);
  const [notice, setNoticeState] = useState<AppNotice | null>(null);
  const noticeSequence = useRef(0);
  const lastErrorRef = useRef<{ message: string; at: number }>({ message: "", at: 0 });
  const activeTokenRef = useRef(token);
  const unauthorizedTokenRef = useRef<string | null>(null);
  const [fatal, setFatal] = useState("");
  activeTokenRef.current = token;

  const logout = useCallback(() => {
    activeTokenRef.current = "";
    queryClient.clear();
    setToken("");
    setOrganizationId("");
    setView("workspaces");
    setTokenDraft("");
    setNoticeState(null);
  }, [setOrganizationId, setToken, setView]);

  const api = useMemo(() => new ApiClient(token, () => {
    if (!token || activeTokenRef.current !== token || unauthorizedTokenRef.current === token) return;
    unauthorizedTokenRef.current = token;
    setFatal(t("loginInvalidToken"));
    logout();
  }), [logout, t, token]);

  const principalQuery = usePrincipalQuery(api, Boolean(token));
  const organizationsQuery = useOrganizationsQuery(api, Boolean(token));
  const principal = principalQuery.data ?? null;
  const organizations = organizationsQuery.data?.items ?? [];
  const workspacePreviewQuery = useWorkspacePreviewQuery(
    api,
    organizationId,
    Boolean(token && organizationId && view === "injections"),
  );
  const { data: workspacePreview, error: workspacePreviewError, refetch: refetchWorkspacePreview } = workspacePreviewQuery;
  const { error: principalError, refetch: refetchPrincipal } = principalQuery;
  const { error: organizationsError, refetch: refetchOrganizations } = organizationsQuery;
  const loading = Boolean(token && (principalQuery.isPending || organizationsQuery.isPending));
  const authError = principalError ?? organizationsError;
  const authenticated = Boolean(token && principal && !authError);

  const organizationRole = principal?.memberships.find((membership) => membership.organization_id === organizationId)?.role;
  const canManageGlobalState = Boolean(principal && canManageSystem(principal));
  const canManageOrganizationState = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_organization"));
  const canManageMembers = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_members"));
  const canOpenAdministration = canManageGlobalState || canManageOrganizationState || canManageMembers;
  const currentOrganization = organizations.find((organization) => organization.id === organizationId);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
  }, [theme]);

  useEffect(() => {
    document.title = token && principal ? `${t(viewTitles[view])} · ${t("appName")}` : t("appName");
  }, [principal, t, token, view]);

  useEffect(() => {
    if (window.location.hash) setView(viewFromHash(window.location.hash));
  }, [setView]);

  const reportError = useCallback((errorMessage: string) => {
    const value = errorMessage.trim();
    if (!value) return;
    const now = Date.now();
    if (value === lastErrorRef.current.message && now - lastErrorRef.current.at < ERROR_NOTICE_DEDUPE_MS) return;
    lastErrorRef.current = { message: value, at: now };
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
    const result = await refetchWorkspacePreview();
    if (result.error) reportError(message(result.error, t("requestFailed")));
  }, [refetchWorkspacePreview, reportError, t]);

  const refreshOrganizations = useCallback(async (preferredOrganizationId?: string) => {
    const [nextPrincipal, organizationPage] = await Promise.all([
      refetchPrincipal(),
      refetchOrganizations(),
    ]);
    if (nextPrincipal.error) throw nextPrincipal.error;
    if (organizationPage.error) throw organizationPage.error;
    const visibleOrganizations = organizationPage.data?.items ?? [];
    const next = preferredOrganizationId && visibleOrganizations.some((organization) => organization.id === preferredOrganizationId)
      ? preferredOrganizationId
      : visibleOrganizations.some((organization) => organization.id === organizationId)
        ? organizationId
        : visibleOrganizations[0]?.id ?? "";
    setOrganizationId(next);
  }, [organizationId, refetchOrganizations, refetchPrincipal, setOrganizationId]);

  useEffect(() => {
    if (!token || !authError) {
      if (token && principal) setFatal("");
      return;
    }
    setFatal(isAuthenticationError(authError) ? t("loginInvalidToken") : message(authError, t("requestFailed")));
  }, [authError, principal, t, token]);

  useEffect(() => {
    if (workspacePreviewError) reportError(message(workspacePreviewError, t("requestFailed")));
  }, [reportError, t, workspacePreviewError]);

  useEffect(() => {
    if (organizationsQuery.isPending || organizationsQuery.isError) return;
    if (organizations.length === 0) {
      if (organizationId) setOrganizationId("");
      return;
    }
    if (!organizations.some((organization) => organization.id === organizationId)) {
      setOrganizationId(organizations[0].id);
    }
  }, [organizationId, organizations, organizationsQuery.isError, organizationsQuery.isPending, setOrganizationId]);

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
    activeTokenRef.current = value;
    unauthorizedTokenRef.current = null;
    queryClient.clear();
    setToken(value);
    setFatal("");
    setNoticeState(null);
  }

  const scopedWorkspaces = workspacePreview?.items ?? [];

  function selectOrganization(next: string) {
    // Settings may expose a placeholder option; never transition into an
    // organization-less state from an invalid selection.
    if (!organizations.some((organization) => organization.id === next)) return;
    setOrganizationId(next);
  }

  if (!authenticated) {
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
            <SettingsPanel api={api} principal={principal} organizations={organizations} organizationId={organizationId} onOrganizationChange={selectOrganization} onProfileChanged={(profile) => queryClient.setQueryData<Principal>(principalQueryKey(), (current) => current ? { ...current, ...profile } : current)} onError={reportError} />
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
  return error instanceof Error && "status" in error && (error as Error & { status: number }).status === 401;
}

function viewFromHash(hash: string): AppView {
  const candidate = hash.replace(/^#/, "") as AppView;
  return ["workspaces", "injections", "plugins", "administration", "audit", "settings"].includes(candidate)
    ? candidate
    : "workspaces";
}
