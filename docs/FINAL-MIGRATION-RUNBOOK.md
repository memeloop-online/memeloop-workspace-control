# Final unified-namespace migration runbook

This is an executable handoff plan, not authorization to change a cluster. Its target is the exact
namespace `memeloop-workspace-control`, with the existing installation ID
`k3si-7032544955` retained and every workspace resource name qualified by its workspace ID. The
final layout has five workspaces: four MWC workspaces plus the independent Coder TOKEN center dev
workspace. Do not start the latter cutover from itself: an external agent must perform it because
its source claim is 100 GiB and is in scope for preservation.

This runbook deliberately does not contain a namespace-deletion command or a destructive
"one-shot" script. No old namespace may be cleaned up until every acceptance and rollback-window
gate below has been signed off.

## Recorded read-only baseline (2026-09-08)

These observations are evidence, not a replacement for an immediately-pre-cutover capture using
`scripts/final-migration-readiness.sh`.

| Logical workload | Current namespace / PVC | PV / storage | Required disposition |
| --- | --- | --- | --- |
| Control plane SQLite | `mwc-k3si-7032544955` / `data-mwc-k3si-7032544955-0` (10Gi) | `pvc-317ea51a-86f8-4f1c-b2ed-2c111795c331`, `local-path`, `Delete` | Reviewed retained local-path PV rebind after offline migration and file backup; not a Longhorn volume. |
| maintainance | `ws-k3si-7032544955-bd2dc9ca6aa2b1b5` / `workspace-data-w-bd2dc9ca6aa2b1b5-0` (2Gi) | `pvc-611ec96e-4bd9-4162-b237-dab2b68d3694`, Longhorn, `Delete` | Direct PV rebind only after gates. |
| tiddlywiki-dev | `ws-k3si-7032544955-b268ff46894a14b9` / `workspace-data-w-b268ff46894a14b9-0` (30Gi) | `pvc-ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96`, Longhorn, `Delete` | Direct PV rebind only after gates. |
| game-forking | `ws-k3si-7032544955-97645a0fb4771b1d` / `workspace-data-w-97645a0fb4771b1d-0` (60Gi) | `pvc-bf5e159e-bf80-47ae-bda3-58affce88008`, Longhorn, `Delete` | **Blocked:** volume was `attached/degraded`; restore healthy state and verify snapshots first. |
| rust-dev-test (MWC) | `ws-k3si-7032544955-b405d2441a1ee149` / `workspace-data-w-b405d2441a1ee149-0` (100Gi) | `pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e`, Longhorn, `Delete` | Independent MWC workspace; port 32671 is borrowed, so do not stop it until its borrower releases it. |
| Coder TOKEN center dev (independent) | `coder` / `coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home` (100Gi) | `pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0`, Longhorn, `Retain` | External-agent cutover only. The observed Pod, PVC and PV claim UID all match; it is not `rust-dev-test`. |

All Longhorn entries must be `healthy`, attached only where expected, and have no outstanding
replica rebuild/fault condition at the freeze point. The target namespace being absent is expected
before cutover, not a blocker. `Delete` is unsafe during rebinding: set each selected PV reclaim
policy to `Retain` and verify it before its old PVC is removed. Record all Pod-to-PVC-to-PV,
VolumeAttachment, Longhorn Volume, snapshot, node, claim UID and manifest digests in the external
change record; do not record secret values or database contents.

## Mandatory order and gates

1. Freeze automation first. Pause Argo CD auto-sync/self-heal for both the MWC and routing
   Applications and any controller that can recreate old-namespace StatefulSets/PVCs. Merely
   scaling a workload is insufficient while its reconciliation controller remains active.
2. Resolve the MWC baseline blocker, perform a fresh read-only evidence capture, and create
   verified Longhorn snapshots for the four MWC volumes. Confirm snapshot completion and a tested
   restore path. The independent Coder volume remains live and is explicitly out of this first
   phase. The existing pre-start snapshots are retained but are not proof of a fresh cutover
   snapshot.
3. Promote schema **19 -> 20 only** using the published 19-to-20 bridge release. Read the
   successful CI provenance/artifact metadata and copy the exact control-plane, ttyd and other
   required immutable digests from it. This document intentionally does not invent unpublished
   digests. Verify the migration transaction and all workspace reconciliation records before the
   next release.
