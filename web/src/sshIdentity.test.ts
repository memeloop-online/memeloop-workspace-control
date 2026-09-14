import assert from "node:assert/strict";
import test from "node:test";
import { jumpHostKnownHostsEntry, workspaceKnownHostsEntry } from "./sshIdentity.ts";

const connection = {
  display_name: "node-dev",
  alias: "mwc-bd2dc9ca6aa2b1b5",
  hostname: "workspace.example.test",
  port: 30693,
  user: "workspace",
  command: "ssh -p 30693 workspace@workspace.example.test",
  config: "Host mwc-bd2dc9ca6aa2b1b5\n  HostName workspace.example.test\n  Port 30693\n  User workspace\n  HostKeyAlias workspace-bd2dc9ca6aa2b1b5\n",
  app: { display_name: "node-dev", hostname: "mwc-bd2dc9ca6aa2b1b5", ssh_port: null, port_strategy: "ssh_config" as const },
};

const identity = {
  algorithm: "ssh-ed25519",
  public_key: "ssh-ed25519 AAAATEST workspace-host",
  fingerprint: "SHA256:test",
};

test("workspace known_hosts entry uses the configured HostKeyAlias", () => {
  assert.equal(workspaceKnownHostsEntry(connection, identity), "workspace-bd2dc9ca6aa2b1b5 ssh-ed25519 AAAATEST workspace-host");
});

test("jump host entry is derived only when ProxyJump is configured", () => {
  assert.equal(jumpHostKnownHostsEntry(connection, identity), null);
  assert.equal(
    jumpHostKnownHostsEntry({ ...connection, config: `${connection.config}  ProxyJump access+bd2d@ssh.example.test\n` }, identity),
    "ssh.example.test ssh-ed25519 AAAATEST workspace-host",
  );
});
