# Implementation status

This is the durable continuation checkpoint. Continue from **Next actions** after context
compaction. Do not repeat completed audits unless new evidence contradicts them.

Last updated: 2026-09-08

## Execution ledger — resume here

This is the single execution ledger. Historical sections below are evidence for their recorded
revision, not a claim that the final migration or external sandbox acceptance is complete.
Do not restart completed audits. Update the rows with commits, test evidence, and deployment
status independently; a source commit is not a production rollout.

Latest decisions: control plane and all workspaces must ultimately share the exact Namespace
`memeloop-workspace-control`, with workspace-ID-qualified resource names. Keep the existing
single cross-region hybrid-cloud K3s cluster and Flannel/Tailscale topology. Do not introduce
Cilium/Calico, split clusters, or restore `runtime_profile`. Preserve durable Codex data.

| ID | Work | State / owner / next gate |
| --- | --- | --- |
| MIG-01 | Canonical shared Namespace in product and Chart | Source committed: `c0b2a80`, `0a545d3`; not deployed. PostgreSQL fixture raw-SQL fix landed with integration updates; full CI rerun still required. Latest all-target fixture cleanup owned by `http_fixture_clocks`; no rollout until green. |
| MIG-02 | Required schema 19→20→21 transitions and GitOps promotion | Pending. Existing 19→20 bridge evidence below remains valid; never skip required transforms. Stage each promotion, not the final GitOps chain at once. |
| MIG-03 | Four MWC workloads/PVCs/control-plane volume into canonical Namespace | Pending gated cutover. Reuse verified volumes; snapshot, single writer, SSH/host key/PVC/session validation before retiring old resources. |
| MIG-04 | Last active Coder TOKEN center dev workspace | External-agent cutover only. Current source PVC is 100 GiB; do not stop this workspace from inside itself. Prepare complete copyable final procedure. |
| MIG-05 | Delete superseded namespaces/resources and retired code/names | Pending after MIG-02/03/04 acceptance. Remove one-time migration compatibility only after live migration; no false claim of completion while old namespaces remain. |
| IMG-01 | Upgrade maintainance and rust-dev-test to verified images | Waiting for borrower release of ports 31871/32671; preserve sessions/WAL/logs. 2026-09-08 messaging tool returned unavailable, so release is not confirmed. |
| UI-01 | Final UI closeout | Source committed `c3596e9`, embedded assets `44eca28`; publish/deploy and targeted responsive regression remain. Do not redo earlier twelve-feature audit. |
| SEC-01 | Actual NetworkPolicy enforcement, same-/cross-node tests | Harbor recovered by separate ops. Ingress deny passed all four paths; cross-node ingress selector allow FAILED. Egress matrix passed 12 checks. Boundary retry passed 30 checks including same/other node API/kubelet denial, Kubernetes Service denial, public TCP and DNS retained. Evidence: `/tmp/mwc-network-{acceptance,egress,boundaries-alidns}-20260908.json`; all disposable namespaces deleted. No test process remains running. Other-node and startup coverage remain. |
| SEC-02 | Product egress isolation and least-source ingress | Source `0f062b8`, review fix `5589e35` (global-unicast IPv6, optional DNS fail-closed). Deployment and full SEC-01 evidence pending. Live policy remains ingress-only with broad ttyd sources: external tenant acceptance is NOT passed. |
| SEC-03 | Template optional `runtime_class_name`, API/YAML/UI/Pod rendering | Source committed `37fc09b`; formatting, TypeScript and targeted draft tests passed. Full CI and live runtime acceptance pending. No silent fallback; existing ordinary templates unchanged. |
| SEC-04 | gVisor node preparation and optional RuntimeClass | iv preflight complete, pause pull recovered; default runc remains unchanged. Artifact transfer incomplete, nothing installed. Node hosts gateway/storage workloads: require restart/recovery window before registration. Installer review remains active. |
| SEC-05 | API-key allowed-template IDs and bypass prevention | Source `f02d7cb`, bypass fixes `205be6e`, `506f13f`, `0f90fd9`; HTTP clocks `cec798b`. CI `34228134529` stopped at formatting, now corrected and resubmitted. Additional read-only key/user-injection HTTP regression delegated to `http_fixture_clocks`. Not deployed/accepted. |
| SEC-06 | External sandbox release acceptance | Pending SEC-01..05. Verify network escape paths, privilege/credential boundaries, CPU/memory/disk/PID pressure, SSH/Web Shell, restart/reschedule; fail closed. Installing components alone is not acceptance. |
| OPS-01 | Operator-only setup and automated product checks | Hardened installer `5bd4b95` passed real-tar fixture tests, CI hook `4b5c8b0`; node preparation `7b250f7`. Host registration/canary still pending eligible kernel and recovery checks. |
| CLEAN-01 | Superseded API keys/cache injections/image policies | Preserve previous evidence; final transactional cleanup/rotation and deletion verification remain. Never expose secret values. |

