# Implementation status

This is the durable continuation checkpoint. Continue from **Execution ledger** after context
compaction. Do not repeat completed audits unless new evidence contradicts them.

Last updated: 2026-09-09

## Execution ledger — resume here

This is the single execution ledger. Historical sections below are evidence for their recorded
revision, not a claim that the final migration or external sandbox acceptance is complete.
Do not restart completed audits. Update the rows with commits, test evidence, and deployment
status independently; a source commit is not a production rollout.

Latest decisions: control plane and all workspaces must ultimately share the exact Namespace
`memeloop-workspace-control`, with workspace-ID-qualified resource names. Keep the existing
single cross-region hybrid-cloud K3s cluster and Flannel/Tailscale topology. Do not introduce
Cilium/Calico, split clusters, or restore the removed runtime-profile abstraction. Preserve durable Codex data.

| ID | Work | State / owner / next gate |
| --- | --- | --- |
| MIG-01 | Canonical shared Namespace in product and Chart | COMPLETE. Production uses exact `memeloop-workspace-control`; CI `34324256107` and GitOps `eb05819` passed. |
| MIG-02 | Required schema 19→20→22 transitions and GitOps promotion | COMPLETE. Offline migration and integrity evidence recorded below; production schema22. Compatibility removed in deployed a9bb4e4. Never rerun bridge steps against current DB. |
| MIG-03 | Four MWC workloads/PVCs/control-plane volume into canonical Namespace | COMPLETE. Five original PVs directly rebound; four strict SSH and real browser ttyd checks passed. All canonical claims Bound. |
| MIG-04 | Last active Coder TOKEN center dev workspace | External-agent cutover only; pending execution. Copyable procedure is in `docs/FINAL-MIGRATION-RUNBOOK.md`, updated for completed MWC migration. Source PVC 100Gi; do not stop this workspace from inside itself. |
| MIG-05 | Delete superseded namespaces/resources and retired code/names | Five old MWC namespaces deleted and absence confirmed; bridge code removed/deployed. Remaining Coder cleanup belongs to MIG-04. Preserve immutable installation identity and recovery artifacts. |
| IMG-01 | Upgrade maintainance and rust-dev-test to verified images | COMPLETE. Borrower released both ports; images upgraded during accepted cutover. Old unreferenced 20260902 policies disabled. |
| UI-01 | Final UI closeout | Production 768/1440 checks pass; 360px has 15px right-edge clipping. Main inspected actual screenshots, also found low-contrast settings metadata. `canonical_ui_acceptance` owns bounded CSS fix and viewport regression; no full-feature re-audit. |
| SEC-01 | Actual NetworkPolicy enforcement, same-/cross-node tests | Harbor recovered by separate ops. Ingress deny passed all four paths; cross-node ingress selector allow FAILED. Egress matrix passed 12 checks. Boundary retry passed 30 checks including same/other node API/kubelet denial, Kubernetes Service denial, public TCP and DNS retained. Evidence: `/tmp/mwc-network-{acceptance,egress,boundaries-alidns}-20260908.json`; all disposable namespaces deleted. No test process remains running. Other-node and startup coverage remain. |
| SEC-02 | Product egress isolation and least-source ingress | Source `0f062b8`, review fix `5589e35` (global-unicast IPv6, optional DNS fail-closed). Deployment and full SEC-01 evidence pending. Live policy remains ingress-only with broad ttyd sources: external tenant acceptance is NOT passed. |
| SEC-03 | Template optional `runtime_class_name`, API/YAML/UI/Pod rendering | Source `37fc09b` included in deployed release; full CI passed. Live gVisor acceptance remains blocked on SEC-04. No silent fallback; existing ordinary templates unchanged. |
| SEC-04 | gVisor node preparation and optional RuntimeClass | User authorized root access and reboot of 100.64.0.10 (`serv-146231`). Kernel 5.15.220 booted; K3s active, old 4.18 remains default for rollback. runsc NOT installed: systemd 239 fails the supported systemd-cgroup prerequisite; do not bypass with filesystem cgroups. User now authorizes OS-upgrade PLANNING only; `gvisor_node_rollout` owns `docs/SERV-146231-OS-UPGRADE-PLAN.md`. Do not execute an OS upgrade yet. |
| SEC-05 | API-key allowed-template IDs and bypass prevention | Full CI and production safe-path acceptance PASS. Temporary template-bound parent/child keys authenticated; wider-template/longer-expiry mint and scope-exceeded reads returned403; both revoked and returned401. Resource mutation bypass paths remain covered by CI, not production mutation tests. |
| SEC-06 | External sandbox release acceptance | Pending SEC-01..05. Verify network escape paths, privilege/credential boundaries, CPU/memory/disk/PID pressure, SSH/Web Shell, restart/reschedule; fail closed. Installing components alone is not acceptance. |
| OPS-01 | Operator-only setup and automated product checks | Hardened installer `5bd4b95` passed real-tar fixture tests, CI hook `4b5c8b0`; node preparation `7b250f7`. Host registration/canary still pending eligible kernel and recovery checks. |
| CLEAN-01 | Superseded API keys/cache injections/image policies | Preserve previous evidence; final transactional cleanup/rotation and deletion verification remain. Never expose secret values. |