4. After schema-20 acceptance, begin the final offline window: pause old GitOps/reconcilers and
   stop the old control plane. Use the current, verified-and-published schema-22 release to run
   the offline **20 -> 22** migration; it includes both required transformations, so no
   schema-21 coordinator/process is started. Do not jump 19 directly to 22. This SQLite cutover
   reuses the retained PV; JSON snapshot import is only supported for PostgreSQL destinations.
   If the schema-22 CI/provenance or publication gate is not green, cutover is
   prohibited. Schema compatibility is a hard gate, not a cosmetic version bump.
5. Keep MWC writes frozen and stop the four MWC workspaces from an external operator context.
   The independent 100Gi Coder workspace has no MWC database writer relationship: leave it
   running and do not migrate or stop it in this phase. Confirm the four MWC Pods are gone,
   their StatefulSets/controllers cannot recreate them, and no
   VolumeAttachment remains for a volume about to move, and Longhorn reports detached before a PV
   claim is moved. Preserve all of `.codex` except explicit pod-lifetime scratch/cache directories:
   `.codex/sessions`, logs, SQLite/WAL, auth/configuration and repositories are durable data.
6. Rebind the offline-migrated control-plane volume into the target namespace and prepare the
   four MWC workload manifests, then move those four MWC PV bindings one at a time. Start the target control
   plane only after its database, namespace-bearing records and target resource plan are ready;
   start a single MWC writer only after its binding and integrity checks pass. Do not simultaneously
   mount old and target claims.
7. Validate the four MWC workspaces (PVC/PV identity, SSH host-key continuity, authenticated SSH
   command, Web Shell, retained Codex data, routes/NodePorts, and reconciler health). Re-enable the
   target MWC GitOps/reconciler only after it has target-only desired state. Keep the old MWC
   resources intact throughout their rollback window.
8. This completes the MWC platform phase. The independent Coder migration is a later external-only
   single-workspace change; it must not interrupt the healthy four MWC workspaces. The final global
   cleanup gate applies only after that second phase is accepted.

## PV direct-rebind procedure (one workspace at a time)

This procedure applies to the four MWC Longhorn PVs in the first phase. The Coder PV is excluded
and is handled only by its external controller owner in the later second phase. It is a human
runbook so each irreversible action receives a separate review.

1. Capture each MWC Pod -> PVC -> PV -> Longhorn Volume chain and Longhorn volume state. Require
   the old PVC UID to match `PV.spec.claimRef.uid`; compare only values from the same chain. The
   independent Coder 100Gi chain is not part of this operation and must never be cross-compared.
2. Stop/pause the old StatefulSet and its reconciler. Wait for no old Pod, no active attachment,
   no unexpected Longhorn workload status, and a completed fresh snapshot. Verify no process can
   write the old claim.
3. Patch the selected PV reclaim policy to `Retain`; read it back. Delete only the selected old
   PVC after the retained-policy readback. Never delete the PV or Longhorn Volume.
