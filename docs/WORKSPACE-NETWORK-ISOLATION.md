# Workspace network isolation

## Configuration ownership

The template's `spec.egress_policy` selects `unrestricted` (the default) or `internet_only`.
The platform renders a NetworkPolicy selecting that workspace's own Pod labels, even when
all workspaces share `memeloop-workspace-control`. A namespace boundary is not required for
this selection, and a namespace alone is not proof of traffic isolation.

For external users, configure a reviewed template with `cluster_access: false`, bind the
automation API key to its template ID, and grant only the scopes that automation needs.
`runtime_class_name` selects an installed Kubernetes RuntimeClass; it is not an image name
and does not install a runtime on nodes. Node operators must complete the
[runtime acceptance procedure](SANDBOX-NODE-OPERATIONS.md) before advertising such a class.

Changing a stored template is not evidence that an existing workspace's captured template or
running Pod changed. Inspect the workspace's effective configuration and its rendered policy.
Do not replace the internal maintainance template with an external sandbox template.

## What `internet_only` renders

- DNS to the configured namespace and DNS Pod labels, on TCP and UDP 53.
- IPv4 internet destinations excluding private, loopback, link-local, shared Tailnet,
  reserved and documentation ranges.
- IPv6 global-unicast `2000::/3`, excluding the implementation's special-use ranges.
- Operator-specified additional blocked CIDRs in both address families.

Internet rules do not restrict destination ports. This is an IP connectivity policy, not an
HTTP filter, domain allowlist, protection against internet proxies, or a guarantee that cloud
metadata is unreachable through every provider-specific endpoint.

The Helm settings are:

```yaml
workspace:
  egress:
    dnsNamespace: kube-system
    dnsPodLabels:
      k8s-app: kube-dns
    additionalBlockedCidrs: []
```

Populate `additionalBlockedCidrs` with actual routable public node addresses and any
nonstandard cluster, service, management or provider-metadata ranges not covered by default.
Do not copy example IPs into production. Revisit this inventory when adding hybrid-cloud nodes:
a host's public address is not excluded merely because its Tailnet address is private.
Missing DNS identity fails configuration for an `internet_only` workspace.

### Hybrid-cloud address inventory

For the current cluster, Kubernetes Node status contains only internal addresses.
It is not a complete public-address inventory. Existing host-network
`prometheus-node-exporter` Pods in `monitoring` can expose the assigned host IPv6
addresses through a read-only `/proc/net/if_inet6` inspection. Include all assigned
global-unicast addresses, including deprecated addresses that remain assigned;
the default-route source address alone is insufficient.

That source does not reveal a router's public IPv4 NAT address. The existing
peer-relay endpoint synchronization on NAS obtains a NAT observation from
`tailscale netcheck`; it is not a central, continuously refreshed inventory for
all nodes. Do not install an additional privileged discovery Pod merely to
duplicate the available host-network inspection path.

The current additional CIDRs are a point-in-time inventory. Automatic address
updates remain required before treating rotating residential public addresses
as continuously protected. Updating a values file is also insufficient unless
existing workspace policies receive the new exclusions.

Public DNS queries also found current NAT IPv4 addresses for VT, the shared
Haixia/Sansheng uplink and NAS. These were added through GitOps and appeared in
the running formal gVisor workspace's policy. A UID-1000 TCP probe did not connect
to those three addresses on 443, while its public `1.1.1.1:443` control connected.
This is a finite connectivity sample, not proof of automatic address rotation.
The existing DDNS source names are recorded in the installation's GitOps
`apps/memeloop-workspace-control/NETWORK.md`; retain assigned IPv6 coverage in
addition to the DDNS preferred-address answers.

The control plane refreshes existing, database-backed workspace NetworkPolicies
at startup. It pages through installation-owned policies, skips deleting
workspaces, and patches only the policy specification. A concurrent deletion
does not recreate the policy; version conflicts re-read and recheck ownership.
This applies changed operator configuration after a control-plane rollout
without restarting workspace Pods.

The same refresh includes separately rendered port-mapping policies, matched
against their database records and mapping ownership labels. Live acceptance
closed a disposable mapping's ingress rules, rolled out the updated control
plane, and confirmed that port 3000 and its configured gateway sources were
restored without changing the workspace Pod UID.

Live acceptance temporarily removed the IPv6 allow rule from the disposable
memory workspace, making its policy stricter. The GitOps rollout restored the
configured rule and reported one refreshed policy. The workspace Pod UID stayed
unchanged. This proves startup refresh, not continuous public-address discovery.