2026-09-08 read-only runtime snapshot: 262 Pods have no explicit RuntimeClass, one uses `nvidia`,
and no `gvisor`/`runsc` RuntimeClass exists. This does not prove each node's default handler.
Seven nodes are Ready. July's NetworkPolicy non-enforcement document conflicts with August's
recorded kube-router/SNAT tests; SEC-01 resolves this by fresh bounded tests, not by assumption.
`77bec21` now applies NetworkPolicy before the workload; this orders API writes but does NOT
prove the node has enforced policy before the first container instruction. Acceptance must
cover startup and rescheduling. At this initial snapshot `serv-146231` ran kernel 4.18;
the later authorized 5.15 boot and remaining systemd prerequisite are recorded above.
No node runtime handler has been changed.
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
CI fixture correction `d556493` fixed an invalid environment-variable target in the new
authorization test, preserving the 403/success assertions. Subsequent run `34243036481`
failed; fixture worker is checking its specific result.
gVisor official archive now staged and checksum-verified on serv at
`/var/tmp/serv-146231-gvisor-20260831.0.Q2DBoz`. Installed AlmaLinux's `zstd-1.4.4-1.el8`.
Fixed preflight's early-exit awk SIGPIPE bug; real preflight now correctly reaches the next
gate: systemd 239 is below the documented 244 minimum for runsc's systemd-cgroup mode.
No runsc installation, handler change or further restart occurred. Worker is checking the
actual kubelet/CRI cgroup driver and supported alternatives without weakening checks.
Main acknowledges this systemd prerequisite should have been checked before the kernel reboot.
mTLS parser/client fixture committed `082e20f`; do not push it without the runtime worker's
matching type/manifest commit.
Native ttyd mTLS implementation landed in `4e1a584`; CI `34244261180` found duplicate DNS
validation and a missing Ingress metadata import, now fixed. `293c773` corrects the storage
authentication fixture's real-clock expiry without altering fixed-clock storage tests.
Added Helm CI coverage for complete mTLS env rendering and partial-configuration rejection.
`netpol_evidence` owns a disposable real-ttyd mTLS test: no certificate/wrong CA denied,
valid certificate accepted, `/token` contains no reusable Basic credential. This does not
yet prove Higress end-to-end behavior or permit broadening production ingress sources.
Node compatibility decision `145393d`: serv's actual CRI emits systemd-form cgroup paths;
runsc fs mode does not supply equivalent Kubernetes Pod hierarchy/limits, and its systemd
driver requires >=244. Do not bypass preflight, mix drivers or register runsc there.
Asked user whether to authorize the original iv node's maintenance test or supply a newer-OS
node. serv remains operational on its one-shot 5.15 kernel, old 4.18 still the default; no
gVisor handler or readiness label was installed. Other product/CI work continues.
2026-09-08 storage recovery completed: Headplane volume
`pvc-6a03d5cb-b64b-43e1-b236-5bf3de83a613` is healthy; engine reports both haixia and
replacement replicas RW, rebuildStatus empty. No forced replica deletion/concurrency change
was used. The remaining gVisor gate is runtime/node compatibility, not storage recovery.
CI `34245226707` found workspace_volumes over its 100-line limit; `5930a12` extracts the
independent ttyd TLS volume constructor. Formatting passed; follow the next CI revision.
First native ttyd mTLS test was inconclusive: negative TLS request broke its port-forward,
making subsequent valid-client tests connection failures. Disposable namespace and keys were
removed. Retest uses a valid-client baseline and independent forwarding sessions plus Pod
health/log evidence; no claim of TLS end-to-end acceptance yet.
CI `34247025288` found the TLS manifest test used `Option<bool>` as a bool;
`37ef1d1` now requires `read_only == Some(true)` without weakening the assertion.
Second ttyd canary never reached TLS (Pod readiness timeout); it was cleaned but missing
pre-cleanup failure evidence. Main stopped ad-hoc retries and assigned a durable TS runner
with failure diagnostics before cleanup; review that runner before another cluster test.
Runtime worker owns `docs/WEB-SHELL-MTLS.md` and its README link (Secret roles, SAN/EKU,
rotation and acceptance limits). These docs must not imply live mTLS rollout is complete.
Documentation completed in `e3f9cc4`. CI `34247950017` passed compilation and business_storage,
then exposed an actual snapshot-import validation gap: json_populate_recordset silently
ignores unknown workspace fields. Runtime worker owns strict schema22 field validation and
database_snapshot regression, preserving rejection semantics. CI now uses --no-fail-fast
to collect failures across test binaries in one run instead of stopping at the first binary.
TLS runner review still pending fixes for forwarding readiness, transport-vs-auth failures,
/token validation and cleanup on namespace-deletion errors; no new cluster test approved yet.
Main corrected runner's `/token` assertion to parse JSON `{token:""}` rather than expecting
an empty HTTP body, and cleared its forwarding readiness interval on timeout. Native mTLS
canary is now running as exec session `59638`; resume that exact session for results.
Do not start another canary while this process exists. It owns precise namespace cleanup.
Session `59638` completed failed due to historical image `6746…1bdd` Harbor manifest redirect
loop, and its namespace was confirmed absent. Main fixed malformed diagnostic JSON parsing,
then retried the GitOps-pinned image `0605fae2319fb64efaa5daeb4c98e2afe75e19c4047513a7dad25bf3e95aac46`.
Session `12709` PASSED native ttyd mTLS: valid-client HTML (728583 bytes), `/token` JSON token
empty, missing/wrong client rejected, valid-client recheck passed, Pod restartCount zero.
Namespace `mwc-ttyd-mtls-20260908162656` was deleted and absence independently confirmed.
No canary process remains. This is native ttyd evidence only; Higress upstream mTLS and
end-to-end ticket/WebSocket acceptance are still required.
Snapshot validator test-module import/baseline fixed in `4f25112`; continue its newest CI.

Migration handoff review corrected a dangerous conflation: `rust-dev-test` and Coder TOKEN center
dev are distinct 100 GiB volumes. Both actual PVC→PV claim UIDs match. Final count is four
existing MWC workspaces plus the last Coder workspace. The game-forking Longhorn volume was
degraded on this capture; health must recover before its volume cutover. Namespace-changing
schema migration is OFFLINE with the old coordinator stopped, not a rolling server upgrade.

