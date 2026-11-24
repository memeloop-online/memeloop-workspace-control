import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { FluentProvider } from "@fluentui/react-components";
import { ApiClient } from "./api";
import { BROWSER_SESSION, clearOidcResult, confirmOidc, logoutOidc, oidcResult, readOidcStatus, startOidc, type OidcAction, type OidcStatus } from "./browserAuth.ts";
import { AppShell, EmptyOrganization, LoadingView, LoginScreen, type AppNotice, type AppView } from "./design-system/AppShell";
import { darkTheme, lightTheme } from "./design-system/theme";
import { WorkspacePanel } from "./WorkspacePanel";
import { useI18n, type MessageKey } from "./i18n";
import { canManageOrganization as mayManageOrganization, canManageSystem, hasApiKeyScope } from "./permissions";
import { principalQueryKey, useOrganizationsQuery, usePrincipalQuery, useWorkspacePreviewQuery } from "./state/appQueries";
import { queryClient, useAppStore } from "./state";
import type { Principal } from "./types";

const ERROR_NOTICE_DEDUPE_MS = 10_000;
const NO_OIDC: OidcStatus = { enabled: false, signed_in: false };

const viewTitles: Record<AppView, MessageKey> = {
  workspaces: "workspaces",
  injections: "credentials",
  plugins: "pluginsTitle",
  administration: "administration",
  audit: "audit",
  settings: "settings",
};

const AuditPanel = lazy(() => import("./AuditPanel").then(({ AuditPanel: component }) => ({ default: component })));
const InjectionPanel = lazy(() => import("./InjectionPanel").then(({ InjectionPanel: component }) => ({ default: component })));
const PluginPanel = lazy(() => import("./PluginPanel").then(({ PluginPanel: component }) => ({ default: component })));
const SettingsPanel = lazy(() => import("./SettingsPanel").then(({ SettingsPanel: component }) => ({ default: component })));