2026-09-08 read-only runtime snapshot: 262 Pods have no explicit RuntimeClass, one uses `nvidia`,
and no `gvisor`/`runsc` RuntimeClass exists. This does not prove each node's default handler.
Seven nodes are Ready. July's NetworkPolicy non-enforcement document conflicts with August's
recorded kube-router/SNAT tests; SEC-01 resolves this by fresh bounded tests, not by assumption.
`77bec21` now applies NetworkPolicy before the workload; this orders API writes but does NOT
prove the node has enforced policy before the first container instruction. Acceptance must
cover startup and rescheduling. `serv-146231` runs kernel 4.18 and is excluded from current
gVisor eligibility (documented minimum 5.6). No node runtime has been changed yet.
Read-only node preflight confirmed iv's containerd v3/cgroup v2 and no runsc handler. Initially its pause
image cache was empty. Harbor proxy HEAD for `docker-io/rancher/mirrored-pause:3.6` returned 502;
the registry's node affinity excludes control-plane nodes while its PV is pinned to westlake.
User explicitly assigned repair to another ops agent; MWC does not own that infrastructure change.
The ops agent subsequently restored registry availability. The successful egress matrix observed
the cross-node request as `::ffff:10.42.4.1` at the target, versus the actual same-node client Pod
IP `::ffff:10.42.4.242`. This is target CNI gateway SNAT, not an assumption about Tailnet source
addresses. Product egress controls are viable on the tested paths; source-selector ingress
identity is not preserved across those nodes. Do not "fix" isolation by broadly trusting that
gateway source without compensating authentication/egress controls.
The boundary test initially could not reach Cloudflare before installing policy; that is not
a policy failure. Retrying with reachable public endpoint `223.5.5.5:443` passed 30 checks on
haixia/westlake. Node API/kubelet denial is observed evidence for those tested addresses, not
certification of all nodes, IPv6, public host addresses, metadata or startup timing.
Main inspected 360/1440 API-key screenshots and requested consistent checkbox styling and
removal of duplicated description; `key_template_ui_review` owns this bounded polish.
Additional haixia/iv boundary matrix passed 26 checks, zero failures; evidence
`/tmp/mwc-network-boundaries-haixia-iv-20260908.json`. Namespace
`mwc-network-acceptance-20260908130313` was removed and absence confirmed. iv is a worker,
so only kubelet (not an absent API server) was probed there.
CI `34229220420` passed formatting and found a missing token-prefix import plus a double
reference passed to CIDR containment. Both were corrected; rerun is required before publication.
User suggested cloudnium-ecs-1 / 100.64.0.10 as the first gVisor test host and authorized
careful GitOps-preferred changes with agent-owned rollback. Live Kubernetes maps that IP
to `serv-146231`, kernel `4.18.0-553.139.1.el8_10.x86_64`; no Node named cloudnium-ecs-1 exists.
This fails current gVisor's documented Linux >=5.6 requirement. It also hosts overseas Higress,
CoreDNS, Longhorn and a Wiki. No runtime or kernel change was performed. Asked user to arrange
kernel upgrade or select an eligible node; application CI and installer work continue meanwhile.
User subsequently authorized direct operation on 100.64.0.10 and installed this controller's
SSH key. Root SSH succeeded: hostname `serv.146231.com`, AlmaLinux 8.10, two retained 4.18
kernels, 610 MiB free in /boot and 11 GiB free on /. `gvisor_node_rollout` now owns read-only
kernel/package/one-shot-boot preparation and `docs/SERV-146231-GVISOR-ROLLOUT.md`; main owns
the upgrade/reboot decision and recovery. No host changes have been applied.
UI follow-up `a57c325` passed 61 frontend tests and three viewport checks; main re-inspected
the updated 360px screenshot and confirmed template checkbox styling now matches permission
cards. HTTP scope regression is committed as `504b375`.
CI follow-ups: `34230244396` found needless struct defaults (fixed `af58ead`);
`34231642062` found binary-module path resolution (fixed `43fe230`, also split egress parsing).
Do not rerun those failed revisions; follow the newest run for `43fe230`.
SEC-02 review confirmed current ttyd has no backend authentication; broad SNAT allowances
would bypass gateway tickets. `sandbox_runtime_product` is implementing per-workspace ttyd
Basic authentication using domain-separated control-plane key derivation, a ttyd-only Secret,
and Higress upstream header replacement. No ingress CIDR broadening until direct-access
denial and the normal ticket flow both pass. This is implementation work, not live acceptance.
`5bd4b95` completed installer transaction/rollback hardening. Main reran
`bash tests/gvisor_node_scripts.sh` successfully and added it to CI. This only proves fixture
behavior; no node registration/reboot or gVisor workload acceptance has occurred.
CI `34233272890` passed production compilation/maintainability checks, then found a missing
test import; `7d548a4` fixes it. Follow the newer run, not the superseded failure.
CI `34234125390` then found an unused `test_key_expiry` in `tests/admin_api.rs`.
`http_fixture_clocks` owns that correction and a bounded scan of its fixture helpers.
Node preparation is moving to verified, temporarily staged ELRepo 5.15.220 packages while
preserving AlmaLinux 8 and its existing kernels. `gvisor_node_rollout` owns download/signature/
dry-run evidence only; no package installation or GRUB/reboot yet. Requested the provider
console/power-reset recovery contact because SSH cannot recover a failed kernel boot.
2026-09-08 user explicitly authorized reboot at any time and confirmed cloud-console reset
assistance. Main verified local API/etcd readyz and saved snapshot
`mwc-gvisor-serv-20260908-serv-146231-1788877200`. Longhorn snapshot
`mwc-gvisor-serv-wiki-20260908` is ready for the single-replica overseas Wiki volume
`pvc-598e610d-9b8e-4de2-b59e-9dd5d0c05474` (a local snapshot, not an off-node backup).
Installed only the three signature-verified ELRepo 5.15.220 kernel/core/modules RPMs, keeping
all existing kernels. `/boot` retains 500 MiB free. Old 4.18.0-553.139 default was explicitly
restored after the transaction; no reboot has happened at this checkpoint. Next: verify
initramfs/drivers, cordon the exact node, set the explicit new BLS id for one boot and reboot,
then verify SSH/kernel/K3s/storage/services and uncordon. User controls console reset if needed.
Uncommitted ttyd Basic-auth draft was rejected: upstream `/token` returns the configured
credential. Worker is removing it and implementing native TLS client authentication instead.
Never publish the rejected Basic draft or widen SNAT allowances before live acceptance.
Kernel canary actually rebooted: root SSH returned with `5.15.220-1.el8.elrepo.x86_64`;
old 4.18 default remains and `next_entry` was consumed. Node is Ready and uncordoned;
K3s readyz passed after initial etcd warm-up. Overseas Higress controller 2/2, gateway 1/1,
Wiki 3/3 and Wiki volume healthy/attached. Headplane's two-replica volume remains degraded
(haixia running, replacement/old replicas stopped); node worker is inspecting recovery without
deleting data. Do not begin another runtime restart until that recovery is resolved.
CI `34236197357` reached tests; admin fixture fixes are `7bb67aa` (schema22 and bounded parent
key expiry). Main will push the committed revision; mTLS work remains uncommitted/isolated.
Follow-up recovery evidence `b2ad9e4`: Headplane replacement replica is queued behind
versetensor's existing single concurrent rebuild slot. Preserve both data and concurrency
settings; no destructive repair is justified. gVisor worker is staging verified official
artifacts/scripts on serv and doing read-only preflight while the rebuild proceeds.
The jump-host namespace fallback now uses the canonical namespace (`1adee76`).
Main has prepared all-or-none mTLS environment parsing and eight-combination tests; wait for
the runtime worker's matching config type/manifests before committing that dependent change.
CI `34241444782` failed; fixture worker owns the next log-based correction.

