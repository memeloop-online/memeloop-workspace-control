# Final Coder TOKEN center dev migration runbook

This is an external-agent handoff, not authorization to change a cluster. Perform it outside the
Coder source workspace. Do not stop or migrate that workspace from inside itself;
read-only inspection does not authorize its shutdown.

## Scope and completed-history boundary

The only remaining migration is the independent **Coder TOKEN center dev** durable home:

| Source namespace | Source PVC | Source PV | Capacity / reclaim policy |
| --- | --- | --- | --- |
| `coder` | `coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home` | `pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0` | 100Gi / `Retain` |

It is not the former MWC `rust-dev-test` volume. Target one normally-created workspace in
`memeloop-workspace-control`; do not invent an import API or edit the product database.

Do **not** repeat the completed platform migration: control and four MWC workspaces are already
in exactly `memeloop-workspace-control`, production schema is 22, the five old namespaces are
deleted, and `ttydOpenSSL` image `4cf08fd5` passed SSH and real-browser terminal checks for all
four workspaces. Do not stop those workspaces, recreate old namespaces, run bridge/offline schema
procedures, or reuse historical borrow-port/bridge operations. Evidence is retained in the
[archived MWC migration record](archive/2026-09-09-mwc-platform-migration.md).

## Required preconditions

1. Use an approved change window and external operator identity. If the current Pod → PVC → PV →
   claim-UID chain differs from the table, or another agent is active, stop and coordinate.
2. Read-only capture source Pod/PVC/PV, claim UID/resourceVersion, Longhorn Volume,
   VolumeAttachment, snapshot, node, manifest digest, real home paths, numeric UID/GID and
   ownership. Never record tokens, private keys, credentials, or database contents.
3. Require a healthy source Longhorn volume. Before freeze create and verify a fresh recoverable
   snapshot. Retain it, the source PV and the empty target PV through rollback.
4. Select an authorized Rust template compatible with the real user, UID/GID, home mount and
   toolchain. Inspect actual `/home/token-center-dev`, `/home/rust-dev`, symlinks and ownership;
   do not infer them from names or recursively `chown` the durable home.

## Cutover procedure

1. Create one normal managed target with the authorized organization, owner and template plus an
   idempotency key. Record workspace/PVC/PV identity and labels; wait for its empty PVC/PV to bind.
2. Stop the target through its normal API. Require API `Stopped`, zero replicas, no target Pod and
   detached target volume. Do not leave the target PVC absent while a claim template can recreate it.
3. Preserve read-only connection fields and SSH public-key/host-key fingerprints. At final freeze,
   stop every source Coder writer/controller externally. Require no source Pod or attachment.
4. Change **both** source and empty target PVs to `Retain`, and read both policies back. The empty
   target PV is rollback data. With UID/resourceVersion preconditions, delete only the exact
   stopped source and target PVCs. Never delete either PV or Longhorn volume.
5. Once the source PV is `Released`, clear only its verified old `claimRef`. Create replacement
   target PVC with the original deterministic target name/ownership labels, compatible
   storage/access/volume-mode/capacity, and `spec.volumeName` equal to source PV. Wait for Bound;
   verify both directions and the replacement PVC's **new UID**. Do not start before this check.
6. Re-read health, attachment and PVC/PV chain, then start through normal API. Require one writer.
   Keep retained PVs/snapshots through rollback; do not pause or change the four accepted MWC workspaces.

## Durable data, desktop access, acceptance

Direct PV rebind is preferred and does not need a directory copy. Preserve all Codex durable state:
`.codex/sessions`, logs, SQLite/WAL/SHM, auth/configuration and repositories. Do not copy `tmp` or
build-cache. Never use a broad cleanup command; any fallback-copy plan needs separate checksum and
downtime approval, and may omit only confirmed regenerable inactive scratch/cache paths.

Validate real home/ownership, repositories/Codex data, CLI/toolchain, credential injection,
resource configuration, healthy volume, SSH, browser terminal, host-key continuity and
single-writer state. Verify a changed host key via trusted channel; never disable
`StrictHostKeyChecking`.

Desktop configuration can be migrated only with actual desktop access: back up real SSH/remote
settings, update only the old Coder connection's host/port/user/needed paths, and perform a real
reconnect. Without access, supply exact new fields and user steps; do not claim desktop settings or
a resident process migrated seamlessly. Recoverable persistent sessions do not mean an old process
survives.

## Rollback and cleanup

Before acceptance freeze/detach target writer and restore recorded old claim binding or verified
snapshot. Never delete target volume to ease rollback. Report downtime, validation, rollback,
retained resources and scoped cleanup. Only after source has no writer, target passes acceptance
and rollback window ends may external owner remove old Coder controller/configuration/namespace.
Never bulk-delete other workspaces.

## Copy/paste assignment for external agent

```text
只迁移最后一个 Coder TOKEN center dev，必须从源工作区之外执行。
唯一源：coder/coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home；
PV pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0；100Gi、Retain。先复核实时 Pod→PVC→PV→
claim UID/resourceVersion；有变化或其他代理操作就停止协调。这不是 rust-dev-test。

不要重做四个 MWC/control：它们已在 memeloop-workspace-control，schema 22，旧五 namespace
已删；ttydOpenSSL 4cf08fd5 的 SSH/browser 验收已通过。不要停止它们，不跑 bridge/offline schema、
旧 borrow-port 或 namespace 操作。

用兼容真实 UID/GID、home 挂载、Rust 工具链的正常 API 创建目标；空 PVC Bound 后 API 停止，确认无 Pod/
附件。外部冻结源全部写入者/控制器，完成且验证 fresh Longhorn snapshot。源与目标空 PV 先改 Retain 并读回；
有 UID/resourceVersion 前置条件时才删两个准确停止 PVC，绝不删 PV/Longhorn volume。源 PV Released 后才清
已验证 claimRef；以原目标 PVC 名、身份标签、volumeName=源 PV 创建替换 PVC，双向核验 Bound/new UID 后才启动。

直接 PV rebind 优先，不复制目录。保留 .codex/sessions、日志、SQLite/WAL/SHM、认证/配置和仓库；不复制
tmp/buildcache，不用覆盖整个 home 的清理命令。验收 home/UID、数据、CLI、凭据注入、SSH、网页终端、host-key、
单写入者和卷健康。host-key 必须可信通道核验，不能关闭 StrictHostKeyChecking。

只有实际 desktop access 才能备份/修改旧 Coder 的 SSH/远程连接配置并真实重连；否则给出精确新字段和用户步骤，
不能声称桌面配置或驻留 process 无缝迁移。保留 PV/snapshot 至回滚窗口结束；报告停机、验收、回滚、保留资源和
精确清理清单。不要输出 token、私钥、凭据或数据库内容。
```
