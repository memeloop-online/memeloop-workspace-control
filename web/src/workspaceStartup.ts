import type { MessageKey } from "./i18n";
import type { WorkspaceSshConnection } from "./types";

type HostKeySource = Pick<WorkspaceSshConnection, "alias" | "host_key_alias" | "known_hosts_entry">;

/**
 * Returns the known_hosts line for the workspace host key.
 * Prefers the backend-provided entry; otherwise composes one from the host
 * key alias, which always matches connection.alias and the config HostKeyAlias.
 */
export function knownHostsEntry(connection: HostKeySource, publicKey: string | null): string | null {
  if (connection.known_hosts_entry) return connection.known_hosts_entry;
  if (!publicKey) return null;
  return `${connection.host_key_alias ?? connection.alias} ${publicKey}`;
}

/** Checks that the host key alias appears as the HostKeyAlias line in the SSH config. */
export function hostKeyAliasMatchesConfig(alias: string, config: string): boolean {
  return config.split("\n").some((line) => line.trim() === `HostKeyAlias ${alias}`);
}

export function formatDurationSeconds(value: number | null): string {
  if (value === null || !Number.isFinite(value) || value < 0) return "—";
  if (value < 1) return "<1 s";
  const total = Math.round(value);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `${minutes} m ${seconds} s` : `${seconds} s`;
}

const startupStageLabels: Record<string, MessageKey> = {
  PodScheduled: "startupStagePodScheduled",
  PodReadyToStartContainers: "startupStagePodReadyToStartContainers",
  Initialized: "startupStageInitialized",
  Ready: "startupStageReady",
};

/** Maps a startup stage name to a friendly label key, or null to show the raw name. */
export function startupStageLabelKey(name: string): MessageKey | null {
  return startupStageLabels[name] ?? null;
}

const containerNameLabels: Record<string, MessageKey> = {
  "workspace-scratch-init": "containerNameWorkspaceScratchInit",
  "workspace-bootstrap": "containerNameWorkspaceBootstrap",
  "buildkit-bootstrap": "containerNameBuildkitBootstrap",
};

/** Maps a well-known init container name to a friendly label key, or null to show the raw name. */
export function containerNameLabelKey(name: string): MessageKey | null {
  return containerNameLabels[name] ?? null;
}

const containerStateLabels: Record<string, MessageKey> = {
  running: "containerStateRunning",
  waiting: "containerStateWaiting",
  terminated: "containerStateTerminated",
};

/** Maps a container state to a friendly label key, or null to show the raw state. */
export function containerStateLabelKey(state: string): MessageKey | null {
  return containerStateLabels[state.toLowerCase()] ?? null;
}
