# gVisor sandbox node operations

This runbook prepares a K3s node to provide the **optional** `runsc` containerd handler. It does
not change K3s' default runtime, create a `RuntimeClass`, label a Kubernetes Node, or make a
workload use gVisor. Those are separate reviewed changes. gVisor increases isolation but does not
make a workload risk-free and can reduce Linux syscall, networking, filesystem, observability and
device compatibility.

## Current admission position (2026-09-08)

The observed cluster has seven Ready amd64 nodes, all using K3s `v1.36.2+k3s1` with containerd
`2.3.2-k3s2`. They therefore use the K3s v3 template path below. This is an observation, not a
promise that every node is suitable.

`serv-146231` runs kernel `4.18.0-553.139.1.el8_10.x86_64`. It is below the currently adopted
gVisor minimum kernel `5.6` and is **excluded**: do not install gVisor there and do not add
`sandbox.memeloop.dev/gvisor-ready=true`. It is also an overseas edge/control-plane node.

`iv-yeahgdnw8wwh2yppho5e` is the only presently identified non-control-plane, non-NAS candidate;
it has kernel 5.15 and about 4 CPU / 3.8 GiB allocatable capacity. It is not automatically
approved: it is schedulable and has existing system workloads, so a maintenance/capacity review is
required. Do not disrupt control-plane nodes, active workspace nodes, GPU nodes, the NAS, or the
overseas edge merely to make a sandbox pool.

The user subsequently selected `100.64.0.10` for testing and authorized direct SSH operations.
Its Kubernetes name is `serv-146231`; its host name is `serv.146231.com`. Root SSH is now
verified. Kernel upgrade preparation is in progress; this authorization does not make the
current 4.18 kernel eligible. Keep its existing bootable kernel until the replacement kernel,
storage/network drivers and node rejoin have passed validation.

An approved read-only host-mount inspection on iv found a cgroup v2 filesystem and a generated
containerd `config.toml` at version 3. Its listed handlers are the existing `runc` and
`runhcs-wcow-process`; no `runsc` handler or custom v3 template is present. Its K3s config
explicitly uses `flannel-iface: tailscale0`. No `default_runtime_name`, `sandbox_image`, or
`disable-network-policy` line was present in the limited whitelist output; absence from that
output is not proof of an effective runtime or network-policy setting, so validate those behavior
paths with the canary.

The first one-shot read-only Job could not start because iv had no local pause image and its
existing image path failed before the Job command ran: `HEAD
https://harbor.k3s.onetwo.website/v2/docker-io/rancher/mirrored-pause/manifests/3.6?ns=docker.io`
returned `502 Bad Gateway`. A later read-only `k3s ctr -n k8s.io images ls` check found no local
pause image. Treat restoration of the correct pinned pause image/cache as a separate approved
node-reliability action; do not hide this condition by repeatedly scheduling diagnostic Pods or by
changing the default runtime. The separately assigned operations agent subsequently restored
Harbor, and a CRI pause-image pull succeeded. That image-pull prerequisite is now resolved.

Labels are an assertion about the **measured host**. Add `sandbox.memeloop.dev/gvisor-ready=true`
only after the preflight, archive checksum, K3s restart and an actual `runsc` canary on that exact
node all succeed. Remove the label before the handler is removed. Never infer it from OS type,
kernel family, an old inventory, or another node's result.

## Node preparation

Use a verified direct node SSH or console recovery path. Do not use a privileged diagnostic Pod,
another operational host, or an unreviewed jump path. The controller workstation used during the
initial investigation could read the Kubernetes API but had no usable BatchMode SSH identity for
the seven documented Tailnet node addresses; that is not evidence that SSH is broken.

Copy the scripts to the explicitly approved node and first run the read-only check as root:

```bash
sudo ./gvisor-node-preflight.sh
```

It checks K3s service state, architecture, embedded containerd major version, cgroup mount type,
kernel, the required v3 template location, and only warns for host-policy facts that need canary
evidence. Keep its output with the change record. It intentionally does not read registry files,
secrets, or alter the node.