## Active goal

Finish the single-model release, promote the verified development images, and leave production
with one collision-free resource layout across the database, Kubernetes, source, tests, docs, and
product terminology.

## Historical checkpoint — schema19 migration (2026-09-07)

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

## Historical release — 554a130 (superseded)

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

## Historical source closeout — schema19 (superseded)

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

## Historical closeout preparation — before canonical cutover

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

## Historical rollout sequence — do not resume from this section

Current priority (supersedes historical rollout ordering below): productize the verified Higress
server-name validation fix and continue the gated migration with the published release.
Latest poll: `verify` PASSED; image publication status is below. No production-upgrade
claim yet. OS planning document landed in `4a68871`, migration
runbook path correction in `0e2c8f2`; OS execution remains unapproved.
CI `34254345334` finished SUCCESS (verification and all four image publication jobs).
All four OCI index digests were independently fetched from GHCR and SHA-256 checked without
downloading image layers. Immutable digests from successful job logs:
control-plane `sha256:299f9a18019baea893cc93ac1a397cee3149f66660113a0a4c3ac09b3a898883`,
ttyd `sha256:6f430bf2941bbb0e2110a5211a4884018efd72ec52a217b6db1b442d5b34eed7`,
workspace `sha256:f4161453c3fa5dbbea71e76fffa199a7a7a3db6178411d4f59420c18596bcbb0`,
ssh-jump `sha256:1eb742878128151f98a620651822ceab291dfd714f3200b468582619adccc081`.
Corrected runbook `8e2321e` separates four-MWC cutover from the later external Coder cutover;
`773e61a` gives the existing managed-create → stop → retained-PV rebind → start path.
No requirement to stop the current Coder workspace now. Borrower-release confirmation remains
pending: async question sent to user because the cross-task messaging tool still fails.
GitOps runner `49c6e6f` plus scoped-diagnostics fix `75d31f0` ran to completion (session `8291`).
Evidence `/tmp/mwc-higress-ttyd-mtls-20260908.json`: baseline/recovery 200, absent-client and
wrong-server-CA rejection passed; wrong-server-SAN rejection FAILED (200 remained possible).
Exact namespace `mwc-higress-ttyd-mtls-1788886900146-j1xdbb` was deleted and absence verified.
That first canary ended. Companion CA key corrected to official `cacert` in GitOps `e8acafb`.
New standard EnvoyFilter canary `327418d` plus main's masked-CDS parsing/gateway-pinning fix
`3a329a3` PASSED, session `4072` exited 0. Output `/tmp/mwc-higress-ttyd-san-20260908.json`;
namespace `mwc-higress-ttyd-mtls-1788888479795-bwcwhy`, exact gateway filter
`higress-system/ttyd-mtls-san-1788888479795-bwcwhy`. Uses newly published ttyd digest `6f430bf…`.
It matched only temporary Service/7681: valid wildcard 200, wrong CA 503, wrong SAN 503, both
restorations returned 200. Cleanup failures were empty; filter and namespace absence verified
independently. No canary process remains. `sandbox_runtime_product` now owns productization
of the exact per-workspace filter lifecycle, ownership checks, Chart permissions and tests.
Gateway permissions must use a conditional Role/RoleBinding in the gateway Namespace, not
cluster-wide EnvoyFilter access. Main owns the documentation and ledger. `release_manifest_plan`
(Luna) completed staged promotion `8587d39` in the GitOps worktree: four manifest files use
the canonical Namespace and verified `32945cc` release, an existing target SQLite claim,
and paused automated sync. Helm/kustomize/repository checks passed; main reviewed the diff.
Nothing was pushed or applied. Do not push the staging branch wholesale (31 commits ahead);
promote the exact reviewed delta onto then-current GitOps `origin/master` only after cutover gates.
Main startup validation landed in `7473093` (namespace mismatch fails before serving);
rustfmt/diff checks passed, full Rust verification awaits the worker's complete patch and CI.
Main Helm regression `0200a66` parses rendered YAML and checks exact gateway Role verbs,
RoleBinding namespace/SA, absence of cluster-wide filter grants and disabled-mode Role absence.
Local enabled/disabled renders passed; namespace mismatch and a mutated wildcard-verb fixture
were rejected. These checks are wired into CI; do not push them without the matching Chart
and lifecycle implementation currently owned by `sandbox_runtime_product`.
Product implementation landed in `20efc85`; main review fixes `c63cb93` retain the filter until
Ingress absence is observed and clean the cross-namespace filter even if the workspace Namespace
is already gone. `http_fixture_clocks` owns fake-Kubernetes lifecycle regressions (not only
manifest assertions), including disabled-mode no-CRD access, missing credentials, ordering,
foreign ownership, UID preconditions and missing Namespace cleanup.
Early CI `34262698041` stopped on `ResourceBuilder::build` length (107/100), not an API/type
error. Main refactor `80a1a67` separates gateway validation from resource assembly without
weakening lint rules. Follow-up CI `34263765347` passed production checking, then stopped on
the renderer test's boolean-comparison lint. Main `c094554` replaces the string search with
a structural absent-field assertion; rustfmt passed. Verification-only CI `34266603901`
PASSED for `67b1f53` on `ci/ttyd-san-c63cb93`, including the full test and deployment-check
steps. Images were intentionally skipped. This validates the product, lint fixes and cache
setup but does not include the new lifecycle regression commit `54f9409`.
`http_fixture_clocks` completed lifecycle regressions in `54f9409`. Main review fixes include
full-call assertions that the SAN filter survives Ingress DELETE until absence is observed,
a retained finalizer over multiple calls, and missing-UID deletion rejection. The 413-line
scenario module shares 249/248-line harness and fixtures; formatting/diff checks passed.
Verification CI `34267525211` PASSED for `a1b9257`; its logs explicitly show all nine
`client_tests::ttyd_mtls` scenarios passing, plus full tests, ENOSPC and Helm checks.
Main follow-up `b91fc80` strengthens disabled-mode assertions to include collection access;
the main-branch publication pipeline must revalidate that assertion before image publication.
`release_manifest_plan` (Luna) completed the Rust-dependency CI cache in `618c300`, with an
immutable upstream action pin. It now prepares a read-only gap analysis for authenticated
WebSocket acceptance. Its gap analysis is complete; the agent now implements a client-only
upstream certificate acceptance mode without executing it. `key_template_ui_review` prepares
a separate real-browser ticket/interactive-terminal/replay runner. GitOps client-only runner
`8d8a4e7` is committed with static/repository checks passed; main runtime review and execution
remain. No cluster changes are delegated.
Local YAML parsing with duplicate-key rejection passed for the CI workflow; the immutable
cache pin and unchanged full `--all-features --no-fail-fast` test step were asserted.
Actual cache execution and lifecycle test results still require GitHub Actions.
The lifecycle verification gate has passed. Promote the reviewed product source to main for
the publication pipeline; do not deploy before published digests and rollout gates are verified.
Main promotion `1e0ab66` is pushed; publication CI `34268442698` FAILED in two `admin_api`
tests (expected user creation 201, got 403); no images were published. Main traced an actual
default-expiry bug: omitted child expiry becomes request-time + 30 days, exceeding an issuer
seeded with the same TTL once the clock crosses a second. `d665e89` caps only the default at
issuer expiry, while explicit over-parent requests remain forbidden. `http_fixture_clocks`
completed deterministic short-lived-issuer regression `14d5060`; main review follow-up
`681da5f` separates scope escalation from an explicit over-parent expiry case, so neither
assertion can pass for the other reason. Main reviewed both; formatting/diff checks passed.
Fix and regression are published as `df281ea`; main CI `34270275217` PASSED verification
and all four image publications. Watch session `20896` exited 0 and is no longer running.
Main independently fetched each public OCI index and verified its SHA-256 against the registry
header for `sha-df281ea` (no image layers downloaded, authentication held only in memory):

