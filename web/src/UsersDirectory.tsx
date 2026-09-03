import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  formatApiKeyExpiry,
  formatApiKeyPageStatus,
  formatApiKeyScopes,
  formatApiKeyStatus,
  formatTime,
} from "./adminApiKeyView";
import type { ApiClient } from "./api";
import { applyLocalRevocations, getApiKeyStatus } from "./apiKeyStatus";
import { useI18n } from "./i18n";
import { hasApiKeyScope } from "./permissions";
import type { ApiKeyPage, ApiKeySummary, MembershipSummary, Principal, Role, UserSummary } from "./types";

type DirectoryItem = UserSummary & { membershipRole: Role | null };

export interface UsersDirectoryProps {
  api: ApiClient;
  organizationId: string;
  principal: Principal;
  canManageUsers: boolean;
  canEditQuota: boolean;
  refreshVersion: number;
  onError: (message: string) => void;
  onEditQuota: (userId: string) => void;
}

export function UsersDirectory({ api, organizationId, principal, canManageUsers, canEditQuota, refreshVersion, onError, onEditQuota }: UsersDirectoryProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [size, setSize] = useState(50);
  const [items, setItems] = useState<DirectoryItem[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<(string | null)[]>([null]);
  const [pageNumber, setPageNumber] = useState(1);
  const [loading, setLoading] = useState(false);
  const loadingRef = useRef(false);
  const requestRef = useRef(0);

  useEffect(() => {
    let active = true;
    const requestId = ++requestRef.current;
    const search = query.trim() || undefined;
    // Prevent a role from the old organization/query from briefly offering an action.
    setItems([]);
    setNextCursor(null);
    setCursorHistory([null]);
    setPageNumber(1);
    loadingRef.current = true;
    setLoading(true);
    const timer = window.setTimeout(() => {
      void loadPageData(api, organizationId, size, search, canManageUsers, null)
        .then((page) => {
          if (!active || requestRef.current !== requestId) return;
          setItems(page.items);
          setNextCursor(page.nextCursor);
        })
        .catch((error) => {
          if (active && requestRef.current === requestId) onError(message(error, t("requestFailed")));
        })
        .finally(() => {
          if (requestRef.current !== requestId) return;
          loadingRef.current = false;
          if (active) setLoading(false);
        });
    }, 250);
    return () => { active = false; window.clearTimeout(timer); };
  }, [api, organizationId, query, size, canManageUsers, refreshVersion, onError, t]);

  async function loadPage(pageCursor: string | null) {
    if (loadingRef.current) return false;
    const requestId = ++requestRef.current;
    loadingRef.current = true;
    setLoading(true);
    try {
      const search = query.trim() || undefined;
      const page = await loadPageData(api, organizationId, size, search, canManageUsers, pageCursor);
      if (requestRef.current !== requestId) return false;
      setItems(page.items);
      setNextCursor(page.nextCursor);
      return true;
    } catch (error) {
      if (requestRef.current === requestId) onError(message(error, t("requestFailed")));
      return false;
    } finally {
      if (requestRef.current === requestId) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }

  async function nextPage() {
    if (!nextCursor) return;
    const pageCursor = nextCursor;
    if (await loadPage(pageCursor)) {
      setCursorHistory((history) => [...history, pageCursor]);
      setPageNumber((page) => page + 1);
    }
  }

  async function previousPage() {
    if (pageNumber <= 1) return;
    const pageCursor = cursorHistory[pageNumber - 2] ?? null;
    if (await loadPage(pageCursor)) {
      setCursorHistory((history) => history.slice(0, -1));
      setPageNumber((page) => page - 1);
    }
  }

  function updateUser(updated: UserSummary) {
    setItems((current) => current.map((user) => user.id === updated.id ? { ...updated, membershipRole: user.membershipRole } : user));
  }

  function updateMembership(userId: string, membershipRole: Role | null) {
    setItems((current) => current.map((user) => user.id === userId ? { ...user, membershipRole } : user));
  }

  return <div>
    <div className="form-actions"><label>{t("searchUsers")}<input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("searchUsersPlaceholder")} /></label><label>{t("rowsPerPage")}<select value={size} onChange={(event) => setSize(Number(event.target.value))}><option value="25">25</option><option value="50">50</option><option value="100">100</option></select></label></div>
    <div className="workspace-pagination" aria-label={t("userPagination")}><button className="button" type="button" disabled={pageNumber <= 1 || loading} onClick={() => void previousPage()}>{t("previousPage")}</button><span role="status">{t("userPageStatus")} {pageNumber} · {items.length}</span><button className="button" type="button" disabled={!nextCursor || loading} onClick={() => void nextPage()}>{t("nextPage")}</button></div>
    <div className="user-directory-list" role="list" aria-busy={loading} tabIndex={0} style={{ display: "grid", gap: "8px", maxHeight: "560px", overflowY: "auto", marginTop: "12px" }}>
      {items.map((user) => <UserDirectoryRow key={user.id} user={user} api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageUsers} canEditQuota={canEditQuota} onError={onError} onUpdated={updateUser} onMembershipChanged={updateMembership} onEditQuota={onEditQuota} />)}
      {items.length === 0 && loading && <p role="status">{t("loading")}</p>}
      {items.length === 0 && !loading && <p>{t("noUsers")}</p>}
    </div>
  </div>;
}