For containerd 2.x, K3s renders `/var/lib/rancher/k3s/agent/etc/containerd/config.toml` from
`config-v3.toml.tmpl`; K3s recommends extending the `base` template rather than copying rendered
configuration. The installer appends this handler and leaves runc as default:

```toml
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc]
  runtime_type = "io.containerd.runsc.v1"
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc.options]
  TypeUrl = "io.containerd.runsc.v1.options"
```

Obtain a pinned gVisor release archive and independently verified SHA-256 from the official
release. The archive must include `runsc`, `containerd-shim-runsc-v1`, and the adjacent
`gvisor-bin/` directory. The installer refuses a missing checksum, host-name mismatch, absent
sidecars, an existing runsc stanza, or an implicit restart:

```bash
sudo ./gvisor-node-install.sh \
  --node "$(hostname)" --archive /path/to/gvisor-archive.tar.* \
  --sha256 '<verified-64-hex-sha256>' --apply --restart-k3s
```

The restart is real operational impact: K3s/containerd is restarted on that node to render and
load the handler. It can interrupt local Pods and image operations. Do it only after the node is
approved, capacity is reviewed, and a recovery path is verified; it is never appropriate as a
blind all-node rollout. Existing Pods continue using their created runtime, but workload movement
or restart can expose capacity/compatibility problems.

## RuntimeClass, canary, and overhead

After a successful single-node handler verification, GitOps may review a separate RuntimeClass.
This is illustrative only; it is not a live manifest and must not be applied until the label is
measured on at least one node:

```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: gvisor
handler: runsc
scheduling:
  nodeSelector:
    sandbox.memeloop.dev/gvisor-ready: "true"
```

Kubernetes merges this selector with a Pod's selector, so conflicts reject admission. Start with
a disposable, non-privileged canary that explicitly sets `runtimeClassName: gvisor`, is pinned to
the approved node, and exercises the workspace's actual network, volume, process, signal and
observability paths. Do not test GPU, `hostNetwork`, privileged containers, host devices,
hostPath, or workloads that require unsupported kernel interfaces as the first canary.

gVisor consumes host resources beyond application requests. Measure steady and peak CPU, memory,
PID, ephemeral-storage and startup latency on the exact image/profile before declaring
`RuntimeClass.overhead.podFixed`; do not guess a universal number. Until that measurement exists,
avoid overcommitting the candidate and treat scheduler accounting as incomplete. Kubernetes
accounts a declared overhead in scheduling and Pod cgroups, but a made-up value is worse than an
explicitly documented capacity reservation.

## Acceptance, rollback, and updates

Acceptance evidence for one node is: preflight output; archive source and checksum; rendered
`config.toml` containing the `runsc` handler; `k3s`/`k3s-agent` active after the approved restart;
one canary scheduled only to the labelled node; canary logs and functional tests; and a comparison
against the normal runtime. Include failures, not just success output.

Rollback the workload first: remove `runtimeClassName` (or scale down/delete the disposable
canary) and wait until no Pods use `gvisor`. Remove the `gvisor-ready` label so no new sandbox Pods
schedule there. In a maintenance window, restore the timestamped template backup under
`/var/lib/rancher/k3s/agent/etc/containerd/gvisor-backups/`, remove the handler/binaries only
after confirming no use, then restart only that node's K3s service and verify it returns Ready.
Do not delete the RuntimeClass while workloads still reference it; that can make later Pod
creation fail. A rollback can still interrupt local workloads, so it is not zero-risk.

Update gVisor one node at a time with the same checksum, preflight, canary and rollback gates.
Reassess kernel and cgroup state after K3s/OS upgrades. Reference material: [K3s advanced
containerd configuration](https://docs.k3s.io/advanced), [gVisor installation](https://gvisor.dev/docs/user_guide/install/),
[gVisor containerd configuration](https://gvisor.dev/docs/user_guide/containerd/configuration/),
and [Kubernetes RuntimeClass](https://kubernetes.io/docs/concepts/containers/runtime-class/).