| Image suffix | Verified OCI index digest |
| --- | --- |
| control plane | `sha256:b5b946a6855dbd937404fb77a014aa8ddc1efb35e26fb5d5e11c6113e228632c` |
| `-ttyd` | `sha256:c2a7ff260d2f1de7f129865fb93d9cc24d80311e202ef668fcbec1bacfa6ace6` |
| `-workspace` | `sha256:c37fa3729f548bec8a693c956f05017fb5af08f508145253b23ea843579be12b` |
| `-ssh-jump` | `sha256:6b1db4a26be39a32136a9a03c97f94378a7d94b6bdb9a7d6e84323e0f7efb6a8` |

`release_manifest_plan` completed final pin update `7206fdc`; main reviewed exactly four
changed fields (source revision, control/ttyd/jump digests). Helm rendering and diff checks
passed. The full candidate is the namespace/manual-sync change `8587d39` plus pin update
`7206fdc`, not the pin-only commit alone. Neither is pushed/applied.
Keep manual sync, existingClaim, migration gates and mTLS-disabled configuration; do not
deploy the schema-22 release over a schema-19 database or push the staging branch wholesale.
Browser runner review requires UUID v7 acceptance, CDP socket identity from `webSocketCreated`,
and rendered xterm-buffer matching (not an assumed DOM renderer or command-input echo).
The browser worker is correcting these before live execution.
Read-only normal-user API check selected Ready `tiddlywiki-dev`
(`01a06174-8ce2-7d52-b268-ff46894a14b9`) for browser acceptance, avoiding borrowed workspaces.
The existing `mwc-daily-user-tokens` Secret in `mwc-k3si-7032544955`, key
`lindongwu11-token`, authenticated successfully; its value was held only in memory.
List API requires `organization_id`, obtainable from `/api/v1/me` memberships.
Do not repeat credential discovery or output token values. Browser runner `a38de51` is now
committed and main-reviewed. Real run session `30199` passed the first authenticated socket,
split-marker command execution and actual `stty size` resize, then FAILED the replay evidence
wait (no explicit 401/403 was recorded). Fresh-ticket recovery was not reached. Session ended,
browser contexts and temporary token file were cleaned by `finally`. Do not claim full browser
acceptance from that failed run. Follow-up `4bc8926` handles Chromium's explicit HTTP-401
authentication error event; main checked its upstream source mapping and reran all three
fixtures, including real local Chromium against a server returning 401.
Real browser session `62163` then exited 0: initial 101, split-marker command, actual terminal
resize, consumed-ticket rejection and fresh-ticket recovery all PASSED on `tiddlywiki-dev`.
All browser contexts and the generated owner-only token file were cleaned in `finally`;
no query/ticket, screenshot or trace was persisted. Do not repeat this standalone browser gate
absent a relevant change. This ran against the existing installation without upstream mTLS:
it is not proof of the new product release's combined mTLS + external-auth rollout.
Release publication is confirmed above; deployment and combined rollout acceptance remain pending.
GitOps app README is added in `dc3b1c6`, refined in `bc4e709`: separate console-access
instructions with single-line Bash/PowerShell commands for `lindongwu11-token`. Main verified
Bash syntax and PowerShell decoding using a fake value, never the real credential.
GitOps SSH access is restored: the missing Forgejo RSA host-key record was independently
matched between both authenticated-cluster Pods, live port 30022 and Argo CD configuration
(`SHA256:ewYa4jlChcGPmsOQAyKQXUErp4g8x9BWIKmrAJyZLEM`). Main added only that verified key
to effective `/home/rust-dev/.ssh/known_hosts`; strict checking stayed enabled. Fetch succeeded.
After confirming current remote master lacked the README, main created the isolated worktree
`/home/token-center-dev/workspace/k3s-gitops-mwc-console-access` from `c0300f9` and cherry-picked
only README commits as `957f9c1`/`ebd7d8b`. Push to GitOps master succeeded; the remote change is
exactly one Markdown file, with no Application or migration changes.
Product follow-up CI `34273967636` for `6bc57e7` also PASSED. No CI job remains active.
Next execution gate is still explicit release of borrowed ports 31871/32671 before stopping,
snapshotting and migrating those workspaces; current TOKEN center dev remains external-only.
Main reviewed GitOps `8d8a4e7`; client-only live acceptance PASSED with the already verified
ttyd `6f430bf` image. Session `27715` exited 0; result is
`/tmp/mwc-higress-ttyd-client-20260908.json`. Valid / absent / restored / untrusted / restored
client certificates produced stable 200 / 503 / 200 / 503 / 200 with matching active SDS.
Cleanup failures are empty; Namespace `mwc-higress-ttyd-mtls-1788895385437-957o0r` and filter
`ttyd-mtls-san-1788895385437-957o0r` independently confirmed absent. Do not repeat this passed
matrix absent a relevant change. Browser authentication and product rollout remain pending.
Disable is explicitly two-stage: keep mTLS/RBAC while removing owned Ingresses first and
then filters; remove mTLS/RBAC only after absence checks. Disabled installations must not
query EnvoyFilter APIs. Settled workspaces are not automatically requeued on configuration
restart, so operator cleanup must not assume a periodic resync or start stopped workspaces.
Do not repeat this passed canary absent a relevant change. Production rollout, certificate
lifecycle and authenticated WebSocket acceptance remain pending. No gateway fork or broad
cluster patch is authorized. Do not weaken the acceptance assertion.
Native ttyd acceptance already passed; do not repeat it. Never apply broad SNAT source allowances without the gateway
authentication boundary verified end to end. Shared-namespace cutover still requires the
schema bridge, writer shutdown, retained-volume and rollback gates.