Migration handoff review corrected a dangerous conflation: `rust-dev-test` and Coder TOKEN center
dev are distinct 100 GiB volumes. Both actual PVC→PV claim UIDs match. Final count is four
existing MWC workspaces plus the last Coder workspace. The game-forking Longhorn volume was
degraded on this capture; health must recover before its volume cutover. Namespace-changing
schema migration is OFFLINE with the old coordinator stopped, not a rolling server upgrade.

## Active goal

Finish the single-model release, promote the verified development images, and leave production
with one collision-free resource layout across the database, Kubernetes, source, tests, docs, and
product terminology.

## Completed production migration

- The control plane is running schema 19. Its database stores only the dedicated Namespace scope
  and Namespace; every resource name and route is derived from the installation and workspace IDs.
- The installed binary exposes only `migrate`, `export`, `import`, and `migrate-to-postgres`
  database commands. The one-time transition command is absent.
- Four live workspace rows have collision-free runtime identities. All four Pods are Ready, use
  their exact canonical PVC, and retain their original SSH NodePort:
  - `maintainance`: `w-bd2dc9ca6aa2b1b5`, port `31871`, 2 GiB.
  - `tiddlywiki-dev`: `w-b268ff46894a14b9`, port `30953`, 30 GiB.
  - `game-forking`: `w-97645a0fb4771b1d`, port `30732`, 60 GiB.
  - `rust-dev-test`: `w-b405d2441a1ee149`, port `32671`, 100 GiB.
