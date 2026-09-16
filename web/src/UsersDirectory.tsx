import { useEffect, useRef, useState } from "react";
import {
  Button,
  Checkbox,
  DataGrid,
  DataGridBody,
  DataGridCell,
  DataGridHeader,
  DataGridHeaderCell,
  DataGridRow,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Option,
  Select,
  Spinner,
  Text,
  Textarea,
} from "@fluentui/react-components";
import type { TableColumnDefinition } from "@fluentui/react-components";
import { ArrowLeftRegular, ArrowRightRegular, EditRegular, KeyRegular, SaveRegular } from "@fluentui/react-icons";

import {
  formatApiKeyExpiry,
  formatApiKeyPageStatus,
  formatApiKeyScopes,
  formatApiKeyStatus,
  formatTime,
} from "./adminApiKeyView";
import type { ApiClient } from "./api";
import { applyLocalRevocations, getApiKeyStatus } from "./apiKeyStatus";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import { hasApiKeyScope } from "./permissions";
import type { ApiKeyPage, ApiKeySummary, MembershipSummary, Principal, Role, UserSummary } from "./types";
import { AdminToolbar, SaveButton, useAdminStyles } from "./admin/fluentAdmin";

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
  const styles = useAdminStyles();
  const [query, setQuery] = useState("");
  const [size, setSize] = useState(50);
  const [items, setItems] = useState<DirectoryItem[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<(string | null)[]>([null]);
  const [pageNumber, setPageNumber] = useState(1);
  const [loading, setLoading] = useState(false);
  const loadingRef = useRef(false);
  const requestRef = useRef(0);
  const [selectedUser, setSelectedUser] = useState<DirectoryItem | null>(null);
  const [apiKeyUser, setApiKeyUser] = useState<DirectoryItem | null>(null);

  useEffect(() => {
    let active = true;
    const requestId = ++requestRef.current;
    setItems([]);
    setNextCursor(null);
    setCursorHistory([null]);
    setPageNumber(1);
    loadingRef.current = true;
    setLoading(true);
    const timer = window.setTimeout(() => {
      void loadPageData(api, organizationId, size, query.trim() || undefined, canManageUsers, null)
        .then((page) => {
          if (!active || requestRef.current !== requestId) return;
          setItems(page.items);
          setNextCursor(page.nextCursor);
        })
        .catch((error) => { if (active && requestRef.current === requestId) onError(message(error, t("requestFailed"))); })
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
      const page = await loadPageData(api, organizationId, size, query.trim() || undefined, canManageUsers, pageCursor);
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
    const cursor = nextCursor;
    if (await loadPage(cursor)) {
      setCursorHistory((history) => [...history, cursor]);
      setPageNumber((page) => page + 1);
    }
  }

  async function previousPage() {
    if (pageNumber <= 1) return;
    const cursor = cursorHistory[pageNumber - 2] ?? null;
    if (await loadPage(cursor)) {
      setCursorHistory((history) => history.slice(0, -1));
      setPageNumber((page) => page - 1);
    }
  }

  function updateUser(updated: UserSummary) {
    setItems((current) => current.map((user) => user.id === updated.id ? { ...updated, membershipRole: user.membershipRole } : user));
    setSelectedUser((user) => user?.id === updated.id ? { ...updated, membershipRole: user.membershipRole } : user);
  }

  function updateMembership(userId: string, membershipRole: Role | null) {
    setItems((current) => current.map((user) => user.id === userId ? { ...user, membershipRole } : user));
    setSelectedUser((user) => user?.id === userId ? { ...user, membershipRole } : user);
  }

  return <div className={styles.stack}>
    <AdminToolbar action={<div className={styles.actions}>
      <Field label={t("rowsPerPage")}><Select value={String(size)} onChange={(event) => setSize(Number(event.target.value))}><Option value="25">25</Option><Option value="50">50</Option><Option value="100">100</Option></Select></Field>
      <Button icon={<ArrowLeftRegular />} disabled={pageNumber <= 1 || loading} onClick={() => void previousPage()}>{t("previousPage")}</Button>
      <Button icon={<ArrowRightRegular />} iconPosition="after" disabled={!nextCursor || loading} onClick={() => void nextPage()}>{t("nextPage")}</Button>
    </div>}>
      <Field label={t("searchUsers")}><Input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("searchUsersPlaceholder")} /></Field>
      <Text size={300}>{t("userPageStatus")} {pageNumber} · {items.length}</Text>
    </AdminToolbar>
    {loading && items.length === 0 ? <Spinner label={t("loading")} /> : items.length === 0 ? <Text className={styles.empty}>{t("noUsers")}</Text> : <div className={styles.table} aria-busy={loading}>
      <DataGrid items={items} columns={userColumns({ t, styles, canManageUsers, canEditQuota, principal, setSelectedUser, setApiKeyUser, onEditQuota })}>
        <DataGridHeader><DataGridRow<DirectoryItem>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader>
        <DataGridBody<DirectoryItem>>{({ item }) => <DataGridRow<DirectoryItem>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody>
      </DataGrid>
    </div>}
    {selectedUser && <UserEditDialog user={selectedUser} api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageUsers} onClose={() => setSelectedUser(null)} onError={onError} onUpdated={updateUser} onMembershipChanged={updateMembership} />}
    {apiKeyUser && <AdminUserApiKeysDialog api={api} userId={apiKeyUser.id} userDisplayName={apiKeyUser.display_name} onClose={() => setApiKeyUser(null)} onError={onError} />}
  </div>;
}

async function loadPageData(api: ApiClient, organizationId: string, size: number, search: string | undefined, canManageUsers: boolean, cursor: string | null): Promise<{ items: DirectoryItem[]; nextCursor: string | null }> {
  if (!canManageUsers) {
    const page = await api.membersPage(organizationId, { limit: size, search, cursor: cursor ?? undefined });
    return { items: page.items.map(memberToItem), nextCursor: page.next_cursor };
  }
  const users = await api.usersPage({ limit: size, search, cursor: cursor ?? undefined, organization_id: organizationId });
  return { items: users.items.map(userToItem), nextCursor: users.next_cursor };
}

function userToItem(user: UserSummary): DirectoryItem { return { ...user, membershipRole: user.membership_role ?? null }; }
function memberToItem(membership: MembershipSummary): DirectoryItem { return { ...membership.user, membershipRole: membership.role }; }

function UserEditDialog({ user, api, organizationId, principal, canManageUsers, onClose, onError, onUpdated, onMembershipChanged }: { user: DirectoryItem; api: ApiClient; organizationId: string; principal: Principal; canManageUsers: boolean; onClose: () => void; onError: (message: string) => void; onUpdated: (user: UserSummary) => void; onMembershipChanged: (userId: string, role: Role | null) => void }) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const [displayName, setDisplayName] = useState(user.display_name);
  const [systemAdmin, setSystemAdmin] = useState(user.system_admin);
  const [disabled, setDisabled] = useState(user.disabled);
  const [role, setRole] = useState<Role>(user.membershipRole ?? "member");
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState("");
  const [confirmRemove, setConfirmRemove] = useState(false);
  const isCurrentUser = user.id === principal.user_id;

  async function save() {
    if (!displayName.trim()) return;
    setSaving(true);
    try {
      if (canManageUsers) onUpdated(await api.updateUser(user.id, { display_name: displayName.trim(), system_admin: systemAdmin, disabled }));
      await api.setMembership(organizationId, user.id, role);
      onMembershipChanged(user.id, role);
      setStatus(t("userSaved"));
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  async function removeMember() {
    setSaving(true);
    try {
      await api.removeMembership(organizationId, user.id);
      setConfirmRemove(false);
      onMembershipChanged(user.id, null);
      onClose();
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setSaving(false);
    }
  }

  return <><Dialog open onOpenChange={(_, data) => { if (!data.open && !saving) onClose(); }}><DialogSurface><DialogBody><DialogTitle>{t("saveUser")} · {user.display_name}</DialogTitle><DialogContent className={styles.dialogBody}>
    {canManageUsers && <><Field label={t("displayName")} required><Input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></Field><Checkbox checked={systemAdmin} disabled={isCurrentUser} onChange={(_, data) => setSystemAdmin(Boolean(data.checked))} label={t("systemAdmin")} /><Checkbox checked={disabled} disabled={isCurrentUser} onChange={(_, data) => setDisabled(Boolean(data.checked))} label={t("disableUser")} /></>}
    <Field label={t("role")}><Select value={role} disabled={saving} onChange={(event) => setRole(event.target.value as Role)}><Option value="member">{t("roleMember")}</Option><Option value="organization_admin">{t("roleOrganizationAdmin")}</Option></Select></Field>
    {status && <MessageBar intent="success"><MessageBarBody>{status}</MessageBarBody></MessageBar>}
  </DialogContent><DialogActions><Button appearance="secondary" disabled={saving} onClick={onClose}>{t("close")}</Button>{user.membershipRole && <Button disabled={saving} onClick={() => setConfirmRemove(true)}>{t("removeOrganizationMember")}</Button>}<SaveButton icon={<SaveRegular />} disabled={saving || !displayName.trim()} onClick={() => void save()}>{saving ? t("saving") : t("saveUser")}</SaveButton></DialogActions></DialogBody></DialogSurface></Dialog>
  <ConfirmDialog
    open={confirmRemove}
    title={t("removeOrganizationMember")}
    description={isCurrentUser ? t("memberRemoveSelfConfirm") : t("memberRemoveConfirm")}
    confirmLabel={t("removeOrganizationMember")}
    cancelLabel={t("cancel")}
    busy={saving}
    danger
    details={<strong>{user.display_name}</strong>}
    onClose={() => setConfirmRemove(false)}
    onConfirm={() => void removeMember()}
  />
  </>;
}

function AdminUserApiKeysDialog({ api, userId, userDisplayName, onClose, onError }: { api: ApiClient; userId: string; userDisplayName: string; onClose: () => void; onError: (message: string) => void }) {
  const { locale, t } = useI18n();
  const styles = useAdminStyles();
  const [items, setItems] = useState<ApiKeySummary[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<(string | null)[]>([null]);
  const [pageNumber, setPageNumber] = useState(1);
  const [loading, setLoading] = useState(true);
  const [revokingKeyId, setRevokingKeyId] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [revoking, setRevoking] = useState(false);
  const localRevocationsRef = useRef(new Map<string, number>());

  useEffect(() => { void loadPage(null, "reset"); }, [api, userId]);

  async function loadPage(cursor: string | null, navigation: "reset" | "next" | "previous" = "reset") {
    setLoading(true);
    try {
      const page = await api.adminUserApiKeys(userId, { status: "all", limit: 25, cursor: cursor ?? undefined });
      const normalized = { ...page, items: applyLocalRevocations(page.items, localRevocationsRef.current) };
      setItems(normalized.items);
      setNextCursor(page.next_cursor);
      if (navigation === "reset") { setCursorHistory([null]); setPageNumber(1); }
      if (navigation === "next") { setCursorHistory((history) => [...history, cursor]); setPageNumber((value) => value + 1); }
      if (navigation === "previous") { setCursorHistory((history) => history.slice(0, -1)); setPageNumber((value) => Math.max(1, value - 1)); }
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setLoading(false);
    }
  }

  async function revoke(key: ApiKeySummary) {
    if (!reason.trim()) return;
    setRevoking(true);
    try {
      await api.revokeAdminUserApiKey(userId, key.id, reason.trim());
      const revokedAt = Math.floor(Date.now() / 1_000);
      localRevocationsRef.current.set(key.id, revokedAt);
      setItems((current) => current.map((item) => item.id === key.id ? { ...item, revoked_at: item.revoked_at ?? revokedAt } : item));
      setRevokingKeyId(null);
      setReason("");
    } catch (error) {
      onError(message(error, t("requestFailed")));
    } finally {
      setRevoking(false);
    }
  }

  const revokingKey = revokingKeyId ? items.find((item) => item.id === revokingKeyId) ?? null : null;

  return <>
  <Dialog open onOpenChange={(_, data) => { if (!data.open && !revoking) onClose(); }}><DialogSurface><DialogBody><DialogTitle>{t("manageUserApiKeys")} · {userDisplayName}</DialogTitle><DialogContent className={styles.dialogBody}>
    <AdminToolbar action={<div className={styles.actions}><Button icon={<ArrowLeftRegular />} disabled={pageNumber <= 1 || loading || revoking} onClick={() => void loadPage(cursorHistory[pageNumber - 2] ?? null, "previous")}>{t("previousPage")}</Button><Button icon={<ArrowRightRegular />} iconPosition="after" disabled={!nextCursor || loading || revoking} onClick={() => void loadPage(nextCursor, "next")}>{t("nextPage")}</Button></div>}><Text size={300}>{formatApiKeyPageStatus(locale, pageNumber, items.length, t)}</Text></AdminToolbar>
    {loading && items.length === 0 ? <Spinner label={t("loading")} /> : items.length === 0 ? <Text className={styles.empty}>{t("noApiKeys")}</Text> : <DataGrid items={items} columns={apiKeyColumns({ t, locale, styles, revokingKeyId, revoking, loading, setRevokingKeyId, setReason })}><DataGridHeader><DataGridRow<ApiKeySummary>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader><DataGridBody<ApiKeySummary>>{({ item }) => <DataGridRow<ApiKeySummary>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody></DataGrid>}
  </DialogContent><DialogActions><Button appearance="secondary" disabled={revoking} onClick={onClose}>{t("close")}</Button></DialogActions></DialogBody></DialogSurface></Dialog>
  <ConfirmDialog
    open={revokingKey !== null}
    title={t("revokeApiKey")}
    description={t("revokeApiKeyConfirm")}
    confirmLabel={t("revokeApiKey")}
    cancelLabel={t("cancel")}
    busy={revoking}
    confirmDisabled={!reason.trim()}
    danger
    details={revokingKey && <div className={styles.stack}><strong>{revokingKey.name}</strong><Text size={200}>{revokingKey.prefix}</Text><Field label={t("apiKeyRevocationReason")} required><Textarea value={reason} onChange={(event) => setReason(event.target.value)} placeholder={t("apiKeyRevocationReasonPlaceholder")} /></Field></div>}
    onClose={() => { if (!revoking) { setRevokingKeyId(null); setReason(""); } }}
    onConfirm={() => { if (revokingKey) void revoke(revokingKey); }}
  />
  </>;
}

function userColumns({ t, styles, canManageUsers, canEditQuota, principal, setSelectedUser, setApiKeyUser, onEditQuota }: { t: ReturnType<typeof useI18n>["t"]; styles: ReturnType<typeof useAdminStyles>; canManageUsers: boolean; canEditQuota: boolean; principal: Principal; setSelectedUser: (user: DirectoryItem) => void; setApiKeyUser: (user: DirectoryItem) => void; onEditQuota: (userId: string) => void }): TableColumnDefinition<DirectoryItem>[] {
  return [
    { columnId: "user", compare: (a, b) => a.display_name.localeCompare(b.display_name), renderHeaderCell: () => t("user"), renderCell: (item) => <div className={styles.stack}><Text weight="semibold">{item.display_name}</Text><Text size={200}>{item.id}</Text></div> },
    { columnId: "role", compare: (a, b) => String(a.membershipRole).localeCompare(String(b.membershipRole)), renderHeaderCell: () => t("role"), renderCell: (item) => item.membershipRole === "organization_admin" ? t("roleOrganizationAdmin") : item.membershipRole === "member" ? t("roleMember") : t("notEnabled") },
    { columnId: "status", compare: (a, b) => Number(a.disabled) - Number(b.disabled), renderHeaderCell: () => t("workspaceState"), renderCell: (item) => item.disabled ? t("userStatusDisabled") : t("userStatusActive") },
    { columnId: "actions", compare: () => 0, renderHeaderCell: () => t("actions"), renderCell: (item) => <div className={styles.actions}>{(canManageUsers || item.membershipRole !== null) && <Button icon={<EditRegular />} onClick={() => setSelectedUser(item)}>{t("editUser")}</Button>}{canEditQuota && <Button onClick={() => onEditQuota(item.id)}>{t("editUserQuota")}</Button>}{principal.system_admin && item.id !== principal.user_id && hasApiKeyScope(principal, "manage_system") && hasApiKeyScope(principal, "manage_api_keys") && <Button icon={<KeyRegular />} onClick={() => setApiKeyUser(item)}>{t("manageUserApiKeys")}</Button>}</div> },
  ];
}

function apiKeyColumns({ t, locale, styles, revokingKeyId, revoking, loading, setRevokingKeyId, setReason }: { t: ReturnType<typeof useI18n>["t"]; locale: string; styles: ReturnType<typeof useAdminStyles>; revokingKeyId: string | null; revoking: boolean; loading: boolean; setRevokingKeyId: (id: string | null) => void; setReason: (reason: string) => void }): TableColumnDefinition<ApiKeySummary>[] {
  return [
    { columnId: "key", compare: (a, b) => a.name.localeCompare(b.name), renderHeaderCell: () => t("manageUserApiKeys"), renderCell: (item) => <div className={styles.stack}><Text weight="semibold">{item.name}</Text><Text size={200}>{item.prefix}</Text><Text size={200}>{formatApiKeyScopes(item, t)}</Text></div> },
    { columnId: "status", compare: (a, b) => getApiKeyStatus(a).localeCompare(getApiKeyStatus(b)), renderHeaderCell: () => t("apiKeyStatus"), renderCell: (item) => formatApiKeyStatus(item, t) },
    { columnId: "expires", compare: (a, b) => (a.expires_at ?? 0) - (b.expires_at ?? 0), renderHeaderCell: () => t("apiKeyExpires"), renderCell: (item) => formatApiKeyExpiry(item.expires_at, locale, t) },
    { columnId: "actions", compare: () => 0, renderHeaderCell: () => t("actions"), renderCell: (item) => getApiKeyStatus(item) === "active" && <Button disabled={revoking || loading || revokingKeyId !== null} onClick={() => { setRevokingKeyId(item.id); setReason(""); }}>{t("revokeApiKey")}</Button> },
  ];
}

function message(error: unknown, fallback: string) { return error instanceof Error ? error.message : fallback; }