1. **2026-09-09 authorization received:** the user confirmed no MWC workspace is in use.
   Ports `31871` and `32671` are released; stopping and migrating the four MWC workspaces
   is authorized. Resume from fresh storage/snapshot checks and GitOps automation freeze,
   not from repeated completed feature/CI audits. The current Coder TOKEN center dev
   remains excluded. Node major-OS-upgrade authorization remains planning-only.
2. After the borrowing task explicitly finishes, stop and snapshot `maintainance` and
   `rust-dev-test`, deploy the schema-20 bridge, and verify database migration plus all four
   workspace reconciliations without cleaning durable `.codex` state.
3. Upgrade the remaining two workspaces to the final development images through the audited API,
   verify SSH/Web Shell/PVC/host-key continuity, then disable their superseded image policies.
4. After bridge acceptance, perform the offline 20→22 migration and target volume binding.
   The current final manifest candidate is GitOps `8587d39` plus `7206fdc`, pinned to verified
   `df281ea` images with automated sync paused. Rebase only that combined reviewed delta onto current GitOps
   `origin/master` for the controlled promotion; do not push the staging history wholesale or sync
   before the target database/PVC is ready. Keep mTLS disabled until its own rollout gates pass.
5. First prove Higress→ttyd mTLS including invalid-client and invalid-server-trust rejection.
   Then scope any required SNAT source allowances to measured CNI gateway `/32` addresses.
   Those addresses alone do not identify Higress: cross-node unrelated Pods may share the
   translated source. Verify they cannot establish a ttyd TLS session without the client key,
   while authorized WebSocket and port mappings remain functional. Do not claim NetworkPolicy
   source identity is preserved through SNAT.
6. Perform one final database/Kubernetes/source acceptance pass and update this checkpoint. Do not
   repeat completed migration, CI, observability, or terminology audits without new contrary
   evidence.

## Safety invariants

### Active cutover checkpoint — 2026-09-09

- User released all MWC workspaces for downtime; Coder remains excluded.
- Live control plane is still the pre-bridge `d0bba81` image. Before any mutation,
  its supported `database export` produced `/var/lib/mwc/pre-cutover-20260909-schema19.json`.
  An off-Pod copy is saved under the private workspace backup directory
  `.mwc-migration-backups/2026-09-09/pre-cutover-schema19.json`, mode 0600.
  Parsed metadata confirms schema 19, installation `k3si-7032544955`, 22 tables;
  SHA-256 `6cb27107deea070f1090703d66d9cdf9c3c7630592904dcc5d84acd2719ddad0`.
  Credential values remain encrypted as stored; the export is sensitive and not committed.
  This online preflight backup does not replace the final frozen-writer backup.
- All four MWC Longhorn volumes currently report attached/healthy, including game-forking.
- GitOps `61fbcfb` is pushed on current master and applied by `cluster-apps` (Synced/Succeeded).
  Main/routing/migration Applications all now have no `syncPolicy.automated`.
  The change removes only nine automation lines; no release/namespace change was mixed in.
- All four workspace API `stop` calls returned 202 with stable per-workspace idempotency
  keys `cutover-20260909-stop-<UUID>`. Readback confirms four StatefulSets replicas=0,
  no workspace Pods, and database states `stopped`; only the control-plane Pod remains.
  A second private off-Pod export `stopped-workspaces-schema19.json` captures this state.
  Private resource backups of the control namespace and four MWC namespaces are also saved.
