import { useEffect, useRef, useState } from "react";
import type { ApiClient } from "./api";
import { useI18n } from "./i18n";
import type { MembershipSummary, Principal, Role, UserSummary } from "./types";

type DirectoryItem = UserSummary & { membershipRole: Role | null };

export interface UsersDirectoryProps {
  api: ApiClient;
  organizationId: string;
  principal: Principal;
  canManageUsers: boolean;
  refreshVersion: number;
  onError: (message: string) => void;
  onEditQuota: (userId: string) => void;
}

export function UsersDirectory({ api, organizationId, principal, canManageUsers, refreshVersion, onError, onEditQuota }: UsersDirectoryProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [size, setSize] = useState(50);
  const [items, setItems] = useState<DirectoryItem[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const loadingRef = useRef(false);
  const requestRef = useRef(0);

  useEffect(() => {
    let active = true;
    const requestId = ++requestRef.current;
    const search = query.trim() || undefined;
    // Prevent a role from the old organization/query from briefly offering an action.
    setItems([]);
    setCursor(null);
    loadingRef.current = true;
    setLoading(true);
    const timer = window.setTimeout(() => {
      void loadInitial(api, organizationId, size, search, canManageUsers)
        .then((page) => {
          if (!active || requestRef.current !== requestId) return;
          setItems(page.items);
          setCursor(page.nextCursor);
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

  async function loadMore() {
    if (!cursor || loadingRef.current) return;
    const requestId = requestRef.current;
    loadingRef.current = true;
    setLoading(true);
    try {
      const search = query.trim() || undefined;
      if (canManageUsers) {
        const page = await api.usersPage({ limit: size, cursor, search });
        if (requestRef.current !== requestId) return;
        setItems((current) => [...current, ...page.items.map((user) => ({ ...user, membershipRole: null }))]);
        setCursor(page.next_cursor);
        const memberships = await allMemberships(api, organizationId, search);
        if (requestRef.current === requestId) applyMemberships(memberships);
      } else {
        const page = await api.membersPage(organizationId, { limit: size, cursor, search });
        if (requestRef.current !== requestId) return;
        setItems((current) => [...current, ...page.items.map(memberToItem)]);
        setCursor(page.next_cursor);
      }
    } catch (error) {
      if (requestRef.current === requestId) onError(message(error, t("requestFailed")));
    } finally {
      if (requestRef.current === requestId) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }

  function applyMemberships(memberships: MembershipSummary[]) {
    const roles = new Map(memberships.map((membership) => [membership.user.id, membership.role]));
    setItems((current) => current.map((user) => ({ ...user, membershipRole: roles.get(user.id) ?? null })));
  }

  function updateUser(updated: UserSummary) {
    setItems((current) => current.map((user) => user.id === updated.id ? { ...updated, membershipRole: user.membershipRole } : user));
  }

  function updateMembership(userId: string, membershipRole: Role | null) {
    setItems((current) => current.map((user) => user.id === userId ? { ...user, membershipRole } : user));
  }

  return <div>
    <div className="form-actions"><label>{t("searchUsers")}<input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("searchUsersPlaceholder")} /></label><label>{t("rowsPerPage")}<select value={size} onChange={(event) => setSize(Number(event.target.value))}><option value="25">25</option><option value="50">50</option><option value="100">100</option></select></label></div>
    <div className="user-directory-list" style={{ display: "grid", gap: "8px", maxHeight: "560px", overflowY: "auto", marginTop: "12px" }} onScroll={(event) => { const element = event.currentTarget; if (element.scrollTop + element.clientHeight >= element.scrollHeight - 96) void loadMore(); }}>
      {items.map((user) => <UserDirectoryRow key={user.id} user={user} api={api} organizationId={organizationId} principal={principal} canManageUsers={canManageUsers} onError={onError} onUpdated={updateUser} onMembershipChanged={updateMembership} onEditQuota={onEditQuota} />)}
      {items.length === 0 && loading && <p role="status">{t("loading")}</p>}
      {items.length === 0 && !loading && <p>{t("noUsers")}</p>}
    </div>
    {cursor && <small role="status">{loading ? t("loading") : t("scrollForMore")}</small>}
  </div>;
}

async function loadInitial(api: ApiClient, organizationId: string, size: number, search: string | undefined, canManageUsers: boolean): Promise<{ items: DirectoryItem[]; nextCursor: string | null }> {
  if (!canManageUsers) {
    const page = await api.membersPage(organizationId, { limit: size, search });
    return { items: page.items.map(memberToItem), nextCursor: page.next_cursor };
  }
  const [users, memberships] = await Promise.all([
    api.usersPage({ limit: size, search }),
    allMemberships(api, organizationId, search),
  ]);
  const roles = new Map(memberships.map((membership) => [membership.user.id, membership.role]));
  return {
    items: users.items.map((user) => ({ ...user, membershipRole: roles.get(user.id) ?? null })),
    nextCursor: users.next_cursor,
  };
}

async function allMemberships(api: ApiClient, organizationId: string, search: string | undefined): Promise<MembershipSummary[]> {
  const members: MembershipSummary[] = [];
  let cursor: string | undefined;
  do {
    const page = await api.membersPage(organizationId, { limit: 200, cursor, search });
    members.push(...page.items);
    cursor = page.next_cursor ?? undefined;
  } while (cursor);
  return members;
}

function memberToItem(membership: MembershipSummary): DirectoryItem {
  return { ...membership.user, membershipRole: membership.role };
}

function UserDirectoryRow({ user, api, organizationId, principal, canManageUsers, onError, onUpdated, onMembershipChanged, onEditQuota }: { user: DirectoryItem; api: ApiClient; organizationId: string; principal: Principal; canManageUsers: boolean; onError: (message: string) => void; onUpdated: (user: UserSummary) => void; onMembershipChanged: (userId: string, role: Role | null) => void; onEditQuota: (userId: string) => void }) {
  const { t } = useI18n();
  const [displayName, setDisplayName] = useState(user.display_name);
  const [systemAdmin, setSystemAdmin] = useState(user.system_admin);
  const [disabled, setDisabled] = useState(user.disabled);
  const [role, setRole] = useState<Role>(user.membershipRole ?? "member");
  const [saving, setSaving] = useState(false);
  const [membershipMessage, setMembershipMessage] = useState("");
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

  return <article style={{ display: "grid", gap: "9px", padding: "12px", border: "1px solid var(--line)", borderRadius: "9px", background: "var(--surface-2)" }}>
    <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: "10px" }}><strong>{user.display_name}</strong><span>{user.disabled ? t("userStatusDisabled") : t("userStatusActive")}</span></div>
    {canManageUsers && <>
      <code>{user.id}</code>
      <label>{t("displayName")}<input value={displayName} onChange={(event) => setDisplayName(event.target.value)} /></label>
      <div className="check-row"><label><input type="checkbox" checked={systemAdmin} disabled={isCurrentUser} onChange={(event) => setSystemAdmin(event.target.checked)} />{t("systemAdmin")}</label><label><input type="checkbox" checked={disabled} disabled={isCurrentUser} onChange={(event) => setDisabled(event.target.checked)} />{disabled ? t("enableUser") : t("disableUser")}</label></div>
      <div className="form-actions" style={{ flexWrap: "wrap", gap: "8px" }}><button className="button primary" disabled={saving || !displayName.trim()} onClick={() => void save()}>{saving ? t("saving") : t("saveUser")}</button><button className="button" disabled={saving} onClick={() => onEditQuota(user.id)}>{t("editUserQuota")}</button></div>
    </>}
    <div className="form-actions" style={{ flexWrap: "wrap", gap: "8px" }}><label>{t("role")}<select value={role} disabled={saving} onChange={(event) => setRole(event.target.value as Role)}><option value="member">{t("roleMember")}</option><option value="organization_admin">{t("roleOrganizationAdmin")}</option></select></label><button className="button" disabled={saving} onClick={() => void saveMembership()}>{user.membershipRole ? t("saveMembership") : t("addOrganizationMember")}</button>{user.membershipRole && <button className="button danger" disabled={saving} onClick={() => void removeMember()}>{t("removeOrganizationMember")}</button>}</div>
    {membershipMessage && <small role="status">{membershipMessage}</small>}
  </article>;
}

function message(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}