- Each workspace passed real public-key SSH command execution, one-time Web Shell access, PVC
  identity checks, host-key continuity, and a full restart with the same PVC and host key.
- Before copying `tiddlywiki-dev` and `game-forking`, the stopped source volumes were snapshotted.
  Target data was verified by a read-only content tree hash including paths, types, owners, modes,
  links, ACLs, extended attributes, and SELinux metadata. Timestamps were normalized. Regenerable
  caches, `node_modules`, and process sockets were intentionally excluded.
- Target cleanup reclaimed 1,133,764,608 bytes from `tiddlywiki-dev` and 526,131,200 bytes from
  `game-forking` in addition to their earlier cache allowlist cleanup. `.codex`, repositories,
  SSH/Git settings, and user files were preserved.
- After start/restart acceptance, the superseded Kubernetes objects, PVCs, PVs, Longhorn volumes,
  and temporary migration Jobs were removed. They are not recoverable from the removed source
  volumes. The healthy target volumes and these pre-start snapshots remain:
  - `mwc-target-b268ff46894a14b9-prestart-20260907`
  - `mwc-target-97645a0fb4771b1d-prestart-20260907`
- Encrypted mode-0600 database checkpoints are stored outside the repository at
  `.mwc-migration-backups/2026-09-07/control-plane-pre-v19.json` and
  `.mwc-migration-backups/2026-09-07/control-plane-v19.json`. Never print their contents.

## Current deployed release

- Product revision: `554a13033800ec10d31b0c4d1a3758623a187290`.
- GitHub Actions run `34155589876` passed the complete frontend, Rust, SQLite,
  PostgreSQL, bootstrap, Helm, image-contract, publication, and provenance suite.
- Control-plane image:
  `sha256:d0bba81f16f9daa5f64239ba575152befba964451a3c4cc89deafbe25e7a1308`.
- ttyd image:
  `sha256:6746fbe74acdec35f899ff7d82ba71989c7c6c0e22f609550665ea3a7fc11bdd`.
- Workspace-base image:
  `sha256:b4d263e2cf4cc8b7818399a24beca87c9c4f5791dbfde4b390c3f9210f402590`.