- Four fresh Longhorn snapshots `mwc-freeze-<shortid>-20260909` are ready with empty errors:
  `bd2dc9ca6aa2b1b5`, `b268ff46894a14b9`, `97645a0fb4771b1d`, `b405d2441a1ee149`.
  Snapshot-controller temporarily attaches detached volumes itself; no workspace was started.
  Detached/unknown after completion is normal, not a health failure.
- GitOps `4586f23` promotes only the verified schema-20 bridge pins while preserving freeze.
  It is pushed/applied; a scoped manual sync of only the MWC Application (no prune) succeeded.
  Bridge image pulled successfully, but startup FAILED with `SchemaUpgradeDataInvalid`.
  Rollout waiter ended with timeout. Main scaled only the frozen control StatefulSet to zero
  to stop crash retries; workspace replicas remain zero. Do not advance to schema 22 or move
  any PV. Diagnose bridge validation against the private stopped-state export; snapshots and
  original claims are intact. No upgrade success is claimed.
- Root cause: schema-19 export omits idempotency records, so its earlier preflight missed stale
  replay payloads. A temporary read-only control-PVC reader was created while all writers were
  stopped, and the SQLite/WAL/SHM files were copied to private `frozen-sqlite/` (0600).
  Actual DB still reports schema 19. Historical replay payloads lack `pod_requests` or contain
  historical environment values; 369/373 idempotency entries are expired, with four new stop
  responses remaining. The bridge currently validates expired records and rejects the upgrade.
  Repair is assigned on an isolated schema-20 bridge branch with expiry/atomicity regressions.
  No production SQLite edits are authorized or performed. The read-only helper Pod was deleted.
- GitOps rollback `f7b85dd` is pushed, preserving other operators' latest `e938e3c` changes
  and all three auto-sync freezes. It restores the pre-bridge release to recover the console
  while the repaired bridge is built in CI. Scoped rollback sync and StatefulSet rollout
  succeeded; authenticated API readback returns all four workspaces `stopped`.
  The console is recovered, no workspace was restarted, and the auto-sync freezes remain.
- Repair `49cea2f` on isolated `bridge-v20-expired-idempotency` is reviewed and pushed.
  CI `34311058524` PASSED verify and all four image publications. Main checked the three
  expiry/rejection/atomic-rollback regression tests explicitly passed in the CI log.
  Publication permits only main and this exact repair branch, still gated by successful verify.
  The four unexpired stop responses pass the known structural checks; faulty historical
  replay entries are expired. Do not merge the temporary bridge into final schema-22 main.
- Main fetched public `sha-49cea2f` OCI indexes and verified body SHA-256 equals registry
  `Docker-Content-Digest`, without downloading layers:
  control `sha256:1e4afb28d960593b59b1b4c851d4f5d11c4bd840d289f78bf441b971a87d1763`;
  ttyd `sha256:bb2b7980deabf374b2ed328aa3061b7ba5c3239bcf1dec6bdfa0e8f6eb5d9e69`;
  workspace `sha256:229bd5214a2c6fdf313f75eba02696e3bc1e79f6d72601d15590a9031b3ebfb1`;
  jump `sha256:b080fc426824d8b5e8e90abe67450baf055776c3c5cc402b11b4073320403011`.
  These are temporary bridge images, not replacements for the already verified final
  schema-22 release or the published Harbor development images.
- Four-workspace PV rebind artifacts landed in `64846af`. Main corrected their delete gate:
  a retained PV becomes `Released` but keeps `claimRef`; wait for PVC absence/Released,
  then guarded explicit claimRef removal, not automatic claimRef disappearance.
- Source review found that product `database import` supports PostgreSQL only. Do not attempt
  a JSON snapshot import into a target SQLite database as the earlier runbook implied.
  A direct retained control-PV rebind is under specific technical review (node/path/UID and
  rollback); no control-PV mutation has occurred.

### Canonical cutover execution — 2026-09-09

- Repaired bridge deployed via GitOps `ca268b6`; rollout passed. Export confirms schema 20
  and four stopped workspaces. New bridge/image-update reconcile jobs drained; three older
  exhausted pending jobs remain (not evidence of a new bridge failure).
- Audited image-update API upgraded maintainance and rust-dev-test to the previously verified
  Harbor 20260907 development images, with generation 7→8 and stopped state preserved.
- Stopped the bridge writer and copied complete SQLite/WAL/SHM to private `frozen-schema20/`.
  Read-only integrity check returned `ok`, schema 20 and four stopped workspace rows.
- Offline Job `mwc-offline-schema22` using verified final `b5b946a` image completed with
  `database schema version 22`. No API/coordinator was started by the job.
  A private `frozen-schema22/` copy independently passed integrity/schema/workspace-count checks.
  Completed Job and temporary read-only helper Pod were removed before volume rebinding.
- Created exact namespace `memeloop-workspace-control` and replicated four required global
  Secrets without printing values (daily tokens, encryption, internal auth, wildcard TLS).
- Guarded `rebind-one.ts` moved the control PV and all four workspace PVs one at a time.
  Server dry-run validates target namespace/name/PV/storage/identity before any deletion;
  PVC deletion uses UID+resourceVersion preconditions, PV patches use claim/RV tests.
  Each source PVC object was removed only after Retain readback; no PV/data was deleted.
  All five target claims are Bound to their original PVs and retained for rollback.
  New PVC UIDs: control `dded5eb4-108f-4203-a5c5-793f9c01df61`;
  maintainance `20d0cc60-056a-45f3-873b-6fc937d06ea8`;
  tiddlywiki `dcc8e761-e7bd-46a0-af3a-0c89135cf082`;
  game `bccf3bde-8969-4be8-bba2-5670003d8b72`;
  rust `420d58ac-1c17-4feb-9fba-359055608ef4`.
