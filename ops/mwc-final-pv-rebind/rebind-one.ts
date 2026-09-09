// One explicitly selected claim per invocation. Never deletes a PV or Namespace.
import { execFileSync, spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const targets: Record<string, [string, string, string, string]> = {
  "control-plane": ["mwc-k3si-7032544955", "data-mwc-k3si-7032544955-0",
    "pvc-317ea51a-86f8-4f1c-b2ed-2c111795c331", "mwc-k3si-7032544955"],
  maintainance: ["ws-k3si-7032544955-bd2dc9ca6aa2b1b5", "workspace-data-w-bd2dc9ca6aa2b1b5-0",
    "pvc-611ec96e-4bd9-4162-b237-dab2b68d3694", "w-bd2dc9ca6aa2b1b5"],
  "tiddlywiki-dev": ["ws-k3si-7032544955-b268ff46894a14b9", "workspace-data-w-b268ff46894a14b9-0",
    "pvc-ae9f3ccf-458d-4f3a-a1a2-97c5e2f84c96", "w-b268ff46894a14b9"],
  "game-forking": ["ws-k3si-7032544955-97645a0fb4771b1d", "workspace-data-w-97645a0fb4771b1d-0",
    "pvc-bf5e159e-bf80-47ae-bda3-58affce88008", "w-97645a0fb4771b1d"],
  "rust-dev-test": ["ws-k3si-7032544955-b405d2441a1ee149", "workspace-data-w-b405d2441a1ee149-0",
    "pvc-fe7130d7-18a2-470e-824e-a0dea84e9a5e", "w-b405d2441a1ee149"],
};
const name = process.argv[2];
if (process.argv.length !== 3 || !targets[name]) throw Error("Select exactly one documented claim");
const [source, claim, pvName, statefulSet] = targets[name];
const destination = "memeloop-workspace-control";
function kubectl(args: string[]) {
  return execFileSync("kubectl", args, { encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });
}
function get(kind: string, object: string, namespace?: string) {
  return JSON.parse(kubectl([...(namespace ? ["-n", namespace] : []), "get", kind, object, "-o", "json"]));
}
function requireCondition(value: unknown, reason: string): asserts value {
  if (!value) throw Error(reason);
}
function noClaimPods(namespace: string) {
  const pods = JSON.parse(kubectl(["-n", namespace, "get", "pods", "-o", "json"])).items;
  requireCondition(!pods.some((p: any) => p.spec.volumes?.some(
    (v: any) => v.persistentVolumeClaim?.claimName === claim)), "Claim still has a Pod");
}
for (const app of ["memeloop-workspace-control", "memeloop-workspace-control-routing",
  "memeloop-workspace-control-migration"]) {
  requireCondition(!get("application", app, "argocd").spec.syncPolicy?.automated, "GitOps is not frozen");
}
requireCondition(get("statefulset", "mwc-k3si-7032544955", "mwc-k3si-7032544955").spec.replicas === 0,
  "Old coordinator is not stopped");
requireCondition(get("statefulset", statefulSet, source).spec.replicas === 0, "Source is not stopped");
noClaimPods(source);
noClaimPods(destination);
for (const kind of ["statefulsets", "deployments"]) {
  const controllers = JSON.parse(kubectl(["-n", destination, "get", kind, "-o", "json"])).items;
  requireCondition(controllers.every((c: any) => c.spec.replicas === 0), "Target controller is active");
}
requireCondition(!kubectl(["-n", destination, "get", "pvc", claim, "--ignore-not-found", "-o", "name"]).trim(),
  "Target claim already exists");
requireCondition(get("namespace", destination).metadata.labels?.["workspace.memeloop.dev/owner-installation"]
  === "k3si-7032544955", "Target namespace ownership mismatch");
const pvc = get("pvc", claim, source);
let pv = get("pv", pvName);
const uid = pvc.metadata.uid;
const manifest = fileURLToPath(new URL(`pvc-${name}.yaml`, import.meta.url));
const targetManifest = JSON.parse(kubectl(["create", "--dry-run=server", "-f", manifest, "-o", "json"]));
requireCondition(targetManifest.kind === "PersistentVolumeClaim"
  && targetManifest.metadata.name === claim && targetManifest.metadata.namespace === destination
  && targetManifest.spec.volumeName === pvName
  && targetManifest.spec.storageClassName === pvc.spec.storageClassName
  && targetManifest.spec.volumeMode === pvc.spec.volumeMode
  && JSON.stringify(targetManifest.spec.accessModes) === JSON.stringify(pvc.spec.accessModes)
  && targetManifest.spec.resources.requests.storage === pvc.spec.resources.requests.storage
  && targetManifest.metadata.labels?.["workspace.memeloop.dev/owner-installation"] === "k3si-7032544955",
  "Target manifest does not match the documented source");
if (name !== "control-plane") {
  for (const key of ["workspace-id", "owner-user-id", "organization-id"]) {
    const label = `workspace.memeloop.dev/${key}`;
    requireCondition(pvc.metadata.labels?.[label]
      && targetManifest.metadata.labels?.[label] === pvc.metadata.labels[label], "Target identity label mismatch");
  }
}
const targetCreate = {
  apiVersion: "v1", kind: "PersistentVolumeClaim",
  metadata: { name: claim, namespace: destination, labels: targetManifest.metadata.labels },
  spec: targetManifest.spec,
};
requireCondition(pvc.status.phase === "Bound" && pvc.spec.volumeName === pvName, "Source PVC mismatch");
requireCondition(pv.spec.claimRef?.uid === uid && pv.spec.claimRef?.namespace === source
  && pv.spec.claimRef?.name === claim, "Source PV claimRef mismatch");
const attachments = JSON.parse(kubectl(["get", "volumeattachments", "-o", "json"])).items;
requireCondition(!attachments.some((a: any) => a.spec.source.persistentVolumeName === pvName),
  "VolumeAttachment still exists");
if (name !== "control-plane") {
  requireCondition(get("volumes.longhorn.io", pv.spec.csi.volumeHandle, "longhorn-system").status.state
    === "detached", "Longhorn volume is not detached");
  const shortId = statefulSet.slice(2);
  const snapshot = get("snapshots.longhorn.io", `mwc-freeze-${shortId}-20260909`, "longhorn-system");
  requireCondition(snapshot.spec.volume === pv.spec.csi.volumeHandle && snapshot.status.readyToUse
    && !snapshot.status.error, "Fresh snapshot gate failed");
}
function claimTests(current: any) {
  return [
    { op: "test", path: "/metadata/resourceVersion", value: current.metadata.resourceVersion },
    { op: "test", path: "/spec/claimRef/namespace", value: source },
    { op: "test", path: "/spec/claimRef/name", value: claim },
    { op: "test", path: "/spec/claimRef/uid", value: uid },
  ];
}
requireCondition(pv.spec.persistentVolumeReclaimPolicy === "Delete"
  || pv.spec.persistentVolumeReclaimPolicy === "Retain", "Unexpected reclaim policy");
kubectl(["patch", "pv", pvName, "--type=json", "-p", JSON.stringify([
  ...claimTests(pv),
  { op: "test", path: "/spec/persistentVolumeReclaimPolicy", value: pv.spec.persistentVolumeReclaimPolicy },
  { op: "replace", path: "/spec/persistentVolumeReclaimPolicy", value: "Retain" },
])]);
requireCondition(get("pv", pvName).spec.persistentVolumeReclaimPolicy === "Retain", "Retain readback failed");

// Local-only authenticated Kubernetes transport; no credentials are read or printed.
const proxy = spawn("kubectl", ["proxy", "--address=127.0.0.1", "--port=0"], {
  stdio: ["ignore", "pipe", "pipe"],
});
try {
  const base = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(Error("Local transport timed out")), 10000);
    proxy.once("error", () => { clearTimeout(timer); reject(Error("Local transport failed")); });
    proxy.once("exit", () => { clearTimeout(timer); reject(Error("Local transport exited")); });
    proxy.stdout.on("data", (chunk: Buffer) => {
      const match = chunk.toString().match(/127\.0\.0\.1:(\d+)/);
      if (match) { clearTimeout(timer); resolve(`http://127.0.0.1:${match[1]}`); }
    });
  });
  const response = await fetch(`${base}/api/v1/namespaces/${source}/persistentvolumeclaims/${claim}`, {
    method: "DELETE", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ apiVersion: "v1", kind: "DeleteOptions", preconditions: {
      uid, resourceVersion: pvc.metadata.resourceVersion,
    } }), signal: AbortSignal.timeout(15000),
  });
  requireCondition(response.ok, `UID-guarded PVC deletion failed: HTTP ${response.status}`);
} finally {
  proxy.kill("SIGTERM");
}
kubectl(["-n", source, "wait", "--for=delete", `pvc/${claim}`, "--timeout=50s"]);
pv = get("pv", pvName);
requireCondition(pv.status.phase === "Released", "Retained PV is not Released");
kubectl(["patch", "pv", pvName, "--type=json", "-p", JSON.stringify([
  ...claimTests(pv),
  { op: "test", path: "/spec/persistentVolumeReclaimPolicy", value: "Retain" },
  { op: "remove", path: "/spec/claimRef" },
])]);
execFileSync("kubectl", ["create", "-f", "-"], {
  input: JSON.stringify(targetCreate), stdio: ["pipe", "pipe", "pipe"],
});
for (let attempt = 0; attempt < 30; attempt++) {
  const target = get("pvc", claim, destination);
  pv = get("pv", pvName);
  if (target.status.phase === "Bound" && pv.status.phase === "Bound") {
    requireCondition(target.spec.volumeName === pvName && pv.spec.claimRef.namespace === destination
      && pv.spec.claimRef.name === claim && pv.spec.claimRef.uid === target.metadata.uid,
    "Target bidirectional binding mismatch");
    console.log(JSON.stringify({ workspace: name, pv: pvName, namespace: destination,
      claim, newUid: target.metadata.uid, phase: "Bound", reclaimPolicy: pv.spec.persistentVolumeReclaimPolicy }));
    process.exit(0);
  }
  await delay(1000);
}
throw Error("Target binding timeout; retained PV and snapshot must not be deleted");
