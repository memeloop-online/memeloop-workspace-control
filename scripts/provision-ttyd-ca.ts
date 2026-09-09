import { execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// One-time bootstrap, not a renewal controller. Existing material is never replaced.
const args = process.argv.slice(2);
const apply = args.includes("--apply");
const contextIndex = args.indexOf("--context");
const context = contextIndex >= 0 ? args[contextIndex + 1] : undefined;
if (!context || args.some((a, i) => a !== "--apply" && a !== "--context" && i !== contextIndex + 1)) {
  console.error("Usage: node scripts/provision-ttyd-ca.ts --context <expected-context> [--apply]");
  process.exit(2);
}
function kubectl(args: string[], input?: string): string {
  return execFileSync("kubectl", ["--context", context!, ...args], {
    input, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"], timeout: 30_000,
  });
}
const targets = [
  { namespace: "memeloop-workspace-control", name: "ttyd-server-ca" },
  { namespace: "higress-system", name: "ttyd-client-ca" },
  { namespace: "memeloop-workspace-control", name: "ttyd-client-trust" },
  { namespace: "higress-system", name: "ttyd-client-cacert" },
];
let directory: string | undefined;
const created: string[] = [];
try {
  // A mistyped/implicit target must never provision an unrelated cluster.
  const current = kubectl(["config", "current-context"]).trim();
  if (current !== context) throw new Error("current context differs from explicitly selected context");
  for (const target of targets) {
    kubectl(["get", "namespace", target.namespace, "-o", "name"]);
    const existing = kubectl(["-n", target.namespace, "get", "secret", target.name, "--ignore-not-found", "-o", "name"]);
    if (existing.trim()) throw new Error("one or more destination Secrets already exist; bootstrap refuses replacement");
  }
  if (!apply) {
    console.log("Preflight passed; four new Secret destinations are absent. No keys generated. Use --apply to provision.");
  } else {
    directory = mkdtempSync(join(tmpdir(), "mwc-ttyd-ca-"));
    chmodSync(directory, 0o700);
    function root(role: string): { certificate: string; key: string } {
      const key = join(directory!, `${role}.key`);
      const cert = join(directory!, `${role}.crt`);
      execFileSync("openssl", [
        "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
        "-noenc", "-sha256", "-days", "3650", "-subj", `/CN=Memeloop ttyd ${role} CA`,
        "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0",
        "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-keyout", key, "-out", cert,
      ], { stdio: "ignore", timeout: 30_000 });
      chmodSync(key, 0o600);
      execFileSync("openssl", ["verify", "-CAfile", cert, cert], { stdio: "ignore" });
      return { certificate: readFileSync(cert).toString("base64"), key: readFileSync(key).toString("base64") };
    }
    const server = root("server");
    const client = root("client");
    const data = [
      { "tls.crt": server.certificate, "tls.key": server.key },
      { "tls.crt": client.certificate, "tls.key": client.key },
      { "ca.crt": client.certificate },
      { cacert: server.certificate },
    ];
    for (const [index, target] of targets.entries()) {
      // Kubernetes create (not apply/patch) also rejects a concurrent creator.
      kubectl(["create", "-f", "-"], JSON.stringify({
        apiVersion: "v1", kind: "Secret",
        metadata: { ...target, labels: { "app.kubernetes.io/part-of": "memeloop-workspace-control" } },
        type: index < 2 ? "kubernetes.io/tls" : "Opaque",
        data: data[index],
      }));
      created.push(`${target.namespace}/${target.name}`);
    }
    console.log("Provisioned two independent CA Secrets and two public-only trust Secrets. No certificate consumers or workloads changed.");
  }
} catch {
  // Subprocess exceptions can embed input/credential material: never print the exception.
  console.error(`CA bootstrap failed; ${created.length} destination Secrets created. Existing or newly created Secrets were not deleted; inspect names and resume manually.`);
  process.exitCode = 1;
} finally {
  if (directory) rmSync(directory, { recursive: true, force: true });
}
