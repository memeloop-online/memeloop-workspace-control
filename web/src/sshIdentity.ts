import type { WorkspaceResponse, WorkspaceSshConnection } from "./types";

type PublicIdentity = NonNullable<WorkspaceResponse["workspace_host_key"]>;

export function workspaceKnownHostsEntry(connection: WorkspaceSshConnection, identity: PublicIdentity): string {
  return `workspace-${connection.alias.replace(/^mwc-/, "")} ${identity.public_key}`;
}

export function jumpHostKnownHostsEntry(connection: WorkspaceSshConnection, identity: PublicIdentity): string | null {
  const match = connection.config.match(/^\s*ProxyJump\s+(?:[^@\s]+@)?([^\s]+)\s*$/m);
  return match ? `${match[1]} ${identity.public_key}` : null;
}
