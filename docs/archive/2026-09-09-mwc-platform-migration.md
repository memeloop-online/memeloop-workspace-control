# Archived record: completed MWC platform migration (2026-09-09)

This is historical evidence, not an executable runbook. It was removed from the active handoff to
prevent accidental repetition. The only active change is the external Coder handoff in
[FINAL-MIGRATION-RUNBOOK.md](../FINAL-MIGRATION-RUNBOOK.md).

## Accepted outcome

- Control plane and four MWC workspaces were migrated to exact namespace
  `memeloop-workspace-control`; the five source namespaces were deleted after acceptance.
- Installation ID was retained; production schema is 22 and the current release accepts schema 22.
- Four MWC volumes were direct-rebound without a durable-home copy. SSH ports and host keys remain.
- `ttydOpenSSL` image digest prefix `4cf08fd5` passed authenticated SSH and browser-terminal checks
  for all four workspaces.
- This phase excluded independent 100Gi Coder TOKEN center dev. It remains in `coder` and is the
  only active migration source.

## Retained evidence and recovery context

The completed change record retained Pod → PVC → PV chains, claim UIDs, VolumeAttachments,
Longhorn health/snapshots, nodes, source manifests, immutable release/CI provenance, SQLite/WAL
backup locations, target acceptance results and rollback materials. See `IMPLEMENTATION_STATUS.md`
for release digests and recovery references; never expose secret values or database contents.

Completed control SQLite PV: `pvc-317ea51a-86f8-4f1c-b2ed-2c111795c331` (10Gi, local-path,
`westlake` affinity). Completed MWC Longhorn PVs:

- `pvc-611ec96e-4bd9-4162-b237-dab2b68d3694`
- `pvc-ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96`
- `pvc-bf5e159e-bf80-47ae-bda3-58affce88008`
- `pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e`

## Operations permanently retired from the active handoff

Do not rerun historic schema 19→20 bridge, offline 20→22 migration, namespace creation/deletion,
old control-plane rebind, old MWC workload rebind, routing bridge, borrowed-port release, or
four-workspace freeze/acceptance sequence. Those operations belong to the accepted migration and
would endanger the target installation.

The complete former runbook remains in repository history for forensic review. This archive keeps
the operational facts and evidence pointers while intentionally excluding copy/paste destructive
steps from the current handoff surface.
