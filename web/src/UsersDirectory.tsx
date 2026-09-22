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
  Tab,
  TabList,
  Text,
  Textarea,
  Tooltip,
} from "@fluentui/react-components";
import type { TableColumnDefinition } from "@fluentui/react-components";
import { AddRegular, ArrowLeftRegular, ArrowRightRegular, CopyRegular, EditRegular, KeyRegular, SaveRegular } from "@fluentui/react-icons";

import {
  formatApiKeyExpiry,
  formatApiKeyPageStatus,
  formatApiKeyScopes,
  formatApiKeyStatus,
  formatTime,
} from "./adminApiKeyView";
import type { ApiClient } from "./api";
import { applyLocalRevocations, getApiKeyStatus, prependCreatedApiKey } from "./apiKeyStatus";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { useI18n } from "./i18n";
import { hasApiKeyScope } from "./permissions";
import type { AdminApiKey, ApiKeyPage, ApiKeyScope, ApiKeySummary, MembershipSummary, Principal, Role, UserSummary, WorkspaceTemplate } from "./types";
import { API_KEY_SCOPES } from "./apiKeyScopes";
import { AdminToolbar, SaveButton, useAdminStyles } from "./admin/fluentAdmin";
import { ApiKeyTemplatePicker } from "./admin/ApiKeyTemplatePicker";

type DirectoryItem = UserSummary & { membershipRole: Role | null };
type UserEditorSection = "details" | "apiKeys";

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
  const [selectedSection, setSelectedSection] = useState<UserEditorSection>("details");
  const canManageApiKeys = principal.system_admin && hasApiKeyScope(principal, "manage_system") && hasApiKeyScope(principal, "manage_api_keys");

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

  function openUser(user: DirectoryItem, section: UserEditorSection) {
    setSelectedSection(section);
    setSelectedUser(user);
  }

  return <div className={styles.stack}>
    <AdminToolbar action={<div className={styles.actions}>
      <Field label={t("rowsPerPage")}><Select value={String(size)} onChange={(event) => setSize(Number(event.target.value))}><Option value="25">25</Option><Option value="50">50</Option><Option value="100">100</Option></Select></Field>
      <Button icon={<ArrowLeftRegular />} disabled={pageNumber <= 1 || loading} onClick={() => void previousPage()}>{t("previousPage")}</Button>
      <Button icon={<ArrowRightRegular />} iconPosition="after" disabled={!nextCursor || loading} onClick={() => void nextPage()}>{t("nextPage")}</Button>
    </div>}>
      <Field label={t("searchUsers")}><Input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("searchUsersPlaceholder")} /></Field>
      <div className={styles.stack}>
        <Text size={300}>{t("userPageStatus")} {pageNumber} · {items.length}</Text>
        {canManageApiKeys && <Text size={200} className={styles.muted}>{t("apiKeyManagementHint")}</Text>}
      </div>
    </AdminToolbar>
    {loading && items.length === 0 ? <Spinner label={t("loading")} /> : items.length === 0 ? <Text className={styles.empty}>{t("noUsers")}</Text> : <div className={styles.table} aria-busy={loading}>
      <DataGrid items={items} columns={userColumns({ t, styles, canManageUsers, canEditQuota, canManageApiKeys, openUser, onEditQuota })}>
        <DataGridHeader><DataGridRow<DirectoryItem>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader>
        <DataGridBody<DirectoryItem>>{({ item }) => <DataGridRow<DirectoryItem>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody>
      </DataGrid>
    </div>}
    {selectedUser && <UserEditDialog user={selectedUser} initialSection={selectedSection} api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageUsers} onClose={() => setSelectedUser(null)} onError={onError} onUpdated={updateUser} onMembershipChanged={updateMembership} />}
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

