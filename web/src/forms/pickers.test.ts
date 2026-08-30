import assert from "node:assert/strict";
import test from "node:test";

import { filterReferenceItems, filterWorkspaces } from "./pickerModel.ts";
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
