import assert from "node:assert/strict";
import test from "node:test";

import { filterReferenceItems, filterWorkspaces, mergeWorkspaces } from "./pickerModel.ts";
import type { StoredInjection, WorkspaceResponse } from "../types.ts";

const translate = ((key: string) => ({ environmentVariable: "环境变量", credentialFile: "敏感文件", sshPublicKey: "SSH 公钥", configFile: "配置文件" })[key] ?? key) as never;

test("credential reference search matches localized type, Unicode name, and target", () => {
  const items = [
    { key: "游戏电脑公钥", target: "/workspace/key.pub", kind: "ssh_public_key" },
    { key: "registry", target: "REGISTRY_TOKEN", kind: "environment_variable" },
  ] as StoredInjection[];
  assert.deepEqual(filterReferenceItems(items, "公钥", translate), [items[0]]);
  assert.deepEqual(filterReferenceItems(items, "registry_token", translate), [items[1]]);
});

test("workspace combobox filters by name, short ID, and login user", () => {
  const items = [
    { workspace: { name: "游戏开发", short_id: "01game", workspace_user: "rust-dev" } },
    { workspace: { name: "Docs", short_id: "01docs", workspace_user: "node-dev" } },
  ] as WorkspaceResponse[];
  assert.deepEqual(filterWorkspaces(items, "游戏"), [items[0]]);
  assert.deepEqual(filterWorkspaces(items, "01docs"), [items[1]]);
  assert.deepEqual(filterWorkspaces(items, "rust-dev"), [items[0]]);
});

test("workspace combobox merges local and remote results without duplicate options", () => {
  const local = { workspace: { id: "one", name: "Local one", short_id: "01one", workspace_user: "dev" } } as WorkspaceResponse;
  const remoteDuplicate = { workspace: { id: "one", name: "Remote one", short_id: "01one", workspace_user: "dev" } } as WorkspaceResponse;
  const remoteOnly = { workspace: { id: "two", name: "Remote two", short_id: "02two", workspace_user: "ops" } } as WorkspaceResponse;
  assert.deepEqual(mergeWorkspaces([local], [remoteDuplicate, remoteOnly]), [local, remoteOnly]);
});

test("workspace combobox filtering keeps the displayed selected label searchable", () => {
  const item = { workspace: { name: "Docs", short_id: "01docs", workspace_user: "node-dev" } } as WorkspaceResponse;
  assert.deepEqual(filterWorkspaces([item], "Docs · 01docs"), [item]);
});
