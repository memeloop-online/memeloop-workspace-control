import assert from "node:assert/strict";
import test from "node:test";

import {
  containerNameLabelKey,
  containerStateLabelKey,
  formatDurationSeconds,
  hostKeyAliasMatchesConfig,
  knownHostsEntry,
  startupStageLabelKey,
} from "./workspaceStartup.ts";

const connection = {
  alias: "mwc-demo-a1b2c3d4",
  host_key_alias: "mwc-demo-a1b2c3d4",
  known_hosts_entry: "mwc-demo-a1b2c3d4 ssh-ed25519 AAAAC3NzaC demo",
};

const config = [
  "Host mwc-demo-a1b2c3d4",
  "  HostName 203.0.113.10",
  "  Port 32222",
  "  User coder",
  "  HostKeyAlias mwc-demo-a1b2c3d4",
].join("\n");

test("uses the backend-provided known_hosts entry and it matches the alias", () => {
  const entry = knownHostsEntry(connection, "ssh-ed25519 IGNORED");
  assert.equal(entry, connection.known_hosts_entry);
  assert.ok(entry!.startsWith(`${connection.alias} `));
});

test("composes the known_hosts entry from the host key alias as fallback", () => {
  const aliasOnly = { alias: "mwc-demo-a1b2c3d4", host_key_alias: undefined, known_hosts_entry: undefined };
  assert.equal(knownHostsEntry(aliasOnly, "ssh-ed25519 KEY"), "mwc-demo-a1b2c3d4 ssh-ed25519 KEY");
  assert.equal(knownHostsEntry({ ...aliasOnly, host_key_alias: "mwc-demo-a1b2c3d4" }, "ssh-ed25519 KEY"), "mwc-demo-a1b2c3d4 ssh-ed25519 KEY");
  assert.equal(knownHostsEntry(aliasOnly, null), null);
});

test("host key alias matches the HostKeyAlias line in the SSH config", () => {
  assert.ok(hostKeyAliasMatchesConfig(connection.alias, config));
  assert.ok(!hostKeyAliasMatchesConfig("workspace-a1b2c3d4", config));
  assert.ok(!hostKeyAliasMatchesConfig("mwc-other", config));
});

test("formats startup durations for the timeline", () => {
  assert.equal(formatDurationSeconds(null), "—");
  assert.equal(formatDurationSeconds(0.4), "<1 s");
  assert.equal(formatDurationSeconds(12), "12 s");
  assert.equal(formatDurationSeconds(83), "1 m 23 s");
  assert.equal(formatDurationSeconds(600), "10 m 0 s");
  assert.equal(formatDurationSeconds(Number.NaN), "—");
});

test("maps known stage, container, and state names to labels", () => {
  assert.equal(startupStageLabelKey("PodScheduled"), "startupStagePodScheduled");
  assert.equal(startupStageLabelKey("PodReadyToStartContainers"), "startupStagePodReadyToStartContainers");
  assert.equal(startupStageLabelKey("Initialized"), "startupStageInitialized");
  assert.equal(startupStageLabelKey("Ready"), "startupStageReady");
  assert.equal(startupStageLabelKey("CustomStage"), null);
  assert.equal(containerNameLabelKey("workspace-scratch-init"), "containerNameWorkspaceScratchInit");
  assert.equal(containerNameLabelKey("workspace-bootstrap"), "containerNameWorkspaceBootstrap");
  assert.equal(containerNameLabelKey("buildkit-bootstrap"), "containerNameBuildkitBootstrap");
  assert.equal(containerNameLabelKey("workspace"), null);
  assert.equal(containerStateLabelKey("running"), "containerStateRunning");
  assert.equal(containerStateLabelKey("Waiting"), "containerStateWaiting");
  assert.equal(containerStateLabelKey("terminated"), "containerStateTerminated");
  assert.equal(containerStateLabelKey("paused"), null);
});