async function loadPageData(api: ApiClient, organizationId: string, size: number, search: string | undefined, canManageUsers: boolean, cursor: string | null): Promise<{ items: DirectoryItem[]; nextCursor: string | null }> {
  if (!canManageUsers) {
    const page = await api.membersPage(organizationId, { limit: size, search, cursor: cursor ?? undefined });
    return { items: page.items.map(memberToItem), nextCursor: page.next_cursor };
  }
  const users = await api.usersPage({ limit: size, search, cursor: cursor ?? undefined, organization_id: organizationId });
  return {
    items: users.items.map(userToItem),
    nextCursor: users.next_cursor,
  };
}

function userToItem(user: UserSummary): DirectoryItem {
  return { ...user, membershipRole: user.membership_role ?? null };
}

function memberToItem(membership: MembershipSummary): DirectoryItem {
  return { ...membership.user, membershipRole: membership.role };
}

function UserDirectoryRow({ user, api, organizationId, principal, canManageUsers, canEditQuota, onError, onUpdated, onMembershipChanged, onEditQuota }: { user: DirectoryItem; api: ApiClient; organizationId: string; principal: Principal; canManageUsers: boolean; canEditQuota: boolean; onError: (message: string) => void; onUpdated: (user: UserSummary) => void; onMembershipChanged: (userId: string, role: Role | null) => void; onEditQuota: (userId: string) => void }) {
  const { t } = useI18n();
  const [displayName, setDisplayName] = useState(user.display_name);
  const [systemAdmin, setSystemAdmin] = useState(user.system_admin);
  const [disabled, setDisabled] = useState(user.disabled);
  const [role, setRole] = useState<Role>(user.membershipRole ?? "member");
  const [saving, setSaving] = useState(false);
  const [membershipMessage, setMembershipMessage] = useState("");
  const [showApiKeys, setShowApiKeys] = useState(false);
  const isCurrentUser = user.id === principal.user_id;

  useEffect(() => {
    setDisplayName(user.display_name);
    setSystemAdmin(user.system_admin);
    setDisabled(user.disabled);
    setRole(user.membershipRole ?? "member");
  }, [user.id, user.display_name, user.system_admin, user.disabled, user.membershipRole]);

  async function save() {
    if (!displayName.trim()) return;
    setSaving(true);
    try {
      const updated = await api.updateUser(user.id, { display_name: displayName.trim(), system_admin: systemAdmin, disabled });
      onUpdated(updated);
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function saveMembership() {
    setSaving(true);
    try {
      await api.setMembership(organizationId, user.id, role);
      onMembershipChanged(user.id, role);
      setMembershipMessage(t("membershipSaved"));
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function removeMember() {
    if (!confirm(t("revokeMemberConfirm"))) return;
    setSaving(true);
    try {
      await api.removeMembership(organizationId, user.id);
      onMembershipChanged(user.id, null);
      setMembershipMessage(t("membershipRemoved"));
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  return <>
    <article role="listitem" style={{ display: "grid", gap: "9px", padding: "12px", border: "1px solid var(--line)", borderRadius: "9px", background: "var(--surface-2)" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: "10px" }}><strong>{user.display_name}</strong><span>{user.disabled ? t("userStatusDisabled") : t("userStatusActive")}</span></div>
      {canManageUsers && <>
        <code>{user.id}</code>
        <label>{t("displayName")}<input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label>
        <div className="check-row"><label><input type="checkbox" checked={systemAdmin} disabled={isCurrentUser} onChange={(event) => setSystemAdmin(event.target.checked)} />{t("systemAdmin")}</label><label><input type="checkbox" checked={disabled} disabled={isCurrentUser} onChange={(event) => setDisabled(event.target.checked)} />{disabled ? t("enableUser") : t("disableUser")}</label></div>
        <div className="form-actions" style={{ flexWrap: "wrap", gap: "8px" }}><button className="button primary" disabled={saving || !displayName.trim()} onClick={() => void save()}>{saving ? t("saving") : t("saveUser")}</button>{principal.system_admin && !isCurrentUser && hasApiKeyScope(principal, "manage_system") && hasApiKeyScope(principal, "manage_api_keys") && <button className="button" type="button" disabled={saving} onClick={() => setShowApiKeys(true)}>{t("manageUserApiKeys")}</button>}</div>
      </>}
      {canEditQuota && <div className="form-actions"><button className="button" disabled={saving} onClick={() => onEditQuota(user.id)}>{t("editUserQuota")}</button></div>}
      <div className="form-actions" style={{ flexWrap: "wrap", gap: "8px" }}><label>{t("role")}<select value={role} disabled={saving} onChange={(event) => setRole(event.target.value as Role)}><option value="member">{t("roleMember")}</option><option value="organization_admin">{t("roleOrganizationAdmin")}</option></select></label><button className="button" disabled={saving} onClick={() => void saveMembership()}>{user.membershipRole ? t("saveMembership") : t("addOrganizationMember")}</button>{user.membershipRole && <button className="button danger" disabled={saving} onClick={() => void removeMember()}>{t("removeOrganizationMember")}</button>}</div>
      {membershipMessage && <small role="status">{membershipMessage}</small>}
    </article>
    {showApiKeys && <AdminUserApiKeysDialog api={api} userId={user.id} userDisplayName={user.display_name} onClose={() => setShowApiKeys(false)} onError={onError} />}
  </>;
}

function AdminUserApiKeysDialog({ api, userId, userDisplayName, onClose, onError }: { api: ApiClient; userId: string; userDisplayName: string; onClose: () => void; onError: (message: string) => void }) {
  const { locale, t } = useI18n();
  const [items, setItems] = useState<ApiKeySummary[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<(string | null)[]>([null]);
  const [pageNumber, setPageNumber] = useState(1);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retryRequest, setRetryRequest] = useState<RetryRequest | null>(null);
  const [revokingKeyId, setRevokingKeyId] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [revoking, setRevoking] = useState(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const requestRef = useRef(0);
  const loadingRef = useRef(false);
  const localRevocationsRef = useRef(new Map<string, number>());

  useEffect(() => {
    dialogRef.current?.showModal();
    return () => dialogRef.current?.close();
  }, []);

  useEffect(() => {
    setItems([]);
    setNextCursor(null);
    setCursorHistory([null]);
    setPageNumber(1);
    void loadPage(null, "reset", true);
    return () => {
      requestRef.current += 1;
      loadingRef.current = false;
    };
  }, [api, userId, onError, t]);

  async function loadPage(cursor: string | null, navigation: PageNavigation = "stay", force = false): Promise<ApiKeyPage | null> {
    if (loadingRef.current && !force) return null;
    const requestId = ++requestRef.current;
    loadingRef.current = true;
    setLoading(true);
    setError(null);
    setRetryRequest(null);
    try {
      const page = await api.adminUserApiKeys(userId, { status: "all", limit: 25, cursor: cursor ?? undefined });
      if (requestRef.current !== requestId) return null;
      const normalizedPage = { ...page, items: applyLocalRevocations(page.items, localRevocationsRef.current) };
      setItems(normalizedPage.items);
      setNextCursor(page.next_cursor);
      if (navigation === "reset") {
        setCursorHistory([null]);
        setPageNumber(1);
      } else if (navigation === "next") {
        setCursorHistory((history) => [...history, cursor]);
        setPageNumber((pageNumberValue) => pageNumberValue + 1);
      } else if (navigation === "previous") {
        setCursorHistory((history) => history.slice(0, -1));
        setPageNumber((pageNumberValue) => Math.max(1, pageNumberValue - 1));
      }
      return normalizedPage;
    } catch (error) {
      if (requestRef.current === requestId) {
        const failure = message(error, t("requestFailed"));
        setError(failure);
        setRetryRequest({ cursor, navigation });
        onError(failure);
      }
      return null;
    } finally {
      if (requestRef.current === requestId) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }

  async function nextPage() {
    if (!nextCursor) return;
    const cursor = nextCursor;
    await loadPage(cursor, "next");
  }

  async function previousPage() {
    if (pageNumber <= 1) return;
    const cursor = cursorHistory[pageNumber - 2] ?? null;
    await loadPage(cursor, "previous");
  }

  async function retry() {
    if (!retryRequest) return;
    await loadPage(retryRequest.cursor, retryRequest.navigation);
  }

  async function revoke(event: FormEvent, key: ApiKeySummary) {
    event.preventDefault();
    const trimmedReason = reason.trim();
    if (!trimmedReason) return;
    setRevoking(true);
    try {
      await api.revokeAdminUserApiKey(userId, key.id, trimmedReason);
      const revokedAt = Math.floor(Date.now() / 1_000);
      localRevocationsRef.current.set(key.id, revokedAt);
      setItems((current) => current.map((item) => item.id === key.id ? { ...item, revoked_at: item.revoked_at ?? revokedAt } : item));
      setRevokingKeyId(null);
      setReason("");
      const refreshedPage = await loadPage(cursorHistory[pageNumber - 1] ?? null);
      if (refreshedPage?.items.length === 0 && pageNumber > 1) {
        const previousCursor = cursorHistory[pageNumber - 2] ?? null;
        await loadPage(previousCursor, "previous");
      }
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setRevoking(false);
    }
  }

  const close = () => {
    if (!revoking) onClose();
  };

  return <dialog ref={dialogRef} className="connection-dialog" aria-labelledby="admin-api-keys-title" onCancel={(event) => { event.preventDefault(); if (!revoking) onClose(); }}>
    <div className="connection-dialog-content">
      <header><div><h3 id="admin-api-keys-title">{t("manageUserApiKeys")}</h3><p>{t("manageUserApiKeysHelp")} {userDisplayName}</p></div><button className="connection-dialog-close" type="button" aria-label={t("close")} disabled={revoking} onClick={close}>×</button></header>
      {error && <div className="error-banner" role="alert"><span>{error}</span> <button className="button" type="button" disabled={loading || !retryRequest} onClick={() => void retry()}>{t("retryApiKeys")}</button></div>}
      <div className="workspace-pagination" aria-label={t("apiKeyPagination")}><button className="button" type="button" disabled={pageNumber <= 1 || loading || revoking} onClick={() => void previousPage()}>{t("previousPage")}</button><span role="status">{formatApiKeyPageStatus(locale, pageNumber, items.length, t)}</span><button className="button" type="button" disabled={!nextCursor || loading || revoking} onClick={() => void nextPage()}>{t("nextPage")}</button></div>
      {loading && items.length === 0 && !error ? <div role="status" aria-label={t("loading")} style={{ display: "grid", gap: "8px" }}><div className="loading-skeleton" style={{ height: "78px" }} /><div className="loading-skeleton" style={{ height: "78px" }} /></div> : items.length === 0 && !error ? <p>{t("noApiKeys")}</p> : items.length > 0 ? <div style={{ display: "grid", gap: "8px", maxHeight: "480px", overflowY: "auto" }}>
        {items.map((key) => <article key={key.id} style={{ display: "grid", gap: "10px", padding: "12px", border: "1px solid var(--line)", borderRadius: "9px", background: "var(--surface-2)" }}>
          <div style={{ display: "grid", gap: "4px" }}><strong>{key.name}</strong><code>{key.prefix}</code><small>{formatApiKeyScopes(key, t)}</small></div>
          <dl className="connection-facts"><div><dt>{t("createdAt")}</dt><dd>{formatTime(key.created_at, locale)}</dd></div><div><dt>{t("lastUsedAt")}</dt><dd>{key.last_used_at ? formatTime(key.last_used_at, locale) : t("never")}</dd></div><div><dt>{t("apiKeyExpires")}</dt><dd>{formatApiKeyExpiry(key.expires_at, locale, t)}</dd></div><div><dt>{t("revokedAt")}</dt><dd>{key.revoked_at !== null ? formatTime(key.revoked_at, locale) : t("never")}</dd></div><div><dt>{t("apiKeyStatus")}</dt><dd>{formatApiKeyStatus(key, t)}</dd></div></dl>
          {getApiKeyStatus(key) === "active" && (revokingKeyId === key.id ? <form onSubmit={(event) => void revoke(event, key)} style={{ display: "grid", gap: "8px" }}><label>{t("apiKeyRevocationReason")}<textarea required maxLength={500} value={reason} onChange={(event) => setReason(event.target.value)} placeholder={t("apiKeyRevocationReasonPlaceholder")} /></label><small>{t("apiKeyRevocationReasonHelp")}</small><div className="form-actions"><button className="button danger" disabled={revoking || !reason.trim()}>{revoking ? t("saving") : t("revokeApiKey")}</button><button className="button" type="button" disabled={revoking} onClick={() => { setRevokingKeyId(null); setReason(""); }}>{t("cancel")}</button></div></form> : <button className="button danger" type="button" disabled={revoking || loading} onClick={() => { setRevokingKeyId(key.id); setReason(""); }}>{t("revokeApiKey")}</button>)}
        </article>)}
      </div> : null}
    </div>
  </dialog>;
}

type PageNavigation = "reset" | "next" | "previous" | "stay";

interface RetryRequest {
  cursor: string | null;
  navigation: PageNavigation;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