- The strict label guard caught a rust UUID typo before any mutation of that claim.
  Corrected against live identity, then its rebind passed. This was not a volume/data failure.
- Transferred exactly four SSH NodePort Services into the target namespace, preserving
  ports 31871/30953/30732/32671. Old exact Services were backed up and removed to release
  cluster-wide port allocations; no other source resources were deleted.
- GitOps `fdc145f` is pushed, preserving other operators' `8685488` changes.
  It applies the reviewed final schema-22 pins, canonical namespace and sqlite.existingClaim;
  Helm render confirms no dynamic control PVC template. Auto-sync remains frozen.
  Scoped target-control rollout PASSED on westlake. Routing App synced to target; removed
  only the backed-up old console Ingress after target Ingress existed to avoid duplicate-host
  routing. Original user token authenticated successfully through the normal console URL.
  Never restart the old schema-20 controller over this DB.
- Started tiddlywiki first through API (202), target StatefulSet rollout passed and SSH
  NodePort remains 30953. Then sent individual start requests (202) for the other three.
  Their readiness plus four-workspace SSH/Web Shell/data acceptance remain pending.
- Post-cutover: all five Pods reached container readiness. Strict host-key-checked SSH
  commands passed on all four preserved NodePorts as `user`; public host keys exactly match
  the pre-cutover backed-up identity Secrets, and `/home/user/.codex/sessions` exists in each.
- Initial browser acceptance returned 503 because four old source Web Shell Ingresses
  duplicated the canonical paths. Verified ownership/path and stopped old StatefulSets,
  then removed only those four backed-up duplicate Ingresses. Browser acceptance now
  PASSES for tiddlywiki/game (real shell I/O, PTY resize, consumed-ticket rejection, fresh ticket).
- Maintainance/rust image pulls exceeded the worker's ten-error retry budget: Pods became
  Ready and SSH worked, but API readiness stayed stale. This is a product defect, not a
  storage/attachment failure. Worker fix assigned to `http_fixture_clocks` (normal readiness
  waiting must not exhaust true-error budget). Sent audited restart recovery requests (202)
  for just these two with cached images. Recovery succeeded: browser session `43310`
  exited 0 for maintainance and rust (real terminal I/O, PTY resize, replay rejection,
  fresh-ticket recovery). All four post-cutover browser checks now PASS. Temporary token
  files and browser contexts were cleaned; no token/ticket/trace output was saved.
- GitOps `4af2f89` updates the app README's Bash and PowerShell credential commands to
  canonical namespace `memeloop-workspace-control`, preserving other operators' `f6bbf03`.
  Remaining work includes the readiness-retry product fix and CI/release, old source resource
  closeout, image-policy cleanup, final external Coder handoff, and sandbox rollout gates.
- Readiness fix `2ea34fa` is pushed to main; CI `34320677127` ended FAILED at
  `run_once` cognitive complexity 33/25. Assigned extraction of result persistence and
  >10-pending recovery regressions; do not restart that finished CI run or weaken lint.
  Structured Pending outcomes and workspace-lease contention restore only the current claim's
  attempt, leaving genuine failure budget intact. Further >10-wait/recovery/lease tests assigned.
- Confirmed four current workspaces and current template responses do not reference the two
  20260902 development images. Disabled exactly those old cluster-admin/rust-dev policies
  through audited, idempotent API calls; no registry image or volume was deleted.
- Old-source closeout inventory confirms five old namespaces have no Pods/PVCs and all
  old StatefulSets are zero. Argo still tracks old control resources; keep auto-sync paused
  until exact cleanup is reviewed. Historical Released/Retain PV
  `pvc-b4facfbe-fa14-4616-99dd-37e5484a2c41` is not one of the five moved volumes;
  do not delete it as part of current volume cleanup.
- External-agent copy/paste handoff updated in `9e34202`: only the remaining Coder workspace,
  explicitly forbidding a repeat of the completed four-MWC/database migration. Includes
  client-connection access limits, preserved session/state boundaries and exact source PV.
- Source closeout resumed after verified production schema 22: `key_template_permissions`
  owns removal of temporary schema20/schema21 upgrade modules and their obsolete tests.
  Retain fresh-22 initialization, existing-22 validation and rejection of unsupported schemas.
  This is code-only cleanup; no further database migration is needed or authorized by it.
  `http_fixture_clocks` independently owns worker complexity refactor and extended retry tests.
- `c6505c7` extracts job-result persistence without weakening lint and adds 12 pending waits
  followed by exactly-once completion, real-failure-budget preservation, and wrong-owner tests.
  `c00ce94` deletes schema20/schema21 modules and conversion branches (452 net lines removed).
  Both are pushed; fresh CI `34323729617` runs on `c00ce94`. Additional isolated PostgreSQL
  and SQLite unsupported-version boundary coverage is assigned; no local Rust builds.
- CI `34323729617` verify PASSED; control-image publication was still in progress at the last
  observation, with the other three image jobs successful. Do not claim full publication yet.
- Boundary tests `a9bb4e4` are reviewed and pushed; CI `34324256107` is running.
  SQLite rejects 20/21/23 without conversion. PostgreSQL uses independent generated schemas
  for fresh/current22 and 20/21/23 rejection; CI supplies MWC_TEST_POSTGRES_URL.
  Wait for this newest complete verification/publication before selecting the final rollout pins.
- CI `34324256107` verify PASSED; three auxiliary image jobs succeeded and control-image
  publication remains active. Watch session `51213` follows this exact run; do not start
  another watcher or rebuild just because an observation yields.
