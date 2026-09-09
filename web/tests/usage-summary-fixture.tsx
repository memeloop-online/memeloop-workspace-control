import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { I18nProvider } from "../src/i18n";
import { WorkspaceStats } from "../src/workspaces/WorkspaceStats";
import type { OrganizationUsageSummary, Resources } from "../src/types";
import "../src/styles.css";
import "../src/ui.css";
import "../src/workspace-ui.css";

const summary: OrganizationUsageSummary = {
  total_count: 128, state_counts: { ready: 96, stopped: 32 },
  requested: { cpu_millis: 64_000, memory_mib: 131_072, disk_gib: 2_048, gpu_count: 0 },
  actual: { cpu_millis: 23_500, memory_mib: 65_536, disk_bytes: 768 * 1024 ** 3 }, observed_at: 1_789_000_000,
  availability: { cpu: "available", memory: "available", disk: "available" },
  coverage: { total_workspaces: 128, eligible_workspaces: 128, template_label_coverage: "complete" },
};
const quota: Resources = { cpu_millis: 96_000, memory_mib: 196_608, disk_gib: 4_096, gpu_count: 0 };
createRoot(document.getElementById("root")!).render(<StrictMode><I18nProvider><main className="content fixture-content"><div className="panel-stack"><WorkspaceStats summary={summary} quota={quota} /></div></main></I18nProvider></StrictMode>);
