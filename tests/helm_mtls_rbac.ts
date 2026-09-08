import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseAllDocuments } from "../web/node_modules/yaml/dist/index.js";

const [enabledPath, disabledPath] = process.argv.slice(2);
assert(enabledPath && disabledPath, "provide enabled and disabled Helm render paths");

function documents(path: string) {
  return parseAllDocuments(readFileSync(path, "utf8")).map((document) => {
    assert.equal(document.errors.length, 0, `invalid rendered YAML: ${document.errors}`);
    return document.toJS();
  }).filter(Boolean);
}

function permitsFilters(rule: { apiGroups?: string[]; resources?: string[] }) {
  return rule.apiGroups?.some((group) => ["*", "networking.istio.io"].includes(group))
    && rule.resources?.some((resource) => ["*", "envoyfilters"].includes(resource));
}

const enabled = documents(enabledPath);
const disabled = documents(disabledPath);
for (const document of [...enabled, ...disabled]) {
  if (document.kind === "ClusterRole") {
    assert(!document.rules?.some(permitsFilters), "gateway filter permissions must not be cluster-wide");
  }
}

const roles = enabled.filter((document) => document.kind === "Role" && document.rules?.some(permitsFilters));
assert.equal(roles.length, 1, "enabled deployment needs exactly one gateway Role");
const role = roles[0];
assert.equal(role.metadata.namespace, "higress-system");
assert.deepEqual(role.rules, [{
  apiGroups: ["networking.istio.io"],
  resources: ["envoyfilters"],
  verbs: ["get", "create", "patch", "delete"],
}]);
const bindings = enabled.filter((document) => document.kind === "RoleBinding"
  && document.roleRef?.name === role.metadata.name);
assert.equal(bindings.length, 1);
assert.equal(bindings[0].metadata.namespace, role.metadata.namespace);
assert.equal(bindings[0].roleRef.kind, "Role");
assert.equal(bindings[0].subjects.length, 1);
const subject = bindings[0].subjects[0];
assert.equal(subject.kind, "ServiceAccount");
assert.equal(subject.namespace, "memeloop-workspace-control");
assert(enabled.some((document) => document.kind === "ServiceAccount" && document.metadata.name === subject.name));
assert(!disabled.some((document) => document.kind === "Role" && document.rules?.some(permitsFilters)),
  "disabled deployment must not require a gateway Role");
console.log("ttyd mTLS RBAC render checks passed");