- Post-cutover monitoring readback: Prometheus reports
  `up{namespace="memeloop-workspace-control"}` = 1 for the control-plane internal service.
  Canonical ServiceMonitor selects the internal auth-port `/metrics`; all four workspace
  Pods' containers are Ready. This proves scrape continuity, not full sandbox acceptance.
- CI `34324256107` completed SUCCESS, including all four image publications.
  Verified GHCR `sha-a9bb4e4` control OCI bytes against the registry digest:
  `sha256:faf59c80e5896a308814cd5df81d3f5f69d1a771c83be6e32627064d23bab2fc`.
  GitOps `eb05819` changes only control source revision and control image digest;
  existing workspace images and ttyd pins remain unchanged. First push encountered a
  transient Forgejo internal error; retry succeeded. Parent applied the new source;
  requested only the MWC child sync at a9bb4e4, with prune disabled. Rollout pending.
- Loki query_range over six hours returns a stream for namespace
  `memeloop-workspace-control`; only stream/count metadata was inspected, not log contents.
- User reconfirmed no MWC workspaces are in use. Exact old-namespace closeout review
  delegated read-only; final Coder workspace remains excluded from in-environment migration.
- GitOps child sync at a9bb4e4 SUCCEEDED; control rollout completed and the running
  imageID exactly matches the verified digest above. `/livez` and `/readyz` both return
  `{"status":"ok"}`. Original owner token returns HTTP 200 from `/api/v1/me` and
  each of the four workspace detail endpoints, all `ready`. An initial list request
  without required query parameters returned 400; this was not an authentication failure.
  No workspace Pod restarted during this control-only update.
- Old-source closeout COMPLETED: independently reviewed absence of Pods/PVCs, zero
  StatefulSets, no cross-namespace RBAC/webhook dependencies, and canonical route targets.
  Verified private source manifests exist, then deleted exactly the old control namespace
  `mwc-k3si-7032544955` and four `ws-k3si-7032544955-<workspace-short-id>` namespaces
  listed in the completed migration inventory. `kubectl wait --for=delete` succeeded.
  No PV, snapshot, backup or Coder resource was deleted. Recovery uses retained manifests
  and volumes; source namespaces can be recreated if a separately reviewed rollback needs them.
- GitOps `4c2221e` moves the wildcard-certificate renewal destination from the old control
  namespace to `memeloop-workspace-control`; acme reports Synced at that commit.
  Main and routing Apps now both report Synced/Healthy after old namespace retirement.
  GitOps `2343f92` restores their original automated prune/self-heal policies against
  target-only desired state. The migration Application remains paused for final Coder handoff.
  All five canonical PVCs remain Bound to the original PVs; all five Pods remain Ready.
- Closeout scope is the five obsolete namespaces, not a claim that every remaining ledger item
  is complete. External Coder migration and protected sandbox/mTLS/gVisor rollout gates remain;
  installation identity and recovery artifacts must not be blindly renamed or removed.
- 2026-09-09 continuation: prior turn classified as progress (release and namespace retirement).
  Main/routing Apps read back Synced/Healthy with automated prune/self-heal restored.
  Updated the resume table itself, not only its historical appendix, to avoid stale pending rows.
- SEC-05 production runner `scripts/live-key-boundary-acceptance.ts` passed with temporary
  ten-minute keys and only read/mint/revoke requests. No workspace/template/injection was changed.
  All temporary keys were revoked and independently returned401. No credentials were logged.
- UI-01 production screenshots at
  `/home/token-center-dev/.codex/visualizations/2026/08/25/01a03b21-c07f-7713-864f-f29b10d74a6f/mwc-ui01-production`
  cover workspaces/settings at360/768/1440. No double scrollbars observed; 360 right-edge
  clipping and light-theme metadata contrast need a bounded fix assigned to the same reviewer.
- mTLS certificate preparation found cert-manager available but no general trust distribution
  controller. Current single-Secret leaf/trust layout conflicts with cert-manager's ca.crt.
  `ttyd_independent_client_trust` owns separate projected client-trust Secret and four-value
  all-or-none validation. `mtls_certificate_rollout` owns native cert-manager leaf manifests
  and stable operator-managed CA/overlapping-root-rotation procedure. No custom CA-sync controller,
  node OS upgrade or production mTLS enablement has been performed.
- `815ff90` implements independent client-trust projection and all16 env-presence tests;
  Helm lint/full/partial rendering checks pass, full Rust tests remain CI-only.
  `8f82fdf`/`ef01dfd` prepare native cert-manager leaves plus a measured-label ServiceMonitor
  and scoped renewal/Ready/expiry alerts. They are not yet deployed.
- `5796c4f` provides one-time, explicit-context CA bootstrap that refuses existing Secrets.
  Ran preflight then apply on default: created two distinct root CA Secrets and two opposite
  public-only trust Secrets in canonical/Higress namespaces. In-memory comparison confirms
  each trust matches the correct opposite root and contains no private key. No leaf, ingress,
  runtime setting or workload changed. Local private temporary files were removed.
  Stable root-CA automatic expiry monitoring remains an explicit gap; leaf alerts are prepared.

- Borrowing restriction for ports `31871` and `32671` was lifted by the user on 2026-09-09;
  preserve the normal snapshot, writer-freeze, volume-binding and rollback gates.
- Never expose tokens, private keys, decrypted credentials, certificates, or database snapshots.
- Workspace image changes require the stopped-state, exact-digest, policy, generation,
  idempotency, audit, and reconcile gates; do not edit SQLite directly.
- Do not delete a retained target PVC or pre-start snapshot during source closeout.