function UserEditDialog({ user, initialSection, api, organizationId, principal, canManageUsers, onClose, onError, onUpdated, onMembershipChanged }: { user: DirectoryItem; initialSection: UserEditorSection; api: ApiClient; organizationId: string; principal: Principal; canManageUsers: boolean; onClose: () => void; onError: (message: string) => void; onUpdated: (user: UserSummary) => void; onMembershipChanged: (userId: string, role: Role | null) => void }) {
  const { t } = useI18n();
  const styles = useAdminStyles();
  const [displayName, setDisplayName] = useState(user.display_name);
  const [systemAdmin, setSystemAdmin] = useState(user.system_admin);
  const [disabled, setDisabled] = useState(user.disabled);
  const [role, setRole] = useState<Role>(user.membershipRole ?? "member");
  const [saving, setSaving] = useState(false);
  const [managingKeys, setManagingKeys] = useState(false);
  const [section, setSection] = useState<UserEditorSection>(initialSection);
  const [status, setStatus] = useState("");
  const [confirmRemove, setConfirmRemove] = useState(false);
  const isCurrentUser = user.id === principal.user_id;
  const canManageApiKeys = principal.system_admin && hasApiKeyScope(principal, "manage_system") && hasApiKeyScope(principal, "manage_api_keys");

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

  return <><Dialog open onOpenChange={(_, data) => { if (!data.open && !saving && !managingKeys) onClose(); }}><DialogSurface className={styles.dialogSurface}><DialogBody><DialogTitle>{t("saveUser")} · {user.display_name}</DialogTitle><DialogContent className={styles.dialogBody}>
    <TabList selectedValue={section} onTabSelect={(_, data) => setSection(data.value as UserEditorSection)}>
      <Tab value="details">{t("user")}</Tab>
      {canManageApiKeys && <Tab value="apiKeys">{t("apiKeys")}</Tab>}
    </TabList>
    {section === "details" ? <>
      {canManageUsers && <><Field label={t("displayName")} required><Input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></Field><Checkbox checked={systemAdmin} disabled={isCurrentUser} onChange={(_, data) => setSystemAdmin(Boolean(data.checked))} label={t("systemAdmin")} /><Checkbox checked={disabled} disabled={isCurrentUser} onChange={(_, data) => setDisabled(Boolean(data.checked))} label={t("disableUser")} /></>}
      <Field label={t("role")}><Select value={role} disabled={saving} onChange={(event) => setRole(event.target.value as Role)}><Option value="member">{t("roleMember")}</Option><Option value="organization_admin">{t("roleOrganizationAdmin")}</Option></Select></Field>
      {status && <MessageBar intent="success"><MessageBarBody>{status}</MessageBarBody></MessageBar>}
    </> : <UserApiKeysPanel api={api} organizationId={organizationId} principal={principal} userId={user.id} isCurrentUser={isCurrentUser} onBusyChange={setManagingKeys} onError={onError} />}
  </DialogContent><DialogActions><Button appearance="secondary" disabled={saving || managingKeys} onClick={onClose}>{t("close")}</Button>{section === "details" && <>{user.membershipRole && <Button disabled={saving} onClick={() => setConfirmRemove(true)}>{t("removeOrganizationMember")}</Button>}<SaveButton icon={<SaveRegular />} disabled={saving || !displayName.trim()} onClick={() => void save()}>{saving ? t("saving") : t("saveUser")}</SaveButton></>}</DialogActions></DialogBody></DialogSurface></Dialog>
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

function UserApiKeysPanel({ api, organizationId, principal, userId, isCurrentUser, onBusyChange, onError }: { api: ApiClient; organizationId: string; principal: Principal; userId: string; isCurrentUser: boolean; onBusyChange: (busy: boolean) => void; onError: (message: string) => void }) {
  const { locale, t } = useI18n();
  const styles = useAdminStyles();
  const [items, setItems] = useState<AdminApiKey[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [cursorHistory, setCursorHistory] = useState<(string | null)[]>([null]);
  const [pageNumber, setPageNumber] = useState(1);
  const [loading, setLoading] = useState(true);
  const [revokingKeyId, setRevokingKeyId] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [revoking, setRevoking] = useState(false);
  const [creating, setCreating] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [name, setName] = useState("");
  const [expiresAt, setExpiresAt] = useState(defaultExpiry);
  const [scopes, setScopes] = useState<ApiKeyScope[]>(() => principal.api_key_scopes.includes("read_workspace") ? ["read_workspace"] : principal.api_key_scopes.slice(0, 1));
  const [templates, setTemplates] = useState<WorkspaceTemplate[]>([]);
  const [templateRestriction, setTemplateRestriction] = useState(false);
  const [allowedTemplateIds, setAllowedTemplateIds] = useState<string[]>([]);
  const [copiedKeyId, setCopiedKeyId] = useState<string | null>(null);
  const localRevocationsRef = useRef(new Map<string, number>());

  useEffect(() => { void loadPage(null, "reset"); }, [api, userId]);
  useEffect(() => onBusyChange(revoking || creating), [creating, onBusyChange, revoking]);
  useEffect(() => () => onBusyChange(false), [onBusyChange]);
  useEffect(() => {
    let active = true;
    void api.templates(organizationId).then((items) => { if (active) setTemplates(items); }).catch((error) => { if (active) onError(message(error, t("requestFailed"))); });
    return () => { active = false; };
  }, [api, onError, organizationId, t]);

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
    if (!isCurrentUser && !reason.trim()) return;
    setRevoking(true);
    try {
      if (isCurrentUser) await api.deleteApiKey(key.id);
      else await api.revokeAdminUserApiKey(userId, key.id, reason.trim());
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

  async function createKey() {
    const expires = Math.floor(new Date(expiresAt).getTime() / 1_000);
    if (!name.trim() || !Number.isFinite(expires) || scopes.length === 0) return;
    setCreating(true);
    try {
      const created = await api.createAdminUserApiKey(userId, { name: name.trim(), scopes, expires_at: expires, allowed_template_ids: templateRestriction ? allowedTemplateIds : null });
      setName("");
      setShowCreate(false);
      setTemplateRestriction(false);
      setAllowedTemplateIds([]);
      await loadPage(null, "reset");
      setItems((current) => prependCreatedApiKey(current, created));
      try { await navigator.clipboard.writeText(created.token); setCopiedKeyId(created.id); } catch { /* Value remains visible in the list. */ }
    } catch (error) { onError(message(error, t("requestFailed"))); } finally { setCreating(false); }
  }

  async function copyKey(key: AdminApiKey) {
    if (!key.token) return;
    try { await navigator.clipboard.writeText(key.token); setCopiedKeyId(key.id); window.setTimeout(() => setCopiedKeyId((id) => id === key.id ? null : id), 1_500); } catch { onError(t("requestFailed")); }
  }

  function toggleScope(scope: ApiKeyScope) { setScopes((current) => current.includes(scope) ? current.filter((value) => value !== scope) : [...current, scope]); }

  const revokingKey = revokingKeyId ? items.find((item) => item.id === revokingKeyId) ?? null : null;

  return <>
    <AdminToolbar action={<div className={styles.actions}><Button icon={<AddRegular />} onClick={() => setShowCreate((value) => !value)}>{t("createApiKey")}</Button><Button icon={<ArrowLeftRegular />} disabled={pageNumber <= 1 || loading || revoking || creating} onClick={() => void loadPage(cursorHistory[pageNumber - 2] ?? null, "previous")}>{t("previousPage")}</Button><Button icon={<ArrowRightRegular />} iconPosition="after" disabled={!nextCursor || loading || revoking || creating} onClick={() => void loadPage(nextCursor, "next")}>{t("nextPage")}</Button></div>}><Text size={300}>{formatApiKeyPageStatus(locale, pageNumber, items.length, t)}</Text></AdminToolbar>
    {showCreate && <div className={styles.stack}><div className={styles.formGrid}><Field label={t("apiKeyName")} required><Input value={name} onChange={(event) => setName(event.target.value)} /></Field><Field label={t("apiKeyExpires")} required><Input type="datetime-local" value={expiresAt} onChange={(event) => setExpiresAt(event.target.value)} /></Field></div><Text weight="semibold">{t("apiKeyPermissions")}</Text><div className={styles.formGrid}>{API_KEY_SCOPES.filter(({ scope }) => principal.api_key_scopes.includes(scope)).map(({ scope, label }) => <Checkbox key={scope} checked={scopes.includes(scope)} onChange={() => toggleScope(scope)} label={t(label)} />)}</div><ApiKeyTemplatePicker templates={templates} selected={allowedTemplateIds} restricted={templateRestriction} disabled={creating} translate={t} onRestrictedChange={setTemplateRestriction} onSelectedChange={setAllowedTemplateIds} /><div className={styles.actions}><SaveButton disabled={creating || !name.trim() || scopes.length === 0} onClick={() => void createKey()}>{creating ? t("saving") : t("createApiKey")}</SaveButton></div></div>}
    {loading && items.length === 0 ? <Spinner label={t("loading")} /> : items.length === 0 ? <Text className={styles.empty}>{t("noApiKeys")}</Text> : <DataGrid items={items} columns={apiKeyColumns({ t, locale, styles, revokingKeyId, revoking, loading, copiedKeyId, onCopy: copyKey, setRevokingKeyId, setReason })}><DataGridHeader><DataGridRow<AdminApiKey>>{(column) => <DataGridHeaderCell>{column.renderHeaderCell()}</DataGridHeaderCell>}</DataGridRow></DataGridHeader><DataGridBody<AdminApiKey>>{({ item }) => <DataGridRow<AdminApiKey>>{(column) => <DataGridCell>{column.renderCell(item)}</DataGridCell>}</DataGridRow>}</DataGridBody></DataGrid>}
  <ConfirmDialog
    open={revokingKey !== null}
    title={t("revokeApiKey")}
    description={t("revokeApiKeyConfirm")}
    confirmLabel={t("revokeApiKey")}
    cancelLabel={t("cancel")}
    busy={revoking}
    confirmDisabled={!isCurrentUser && !reason.trim()}
    danger
    details={revokingKey && <div className={styles.stack}><strong>{revokingKey.name}</strong><Text size={200}>{revokingKey.prefix}</Text>{!isCurrentUser && <Field label={t("apiKeyRevocationReason")} required><Textarea value={reason} onChange={(event) => setReason(event.target.value)} placeholder={t("apiKeyRevocationReasonPlaceholder")} /></Field>}</div>}
    onClose={() => { if (!revoking) { setRevokingKeyId(null); setReason(""); } }}
    onConfirm={() => { if (revokingKey) void revoke(revokingKey); }}
  />
  </>;
}

function userColumns({ t, styles, canManageUsers, canEditQuota, canManageApiKeys, openUser, onEditQuota }: { t: ReturnType<typeof useI18n>["t"]; styles: ReturnType<typeof useAdminStyles>; canManageUsers: boolean; canEditQuota: boolean; canManageApiKeys: boolean; openUser: (user: DirectoryItem, section: UserEditorSection) => void; onEditQuota: (userId: string) => void }): TableColumnDefinition<DirectoryItem>[] {
  return [
    { columnId: "user", compare: (a, b) => a.display_name.localeCompare(b.display_name), renderHeaderCell: () => t("user"), renderCell: (item) => <div className={styles.stack}><Text weight="semibold">{item.display_name}</Text><Text size={200}>{item.id}</Text></div> },
    { columnId: "role", compare: (a, b) => String(a.membershipRole).localeCompare(String(b.membershipRole)), renderHeaderCell: () => t("role"), renderCell: (item) => item.membershipRole === "organization_admin" ? t("roleOrganizationAdmin") : item.membershipRole === "member" ? t("roleMember") : t("notEnabled") },
    { columnId: "status", compare: (a, b) => Number(a.disabled) - Number(b.disabled), renderHeaderCell: () => t("workspaceState"), renderCell: (item) => item.disabled ? t("userStatusDisabled") : t("userStatusActive") },
    { columnId: "actions", compare: () => 0, renderHeaderCell: () => t("actions"), renderCell: (item) => <div className={styles.actions}>{(canManageUsers || item.membershipRole !== null) && <Button icon={<EditRegular />} onClick={() => openUser(item, "details")}>{t("editUser")}</Button>}{canManageApiKeys && <Button icon={<KeyRegular />} onClick={() => openUser(item, "apiKeys")}>{t("apiKeys")}</Button>}{canEditQuota && <Button onClick={() => onEditQuota(item.id)}>{t("editUserQuota")}</Button>}</div> },
  ];
}

function apiKeyColumns({ t, locale, styles, revokingKeyId, revoking, loading, copiedKeyId, onCopy, setRevokingKeyId, setReason }: { t: ReturnType<typeof useI18n>["t"]; locale: string; styles: ReturnType<typeof useAdminStyles>; revokingKeyId: string | null; revoking: boolean; loading: boolean; copiedKeyId: string | null; onCopy: (key: AdminApiKey) => void; setRevokingKeyId: (id: string | null) => void; setReason: (reason: string) => void }): TableColumnDefinition<AdminApiKey>[] {
  return [
    { columnId: "key", compare: (a, b) => a.name.localeCompare(b.name), renderHeaderCell: () => t("manageUserApiKeys"), renderCell: (item) => <div className={styles.stack}><Text weight="semibold">{item.name}</Text><Text size={200}>{item.prefix}</Text><Text size={200}>{formatApiKeyScopes(item, t)}</Text></div> },
    { columnId: "status", compare: (a, b) => getApiKeyStatus(a).localeCompare(getApiKeyStatus(b)), renderHeaderCell: () => t("apiKeyStatus"), renderCell: (item) => formatApiKeyStatus(item, t) },
    { columnId: "expires", compare: (a, b) => (a.expires_at ?? 0) - (b.expires_at ?? 0), renderHeaderCell: () => t("apiKeyExpires"), renderCell: (item) => formatApiKeyExpiry(item.expires_at, locale, t) },
    { columnId: "actions", compare: () => 0, renderHeaderCell: () => t("actions"), renderCell: (item) => <div className={styles.actions}>{item.token ? <Button appearance="subtle" icon={<CopyRegular />} onClick={() => onCopy(item)}>{copiedKeyId === item.id ? t("copied") : t("copy")}</Button> : <Tooltip content={t("apiKeyCopyUnavailable")} relationship="description"><span><Button appearance="subtle" icon={<CopyRegular />} disabled>{t("copy")}</Button></span></Tooltip>}{getApiKeyStatus(item) === "active" && <Button disabled={revoking || loading || revokingKeyId !== null} onClick={() => { setRevokingKeyId(item.id); setReason(""); }}>{t("revokeApiKey")}</Button>}</div> },
  ];
}

function message(error: unknown, fallback: string) { return error instanceof Error ? error.message : fallback; }

function defaultExpiry(): string {
  const value = new Date(Date.now() + 30 * 86_400_000);
  const offset = value.getTimezoneOffset() * 60_000;
  return new Date(value.getTime() - offset).toISOString().slice(0, 16);
}
