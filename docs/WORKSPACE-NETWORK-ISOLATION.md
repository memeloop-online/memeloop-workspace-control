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

The 2026-09-08 egress/boundary matrices passed the paths recorded in
`IMPLEMENTATION_STATUS.md`. They do not establish startup enforcement, all-node coverage,
or full external-tenant acceptance. The production internal workspaces must not be presented
as accepted untrusted-user sandboxes until those remaining gates pass.