export default function App() {
  const { locale, setLocale, t } = useI18n();
  const storedToken = useAppStore((state) => state.token);
  const [oidc, setOidc] = useState<OidcStatus | null>(null);
  const [callback, setCallback] = useState(() => oidcResult(window.location.search));
  const [proofKey, setProofKey] = useState("");
  const [oidcBusy, setOidcBusy] = useState(false);
  const [oidcFeedback, setOidcFeedback] = useState<{ key: MessageKey; error?: boolean } | null>(null);
  const pendingProof = callback === "bind" || callback === "unbind" ? callback : null;
  const proofAction = oidc?.enabled ? pendingProof : null;
  const browserSession = !storedToken && Boolean(oidc?.signed_in);
  const token = storedToken || (browserSession ? BROWSER_SESSION : "");
  const organizationId = useAppStore((state) => state.organizationId);
  const theme = useAppStore((state) => state.theme);
  const view = useAppStore((state) => state.view);
  const setToken = useAppStore((state) => state.setToken);
  const setOrganizationId = useAppStore((state) => state.setOrganizationId);
  const setTheme = useAppStore((state) => state.setTheme);
  const setView = useAppStore((state) => state.setView);
  const [tokenDraft, setTokenDraft] = useState(storedToken);
  const [notice, setNoticeState] = useState<AppNotice | null>(null);
  const noticeSequence = useRef(0);
  const lastErrorRef = useRef<{ message: string; at: number }>({ message: "", at: 0 });
  const activeTokenRef = useRef(token);
  const unauthorizedTokenRef = useRef<string | null>(null);
  const [fatal, setFatal] = useState("");
  activeTokenRef.current = token;

  useEffect(() => {
    const abort = new AbortController();
    void readOidcStatus(abort.signal).then((status) => {
      if (!abort.signal.aborted) setOidc(status);
    }).catch(() => {
      if (!abort.signal.aborted) setOidc(NO_OIDC);
    });
    return () => abort.abort();
  }, []);

  const clearSession = useCallback(() => {
    activeTokenRef.current = "";
    queryClient.clear();
    setToken("");
    setOrganizationId("");
    setView("workspaces");
    setTokenDraft("");
    setNoticeState(null);
    setOidc((current) => current && ({ ...current, signed_in: false }));
  }, [setOrganizationId, setToken, setView]);

  async function logout() {
    if (oidcBusy) return;
    if (browserSession) {
      setOidcBusy(true);
      try {
        if (!(await logoutOidc()).ok) throw new Error("oidc_logout_failed");
      } catch {
        reportError(t("ssoLogoutFailed"));
        return;
      } finally {
        setOidcBusy(false);
      }
    }
    setOidcFeedback(null);
    clearSession();
  }

  const api = useMemo(() => new ApiClient(token, () => {
    if (!token || activeTokenRef.current !== token || unauthorizedTokenRef.current === token) return;
    unauthorizedTokenRef.current = token;
    if (browserSession) setOidcFeedback({ key: "ssoSessionExpired", error: true });
    else setFatal(t("loginInvalidToken"));
    clearSession();
  }), [browserSession, clearSession, t, token]);

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

  const organizationRole = principal?.memberships.find((membership) => membership.organization_id === organizationId)?.role;
  const canManageGlobalState = Boolean(principal && canManageSystem(principal));
  const canManageOrganizationState = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_organization"));
  const canManageMembers = Boolean(principal && organizationId && mayManageOrganization(principal, organizationId, "manage_members"));
  const canManageOwnApiKeys = Boolean(principal && hasApiKeyScope(principal, "manage_api_keys"));
  const canOpenAdministration = canManageGlobalState || canManageOrganizationState || canManageMembers || canManageOwnApiKeys;
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

  useEffect(() => {
    if (!principal || (callback !== "failed" && callback !== "unbound")) return;
    reportError(t(callback === "unbound" ? "ssoAccountNotLinked" : "ssoFailed"));
    clearOidcResult();
    setCallback(null);
  }, [callback, principal, reportError, t]);

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
    setFatal(isAuthenticationError(authError) ? t(browserSession ? "ssoSessionExpired" : "loginInvalidToken") : message(authError, t("requestFailed")));
  }, [authError, browserSession, principal, t, token]);

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
    const allowed = canManageGlobalState || canManageOrganizationState;
    if ((view === "audit" || view === "plugins") && !allowed) {
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
    if (callback) dismissProof();
    setFatal("");
    setNoticeState(null);
  }

  function dismissProof() {
    clearOidcResult();
    setCallback(null);
    setProofKey("");
    setOidcFeedback(null);
  }

  async function chooseOidc(action: OidcAction) {
    if (oidcBusy) return;
    setOidcFeedback(null);
    setOidcBusy(true);
    try {
      await startOidc(action);
    } catch {
      if (principal) reportError(t("ssoFailed"));
      else setOidcFeedback({ key: "ssoFailed", error: true });
      setOidcBusy(false);
    }
  }

  async function submitProof(event: FormEvent) {
    event.preventDefault();
    if (!proofAction || oidcBusy) return;
    const apiKey = proofKey.trim();
    setProofKey("");
    if (apiKey.length < 32) return;
    setOidcFeedback(null);
    setOidcBusy(true);
    try {
      const response = await confirmOidc(proofAction, apiKey);
      if (response.status !== 204) {
        const key = response.status === 401 || response.status === 403 ? "ssoProofRejected" : response.status === 409 ? "ssoBindingConflict" : "ssoFailed";
        setOidcFeedback({ key, error: true });
        return;
      }
      dismissProof();
      let remainsSignedIn = Boolean(storedToken);
      if (proofAction === "unbind") {
        const status = await readOidcStatus().catch(() => ({ enabled: true, signed_in: false }));
        if (browserSession && !status.signed_in) clearSession();
        setOidc(status);
        remainsSignedIn ||= status.signed_in;
      }
      const key = proofAction === "bind" ? "ssoLinked" : "ssoUnlinked";
      if (remainsSignedIn) setNoticeState({ id: ++noticeSequence.current, message: t(key), intent: "success" });
      setOidcFeedback({ key });
    } catch {
      setOidcFeedback({ key: "ssoFailed", error: true });
    } finally {
      setOidcBusy(false);
    }
  }

  const scopedWorkspaces = workspacePreview?.items ?? [];

  function selectOrganization(next: string) {
    // Settings may expose a placeholder option; never transition into an
    // organization-less state from an invalid selection.
    if (!organizations.some((organization) => organization.id === next)) return;
    setOrganizationId(next);
  }

  if (pendingProof && oidc === null) {
    return <FluentProvider theme={theme === "dark" ? darkTheme : lightTheme}><LoadingView label={t("loading")} /></FluentProvider>;
  }

  if (proofAction || !token || !principal || authError) {
    const callbackError = callback === "unbound" ? "ssoAccountNotLinked" : callback === "failed" || (pendingProof && !oidc?.enabled) ? "ssoFailed" : null;
    const errorKey = oidcFeedback ? (oidcFeedback.error ? oidcFeedback.key : null) : callbackError;
    return (
      <FluentProvider theme={theme === "dark" ? darkTheme : lightTheme} style={{ minHeight: "100vh" }}>
        <LoginScreen
          locale={locale} setLocale={setLocale} themeMode={theme} onToggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")}
          tokenDraft={proofAction ? proofKey : tokenDraft} setTokenDraft={proofAction ? setProofKey : setTokenDraft}
          onSubmit={proofAction ? submitProof : login} loading={proofAction ? oidcBusy : loading}
          fatal={errorKey ? t(errorKey) : fatal} success={oidcFeedback && !oidcFeedback.error ? t(oidcFeedback.key) : undefined}
          sso={oidc?.enabled ? { busy: oidcBusy, proofAction, onStart: chooseOidc, onCancel: dismissProof } : undefined} t={t}
        />
      </FluentProvider>
    );
  }

  return (
    <FluentProvider theme={theme === "dark" ? darkTheme : lightTheme} style={{ minHeight: "100vh" }}>
      <AppShell view={view} onViewChange={navigate} locale={locale} setLocale={setLocale} themeMode={theme} onToggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")} principal={principal} currentOrganization={currentOrganization} organizationRole={organizationRole} canOpenAdministration={canOpenAdministration} canManageGlobalState={canManageGlobalState} canManageOrganizationState={canManageOrganizationState} onLogout={logout} onUnlink={oidc?.enabled && !oidcBusy ? () => void chooseOidc("unbind") : undefined} notice={notice} t={t}>
        <Suspense fallback={<LoadingView label={t("loading")} />}>
          {view === "settings" || view === "administration" ? (
            <SettingsPanel api={api} principal={principal} organizations={organizations} organizationId={organizationId} onOrganizationChange={selectOrganization} onOrganizationsChanged={refreshOrganizations} onProfileChanged={(profile) => queryClient.setQueryData<Principal>(principalQueryKey(), (current) => current ? { ...current, ...profile } : current)} onError={reportError} theme={theme} onThemeChange={setTheme} />
          ) : view === "audit" ? (
            <AuditPanel api={api} organizationId={organizationId} systemAdmin={canManageGlobalState} onError={reportError} />
          ) : !organizationId ? <EmptyOrganization systemAdmin={canManageGlobalState} t={t} /> : view === "workspaces" ? (
            <WorkspacePanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} busy={loading} onRefresh={refresh} onError={reportError} />
          ) : view === "injections" ? (
            <InjectionPanel api={api} principal={principal} organizationId={organizationId} workspaces={scopedWorkspaces} onError={reportError} />
          ) : view === "plugins" ? (
            <PluginPanel token={token} organizationId={organizationId} systemAdmin={canManageGlobalState} onOpenCredentials={() => navigate("injections")} />
          ) : null}
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
  if (candidate === "administration") return "settings";
  return ["workspaces", "injections", "plugins", "administration", "audit", "settings"].includes(candidate)
    ? candidate
    : "workspaces";
}
