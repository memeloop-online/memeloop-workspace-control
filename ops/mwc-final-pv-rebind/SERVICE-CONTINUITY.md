# MWC Service-continuity handoff

This is an ordered, per-Service handoff for the four stopped MWC workspaces.
It does not apply manifests.  The later independent Coder migration is not in
scope.  Source services must be backed up as metadata-only resources before
their NodePorts are released; do not export Secret values into a terminal,
Git, or this directory.

## Exact SSH NodePort transfer map

Each row has one live source NodePort Service.  Its name is deterministic and
must be used unchanged in `memeloop-workspace-control`.  The target Service
must have `type: NodePort`, `ports: [{name: ssh, protocol: TCP, port: 2222,
targetPort: 2222, nodePort: <listed>}]`, `publishNotReadyAddresses: true`, and
the exact labels and selector below.  `externalTrafficPolicy: Cluster` and
`sessionAffinity: None` are the observed source behavior.

| Workspace ID | Source exact Service | NodePort | Required target Service name |
| --- | --- | ---: | --- |
| `01a06170-582c-70f0-bd2d-c9ca6aa2b1b5` | `ws-k3si-7032544955-bd2dc9ca6aa2b1b5/w-bd2dc9ca6aa2b1b5-ssh` | 31871 | `w-bd2dc9ca6aa2b1b5-ssh` |
| `01a06174-8ce2-7d52-b268-ff46894a14b9` | `ws-k3si-7032544955-b268ff46894a14b9/w-b268ff46894a14b9-ssh` | 30953 | `w-b268ff46894a14b9-ssh` |
| `01a06180-f1c9-7a01-9764-5a0fb4771b1d` | `ws-k3si-7032544955-97645a0fb4771b1d/w-97645a0fb4771b1d-ssh` | 30732 | `w-97645a0fb4771b1d-ssh` |
| `01a06180-f652-7cd3-b405-d2441a1ee149` | `ws-k3si-7032544955-b405d2441a1ee149/w-b405d2441a1ee149-ssh` | 32671 | `w-b405d2441a1ee149-ssh` |

For each row, the common selector/ownership labels are
`app.kubernetes.io/component=workspace`,
`app.kubernetes.io/managed-by=memeloop-workspace-control`, and
`workspace.memeloop.dev/owner-installation=k3si-7032544955`, plus that row's
`workspace.memeloop.dev/workspace-id`.  The Service metadata also carries
`workspace.memeloop.dev/organization-id=01a0436a-8659-7250-9980-087f006d8f49`
and `workspace.memeloop.dev/owner-user-id=01a04369-e74f-7273-a5c6-77c5d61cb3c9`.

## Ordered transfer gate

Kubernetes allocates NodePorts cluster-wide: the old and target Services
cannot hold the same number at the same time.  While all source writers are
stopped:

1. Save an approved private, metadata-only backup of the one exact source
   Service and record its resourceVersion, nodePort, labels, selector, and
   port tuple.  Confirm no unrelated Service has the intended NodePort.
2. Delete **only that exact old NodePort Service** to release its number.  This
   is an intentional narrow exception to retaining old namespace resources:
   a stopped old Service cannot coexist with the target service on the same
   cluster-wide port.  Do not delete its namespace, ClusterIP service, PVC,
   StatefulSet, ConfigMap, or Secret.
3. Create the target Service in `memeloop-workspace-control` with the exact
   number and metadata above, then require the API to report that same
   `spec.ports[0].nodePort` before proceeding.  If allocation fails, stop;
   do not substitute a new number without an explicit service-continuity
   decision.
4. Only then permit schema-22 to reconcile the target workspace.  Its current
   builder intentionally omits `nodePort` and Kubernetes normally assigns it;
   the pre-created, matching target Service preserves the requested allocation
   while the coordinator owns/reconciles the rest of the workspace.  Verify it
   preserves the explicit port after reconciliation and that its selector has
   endpoints only after the intended target Pod starts.

The per-workspace primary ClusterIP Services (`w-<short-id>`, ports 2222 SSH
and 7681 web-shell) are namespace-local and have no NodePort conflict.  Let
the schema-22 coordinator regenerate them; do not preserve their cluster IPs.

## HTTP port mappings

Read-only inventory found no Service, Ingress, HTTPRoute, or NetworkPolicy
labelled `workspace.memeloop.dev/port-mapping-id` in any of these four source
namespaces.  Therefore there is no existing HTTP port-mapping Service to
transfer.  Product code defines mapping Services as **ClusterIP only**
(`src/kubernetes/port_mappings.rs`); they are not NodePorts.  If a mapping is
created before cutover, stop and add its exact Service/Ingress/NetworkPolicy
objects to the approved inventory; schema-22 must regenerate them from the
retained control-plane records rather than copying an old Pod configuration.

## Secret and ConfigMap continuity

Do not copy any old workspace StatefulSet, Pod, or its generated namespace
objects.  Schema-22 recreates the workspace ConfigMap, environment ConfigMap,
files ConfigMap, environment Secret, and files Secret from the retained
control-plane data and current template.  The exact source names follow each
`w-<short-id>` prefix with `-config`, `-environment-config`, `-files-config`,
`-environment-secret`, and `-files-secret`.

The per-workspace `w-<short-id>-ssh-identity` Secret (keys
`ssh_host_ed25519_key` and `.pub`) must also be regenerated, not copied.  The
private identity is stored encrypted in the control-plane database; keeping
the same installation ID and encryption key lets schema-22 materialize the
same identity in the canonical namespace.  Before the target coordinator
starts, provide its required global Secret references in the target namespace:

| Source Secret (keys only) | Continuity requirement |
| --- | --- |
| `mwc-k3si-7032544955-encryption` (`encryption-key`) | Securely replicate the exact existing value and configure `secrets.encryptionSecretName`; otherwise encrypted SSH identities and injection material cannot be read. |
| `mwc-k3si-7032544955-internal-auth` (`internal-auth-token`) | Securely replicate the exact value and configure `secrets.internalAuthSecretName`; it protects the internal auth path. |
| `mwc-daily-user-tokens` (normal-user token keys) | Preserve through the established secret-management path if current console tokens must remain valid; never output values. |

TLS material such as `wildcard-onetwo` remains centrally resolved by the
gateway configuration; do not copy it into workspace resources.  Verify all
Secret references exist in the canonical namespace before starting the only
target coordinator.