- GitOps commit `3c290f3` pins the exact revision and image digests. Argo CD reports the
  application Synced/Healthy. Public `/metrics` returns 404; the internal 8081 endpoint serves
  OpenMetrics; `/livez` and `/readyz` return HTTP 200.
- The stable `workspace-data` claim-template name is deployed. PVC identity remains unique through
  the StatefulSet name (`workspace-data-w-<workspace-short-id>-0`), avoiding immutable updates and
  retaining existing data.

## Development images and templates

- `Maintenance`: `harbor.k3s.onetwo.website/library/cluster-admin:20260907-22c45c3` at
  `sha256:9b9f6cfc8bd2197fdc9a2e69ada3a8b7d0bdc129ba33908f584ce0e49a88b2e8`.
- `Node Dev`: `harbor.k3s.onetwo.website/library/node-dev:20260907-d866f4d` at
  `sha256:4b938d6210d5f2bebbd43d96c51d23d969ee0f47540d6e369931a1c756b2bc12`.
- `Rust Dev`: `harbor.k3s.onetwo.website/library/rust-dev:20260907-0416764` at
  `sha256:e71597592feacc4adb69643931af3bf4ee7f3f1060aac302c2a8f595a105a4b3`.
- Forgejo CI and real K3s pulls verified Rust 1.98.1, Cargo 1.98.1, Node 24.20.0,
  npm 11.19.0, Codex CLI 0.153.4, GitHub CLI 2.100.0, Helm 4.2.4, kubectl 1.36.4,
  OpenSSH, jq, clang, cmake, and the required browser/native build libraries.
- The three active templates and image policies now point to these immutable images. Three unused
  duplicate templates were disabled and deleted through the audited API.

## Source closeout completed

- The clean schema baseline initializes new databases directly at schema 19 and rejects older
  database versions with one generic unsupported-version error. Historical transformation SQL,
  compatibility branches, stale terminology, and the transition CLI are removed.
- A system-admin-only `PUT /api/v1/workspaces/{workspace_id}/image` endpoint safely upgrades a
  stopped workspace. It requires an enabled exact lowercase SHA-256 image policy, optimistic
  generation matching, an idempotency key, and atomically updates the template snapshot, image,
  generation, reconcile job, audit row, and event in SQLite or PostgreSQL.
- Independent review caught and fixed non-hex digest acceptance. Production implementation files
  are below the 400-line maintainability target. The final main-branch suite and tracked-tree scan
  for removed compatibility terminology pass.

## Active closeout

- `tiddlywiki-dev` and `game-forking` are Ready on the latest Node/Rust, BuildKit and ttyd images.
  Their canonical PVCs and SSH host keys are unchanged. Host-key-checked SSH verified Node 24.20,
  Rust/Cargo 1.98.1, Codex CLI 0.153.4 and retained `.codex/sessions`.
- Codex argument-zero scratch now uses Pod-lifetime storage at `/var/lib/mwc/codex-scratch/tmp`;
  `.codex/tmp` and `.codex/.tmp` point there while sessions, logs, SQLite and WAL remain durable.
  The old cleanup warning is gone. Web Shell passed a real ticket, WebSocket, resize and command
  interaction test; ticket replay returned 401 and logs contain no `execvp failed`.
- Source `54bd4c5` contains the schema-20 bridge and removes template environment/ownership fields
  from the product model and UI. GitHub Actions run `34163836250` passed the complete suite and
  published attested images. The release is intentionally not deployed yet. The bridge transaction
  rejects non-empty old environment data, preserves unrelated idempotency JSON, and avoids
  scheduling a workspace with an active coordination lease.
- Published image digests for the pending bridge release are:
  - control plane: `sha256:ead3563710ac0c196e1575e1c9a82ab9fdc30410a66b479fce7df53f1e20094a`;
  - workspace base: `sha256:bfd8ec19f693d1ccff258bf7c6d3c69b9798e34f8dbbcc7b9ce30ef96184712c`;
  - ttyd: `sha256:83a512e43b3f624e4476cde2cbf08f8b3c6b6b6c1b089de9cfdfc74b4b99d6ce`;
  - SSH jump host: `sha256:1b2729514fde871412e1fefb0719e855a4bde1c02cc52e6e71b32518084601d2`.