## Ingress and gateway trust

The platform applies per-workspace SSH and ttyd ingress rules. Public SSH uses the configured
OpenSSH jump host; internal SSH may use the existing NodePort path. These modes do not
implicitly turn on egress isolation.

In the tested Flannel/Tailscale deployment, cross-node SNAT can hide the original Pod source
from ingress selectors. Do not treat a broad CNI-gateway CIDR exception as an authenticated
Higress identity. [ttyd upstream mTLS](WEB-SHELL-MTLS.md) supplies a separate gateway identity;
the browser still needs external-auth and a single-use ticket.

## Acceptance gates

The controller creates policy before the workload. This orders Kubernetes API operations;
it does not guarantee that the node installs its rules before the first container instruction.
Each eligible node/network path needs measured acceptance:

1. From the first user process, repeatedly probe controlled cluster, node and metadata targets
   during startup. Use only harmless TCP/HTTP connection probes.
2. Check same-node and cross-node workspace, Service, node internal/public IP, API and kubelet
   paths; cover IPv6 if it is routable. Confirm DNS and a reachable public endpoint still work.
3. Repeat the startup checks after controlled Pod recreation and rescheduling to another
   eligible node. Record node identities, effective policies and failures, not only exit codes.
4. Verify gateway mTLS and external-auth together, including missing/untrusted client identity,
   invalid server trust, ticket replay and normal browser shell interaction.
5. Independently test runtime privilege and CPU/memory/PID/ephemeral-storage boundaries.
   NetworkPolicy cannot substitute for these controls.

## Recorded limited NetworkPolicy evidence

On 2026-09-09, disposable, restricted canaries on `haixia` and `westlake`
used a pre-existing egress default-deny policy.  A container's first Node
process made one harmless HTTP request to a controlled Pod IP and recorded its
result before the canary was Ready.  It was blocked both at initial placement
and after deletion/recreation on the other node.  The same startup/egress
sample passed once more between `haixia` and the worker
`iv-yeahgdnw8wwh2yppho5e`; that worker was selected because the existing probe
image was already cached.  The `haixia`/`westlake` two-node boundary matrix
also blocked TCP to the tested Kubernetes API, kubelet, node and Service
targets under the rendered `internet_only` rule while DNS and the configured
public TCP control continued to work.  Same-node and cross-node egress
`podSelector` positive controls passed.  This is sampled startup evidence
only: Kubernetes API ordering and a first-process probe cannot prove that a
CNI has no first-instruction enforcement window.

The matching cross-node ingress `podSelector` test still failed: traffic from
the remote Pod was observed at the target as the Flannel/CNI source
`10.42.4.1`, so its original Pod label was not available for ingress identity.
Do not widen a policy to treat that CNI address as an authenticated gateway.
The deployed workspace policy's `10.42.0.0/16` source is a reachability
compensation for this path, not a user or Higress identity; ttyd's separately
validated upstream mTLS is required before a shell is accessible.  Network
reachability and mTLS-authenticated access are distinct acceptance claims.

Application port mappings do not yet have that upstream mTLS boundary. A
disposable HTTP listener on the formal gVisor workspace returned 401 through
Higress without a session, and 200 after the mapping's one-use bootstrap URL
established a session. A direct request from the current cluster workspace to
the target Pod IP on port 3000 also returned the test content without gateway
authentication. The configured source-CIDR exception allowed that path.

This demonstrates a cluster-source bypass of application gateway authentication,
not an `internet_only` workspace escaping its outbound NetworkPolicy. Do not
claim that only authenticated Higress traffic can reach mapped application ports
until the upstream identity boundary is implemented. Keep the existing
MASQUERADE configuration: the cluster networking runbook records it as required
for cross-node routing. The test listener and mapping were removed afterward.

The implementation in progress uses standard Nginx in the existing ttyd image,
listening on the already-reserved port 8443 and reusing its mounted server
certificate and gateway client CA. A projected ConfigMap allowlists mapping
hostnames to loopback application ports. Higress must authenticate and validate
this upstream, and mapping NetworkPolicies must open only 8443, not the original
application port. Route edits must reload the proxy without replacing the
workspace Pod. The image component alone does not close the bypass: resource
coordination, gateway configuration, rollout and live negative tests are required
before declaring this boundary complete.

These observations do not establish every eligible node, IPv6, host or
metadata behavior, an actual external-tenant workspace lifecycle, or full
external-sandbox acceptance.  The production internal workspaces must not be
presented as accepted untrusted-user sandboxes on the basis of this matrix.
