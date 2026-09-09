import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import type { ApiClient } from "../src/api";
import { I18nProvider } from "../src/i18n";
import { ApiKeySection } from "../src/settings/ApiKeySection";
import type { ApiKeySummary, CreatedApiKey, Principal, WorkspaceTemplate } from "../src/types";
import "../src/styles.css";
import "../src/ui.css";
import "../src/settings-ui.css";

const query = new URLSearchParams(window.location.search);
const restricted = query.get("restricted") === "1";
const templates = Array.from({ length: 24 }, (_, index) => ({
  id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
  organization_id: "org-fixture",
  name: `Workspace template ${String(index + 1).padStart(2, "0")}`,
  enabled: true,
})) as WorkspaceTemplate[];
const allowedTemplateIds = restricted ? templates.filter((_, index) => index % 2 === 0).map(({ id }) => id) : null;
const keys: ApiKeySummary[] = [{
  id: "key-fixture-existing",
  name: "Existing fixture key",
  prefix: "mwc_fixture_existing",
  last_used_at: 1_788_000_000,
  created_at: 1_787_000_000,
  scopes: ["read_workspace"],
  expires_at: 1_789_000_000,
  allowed_template_ids: null,
  revoked_at: null,
}];

const api = {
  apiKeys: async () => keys,
  templates: async () => templates,
  createApiKey: async (input: { name: string; scopes: Principal["api_key_scopes"]; expires_at: number; allowed_template_ids: string[] | null }) => ({
    id: "key-fixture",
    name: input.name,
    prefix: "mwc_fixture",
    last_used_at: null,
    created_at: Math.floor(Date.now() / 1_000),
    scopes: input.scopes,
    expires_at: input.expires_at,
    allowed_template_ids: input.allowed_template_ids,
    revoked_at: null,
    token: "fixture-token",
  } satisfies CreatedApiKey),
  deleteApiKey: async () => undefined,
} as unknown as ApiClient;

const principal = {
  user_id: "user-fixture",
  display_name: "UI fixture",
  system_admin: false,
  memberships: [{ organization_id: "org-fixture", role: "organization_admin" }],
  api_key_scopes: ["manage_api_keys", "read_workspace"],
  api_key_expires_at: null,
  allowed_template_ids: allowedTemplateIds,
} satisfies Principal;

function Harness() {
  return <I18nProvider>
    <main className="content fixture-content">
      <div className="panel-stack">
        <ApiKeySection api={api} organizationId="org-fixture" principal={principal} onError={(message) => console.error(message)} />
      </div>
    </main>
  </I18nProvider>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><Harness /></StrictMode>);