- The follow-up pure-schema-20 release is published from `main` at
  `85397636219585ff523a5f89a605864962f0d002`. It deletes the 19-to-20 bridge, its module/error, the old
  health-route tombstone, and transition-specific template checks. A generic recursive schema-20
  YAML validator rejects unknown fields without naming retired fields. Independent review found no
  P0/P1/P2 issue; frontend 53 tests, plugin sandbox, TypeScript checking, production build, and the
  retired-term scan pass. GitHub Actions run `34171842403` passed and published all four images
  with matching provenance. Production remains pinned to an immutable older revision, so this
  release cannot bypass the required bridge rollout.
- Bootstrap regression coverage explicitly preserves Codex sessions, logs, SQLite/WAL, config, and
  auth files, using the concrete `logs_2.sqlite`/SHM/WAL and `session_index.jsonl` names. Only
  `.codex/tmp` and `.codex/.tmp` use bounded Pod-lifetime scratch storage.
- The bridge rollout is staged but not pushed or deployed on GitOps branch `mwc-schema20-bridge`
  at `5fe5848`, rebased on current GitOps `7900c2a`. Helm lint/render and diff checks pass. It pins
  source `54bd4c5` and the verified
  control-plane, ttyd, and disabled jump-host digests; the workspace-base image remains correctly
  managed through the database image policy rather than a nonexistent Helm value.
- The subsequent pure-schema-20 GitOps state is staged locally at `845b2e0` on
  `mwc-schema20-final`, after bridge commit `5fe5848`. It pins source `8539763` and the verified
  control-plane, ttyd, and disabled jump-host digests. Its maintenance script emits only current
  schema-20 template fields. Helm lint/render, PowerShell parsing, retired-field scanning, and
  `git diff --check` pass. It has not been pushed or deployed.
- Another Codex task is actively using ports `31871` and `32671`. Normal `.codex` session, WAL,
  and log writes are expected and safe. Do not stop, restart, or switch those two workspaces until
  that task reports completion.
- The final GHCR workspace-base digest is enabled in the image policy. Three superseded
  workspace-base policies and six unreferenced old development-image policies are disabled. The
  two old policies still referenced by `maintainance` and `rust-dev-test` remain enabled until
  their coordinated upgrade.
- A live encrypted schema-19 export was preflighted without printing values: all three template
  YAML documents and four workspace snapshots parse, all seven old environment maps are empty,
  and the seven ownership fields are safe for the bridge transaction to remove.

## Next actions

1. Do not deploy the already-published schema-20 release while ports `31871` and `32671` are
   borrowed.
2. After the borrowing task explicitly finishes, stop and snapshot `maintainance` and
   `rust-dev-test`, deploy the schema-20 bridge, and verify database migration plus all four
   workspace reconciliations without cleaning durable `.codex` state.
3. Upgrade the remaining two workspaces to the final development images through the audited API,
   verify SSH/Web Shell/PVC/host-key continuity, then disable their superseded image policies.
4. After the bridge migration and live schema-20 records are verified, advance GitOps to staged
   commit `845b2e0` and validate its exact published digests. Never skip directly to this state.
5. Replace broad Higress source CIDRs with verified per-node CNI gateway `/32` addresses in a
   staged GitOps rollout. A live WebSocket to a westlake workspace proved the backend source as
   `10.42.3.1`, not the gateway's Tailnet address. Public WebSocket and port mappings must pass
   while unrelated Pod access to 7681 fails; new cluster nodes must add their gateway `/32`.
6. Perform one final database/Kubernetes/source acceptance pass and update this checkpoint. Do not
   repeat completed migration, CI, observability, or terminology audits without new contrary
   evidence.

## Safety invariants

- Do not interrupt ports `31871` or `32671` until the borrowing task reports completion.
- Never expose tokens, private keys, decrypted credentials, certificates, or database snapshots.
- Workspace image changes require the stopped-state, exact-digest, policy, generation,
  idempotency, audit, and reconcile gates; do not edit SQLite directly.
- Do not delete a retained target PVC or pre-start snapshot during source closeout.