4. Clear the PV `claimRef` under an approved, audited change, then create the target-namespace
   PVC with the exact target name, `spec.volumeName` set to that PV, compatible storage class,
   access mode, volume mode, capacity, and selector semantics. Wait for a Bound claim and confirm
   that the PV claimRef namespace/name/**new UID** match it.
5. Verify Longhorn reports precisely the intended PV and, before starting the StatefulSet/controller, that it
   is detached and healthy. Start exactly one target workload; confirm its attachment node and
   writer. Keep `Retain` throughout acceptance and the rollback window. Only after final
   acceptance, restore the reclaim policy required by the selected StorageClass (normally `Delete`
   for these four MWC `mwc-longhorn-large-delete` claims).

Never use a copy job as a substitute for this process unless a direct rebind is rejected by a
storage owner. A copy path requires an independent checksum/metadata acceptance plan and a
separate downtime estimate.

## Control-plane SQLite handling

The control-plane PV is `local-path`, not Longhorn. The product's `database import` command
targets PostgreSQL only; it cannot restore a JSON export into SQLite. An export also omits
idempotency replay records, so preserve an offline SQLite/WAL/SHM backup in addition to the
mode-0600 export. Never print either backup's contents.

The 2026-09-09 review approved direct reuse under the user's migration authorization:
PV `pvc-317ea51a-86f8-4f1c-b2ed-2c111795c331`, source claim
`mwc-k3si-7032544955/data-mwc-k3si-7032544955-0`, source UID
`317ea51a-86f8-4f1c-b2ed-2c111795c331`, 10Gi/RWO/Filesystem/local-path.
Its existing directory and node affinity are on `westlake`; preserve both. The physical
directory's source-namespace text is not a reason to rename or move stored database files.

After the bridge passes, stop the sole control-plane writer and run the supported schema-22
`database migrate` command offline against this volume. Verify schema 22 before rebinding.
Then, with no writer or helper Pod mounting it, guard the exact source claim UID and current PV
resourceVersion, set reclaim policy to `Retain`, and read it back. Delete only the source PVC;
wait for PVC absence and PV `Released`, then explicitly clear the guarded old claimRef.
Create the same-named target claim in `memeloop-workspace-control`, prebound with
`spec.volumeName` to the retained PV. Verify Bound and the new claim UID in both directions.

Only then start the target control plane with `sqlite.existingClaim` set to this precreated
claim. Do not use automatic claim templates, and preserve scheduling compatible with the PV's
`westlake` node affinity. Preserve the encryption/auth Secrets and installation ID.
Retain the PV, exports, and frozen file backup through acceptance. A schema rollback requires
restoring the matching offline database backup; rebinding alone does not undo schema changes.

## Second phase: external Coder single-workspace handoff

After the MWC platform phase has passed, keep its four target workspaces running normally. Only an
external agent may schedule the Coder change. It must independently snapshot, stop the Coder source
at the final writer freeze, and validate its own Pod -> PVC -> PV -> Longhorn chain.

The current product exposes normal managed creation through `POST /api/v1/workspaces` and lifecycle
actions through `POST /api/v1/workspaces/{workspace_id}/actions/{action}`. Use the following
standard, single-workspace procedure; it does not require a special Coder-import API or a database
edit.

1. Before creating the target, read-only verify the Coder source's actual home path, numeric
   UID/GID, file ownership and Pod -> PVC -> PV chain. Select a target template whose
   `workspace_user`/`workspace_home` resolve to the compatible user and home mount. The bootstrap
   does not recursively rewrite durable ownership. Preserve `.codex/sessions`, logs, SQLite/WAL,
   auth/configuration and repositories; only explicitly disposable scratch/cache paths may differ.
2. Create one normal managed target using the authorized organization/owner/template request and
   idempotency key. It receives a new workspace ID and deterministic PVC name. Wait for normal
   provisioning to create and bind its initially empty target PVC/PV, then record its identity,
   labels, claim UID and PV.
3. Call the target workspace `stop` action. Wait until its API state is `Stopped`, StatefulSet
   replicas are observed as zero, no target Pod remains, and its Longhorn volume is detached. A
   stopped reconcile keeps replicas zero and does not overwrite the PVC binding.
4. At the final source freeze, stop every Coder writer from the external operator context, take and
   verify a fresh source snapshot, and wait for the Coder Pod and its Longhorn attachment to be
   gone. Do not perform this step from the Coder workspace itself.
5. Change both the target empty PV and the Coder source PV to `Retain` and read the policies back.
   Keep the target empty PV: it is rollback data and must not be deleted. Delete only the stopped
   target PVC and the frozen source Coder PVC after their `Retain` readback; never delete either PV
   or Longhorn volume. Clear the source PV `claimRef` under the approved change record.
6. Create the replacement target PVC with the **same deterministic target name**, source-PV
   `spec.volumeName`, compatible class/access/volume-mode/capacity, and the ownership labels copied
   from the original target PVC. Wait for Bound, then verify the source PV `claimRef` has the target
   namespace/name and the replacement target PVC's **new UID**. Do not start while the target PVC
   is absent: the StatefulSet claim template would otherwise provision a replacement claim.
7. Re-read Longhorn health/attachment and the PVC/PV chain, then call target `start`. Validate host
   key continuity where applicable, SSH, Web Shell, durable home/Codex data and a single writer.
   Keep both retained PVs and snapshots through the Coder rollback window. This phase may not pause
   or stop the four accepted MWC workspaces.

## Cutover acceptance and rollback

The MWC platform phase is accepted only when: the target namespace exists; it has exactly one
control plane and four MWC workspace identities; all four MWC PV claimRefs point to target claims
with matching UIDs; each MWC Longhorn volume is healthy and has exactly one expected attachment;
schema 22 is confirmed after the published 19->20 bridge and offline 20->22 migration; GitOps/Argo
desired state is target-only; no old MWC controller can reconcile; and each MWC workspace passes
its connectivity/data checks without durable `.codex` loss. The final global acceptance additionally
requires the external Coder phase and its separately recorded acceptance.

Rollback is permitted only before old PVC bindings are irreversibly retired and while the schema
bridge rollback procedure, encrypted SQLite export, retained PV bindings, and snapshots have been
verified. Freeze the target writer, detach it, restore the recorded old claim binding or restore a
tested snapshot, and redeploy the previous compatible control-plane image/database state. Do not
roll back across an untested schema boundary and do not delete the target volume to make rollback
easier. Escalate if any schema transformation is non-reversible.

## Copy/paste task for the external cutover agent

```text
只迁移最后一个 Coder TOKEN center dev 工作区。必须从该工作区之外执行。
先读 memeloop-workspace-control/IMPLEMENTATION_STATUS.md 的最新执行记录及
docs/FINAL-MIGRATION-RUNBOOK.md 的“Second phase”章节。

2026-09-09 已完成的部分不要重做：控制面和四个 MWC 工作区已迁入
memeloop-workspace-control，数据库已是 schema 22，原 PV、SSH 端口及主机密钥保留，
四工作区 SSH 与真实浏览器终端验收通过。不要停止它们，不要回滚数据库版本，
不要重新执行第一阶段迁移。旧命名空间只属于尚待清理的残留。

本次唯一源是 coder/coder-f5c0873c-8b1e-4099-b781-af5477688c39-rust-dev-home，
记录的 PV 是 pvc-4d799a12-262f-44ae-b3c2-c8c4cc5dfbc0，容量100Gi。
先复核当前 Pod→PVC→PV→claim UID，若已变化或另有迁移代理在操作则停止协调。
这不是 MWC rust-dev-test 的100Gi卷，不能混用。

使用正常 MWC API 创建一个目标工作区，选择兼容源实际 UID/GID、home 挂载路径和
工具链的 Rust 模板。先验证源的 /home/token-center-dev、/home/rust-dev 等真实路径/
符号链接及所有权，不凭历史名称猜测，也不递归 chown 整个 home。
目标空 PVC Bound 后通过 API 停止目标，确认无 Pod 且卷已卸载。

停止源之前保存原连接配置、SSH公钥/主机密钥指纹和会话数据的只读核验记录。
按已授权窗口停止源 Coder 工作区的所有写入进程与控制器，完成 fresh Longhorn
快照并验证 ready、无错误、可恢复。保留 .codex/sessions、SQLite/WAL/SHM、
日志、配置、认证及代码仓库；不能仅因 logs_2.sqlite 较大就删除它。
直接重绑原卷，无需复制目录。只清理经确认可再生成且已无活跃使用者的构建/
缓存/临时目录；不得用覆盖整个 home 的清理命令。

源 PV 和目标空 PV 均先改 Retain 并读回；使用 UID/resourceVersion 前置条件
只删除两个精确的已停止 PVC，绝不删除 PV。源 PV Released 后测试旧 claimRef 再
清除；用目标原 PVC 名及身份标签，volumeName 指向源 PV，创建替换目标 PVC。
核验双向 Bound 和新的 claim UID 后才通过 API 启动目标。保留空目标 PV 和快照作回滚。

验证：原 home/代码/.codex 数据、实际 user/UID、CLI 工具、凭据注入、SSH、
网页终端、单写入者、资源配置和健康卷。主机密钥变化必须经可信通道校验，不能
直接关掉 StrictHostKeyChecking。

若拥有用户电脑访问权，备份实际客户端 SSH/远程连接配置，仅将旧 Coder 连接的
主机、端口、用户和必要路径映射改为新实例，尽量保持显示名称与工作目录。
通过真实重连验证远程工作环境和历史会话可见。若无电脑权限，提供精确新连接字段
及待用户执行步骤，不声称已自动迁移桌面连接；持久会话可恢复不代表原进程存活。

最后给出验收结果、停机时段、回滚方法、保留资源和精确清理清单。只有源无写入、
目标验收通过且回滚窗口结束，才清理旧 Coder 控制器/配置/命名空间。不要批量删除
其他工作区，不要输出任何 token、私钥、凭据明文或数据库内容。
```
