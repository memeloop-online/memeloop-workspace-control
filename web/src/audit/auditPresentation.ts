import type { MessageKey } from "../i18n";
import type { AuditRecord } from "../types";

const ACTION_LABELS: Readonly<Record<string, MessageKey>> = {
  "workspace.create": "auditActionWorkspaceCreate",
  "workspace.start": "auditActionWorkspaceStart",
  "workspace.stop": "auditActionWorkspaceStop",
  "workspace.restart": "auditActionWorkspaceRestart",
  "workspace.delete": "auditActionWorkspaceDelete",
  "workspace.mark_ready": "auditActionWorkspaceReady",
  "workspace.mark_stopped": "auditActionWorkspaceStopped",
  "workspace.mark_deleted": "auditActionWorkspaceDeleted",
  "workspace.mark_failed": "auditActionWorkspaceFailed",
  "workspace.port_mapping.create": "auditActionPortMappingCreate",
  "workspace.port_mapping.delete": "auditActionPortMappingDelete",
  "injection.replace": "auditActionCredentialReplace",
  "injection.delete": "auditActionCredentialDelete",
  "organization.create": "auditActionOrganizationCreate",
  "organization.update": "auditActionOrganizationUpdate",
  "organization.delete": "auditActionOrganizationDelete",
  "user.create": "auditActionUserCreate",
  "user.update": "auditActionUserUpdate",
  "user.profile.update": "auditActionProfileUpdate",
  "user.api_key.create": "auditActionApiKeyCreate",
  "user.api_key.revoke": "auditActionApiKeyRevoke",
  "user.api_key.admin_revoke": "auditActionApiKeyAdminRevoke",
  "membership.upsert": "auditActionMembershipUpsert",
  "membership.remove": "auditActionMembershipRemove",
  "quota.set": "auditActionOrganizationQuotaSet",
  "user_quota.set": "auditActionUserQuotaSet",
  "image_policy.upsert": "auditActionImagePolicyUpsert",
  "template.create": "auditActionTemplateCreate",
  "template.update": "auditActionTemplateUpdate",
  "template.enabled": "auditActionTemplateEnabled",
  "template.delete": "auditActionTemplateDelete",
  "webhook.create": "auditActionWebhookCreate",
  "plugin.install": "auditActionPluginInstall",
  "plugin.enabled.set": "auditActionPluginEnabled",
  "plugin.uninstall": "auditActionPluginUninstall",
  "plugin.configuration.put": "auditActionPluginConfigurationPut",
  "plugin.configuration.delete": "auditActionPluginConfigurationDelete",
};

const ACTION_STATES: Readonly<Record<string, MessageKey>> = {
  "workspace.create": "stateProvisioning",
  "workspace.start": "stateStarting",
  "workspace.stop": "stateStopping",
  "workspace.restart": "stateRestarting",
  "workspace.delete": "stateDeleting",
  "workspace.mark_ready": "stateReady",
  "workspace.mark_stopped": "stateStopped",
  "workspace.mark_deleted": "stateDeleted",
  "workspace.mark_failed": "stateFailed",
};

const STATE_LABELS: Readonly<Record<string, MessageKey>> = {
  provisioning: "stateProvisioning",
  ready: "stateReady",
  stopping: "stateStopping",
  stopped: "stateStopped",
  starting: "stateStarting",
  restarting: "stateRestarting",
  deleting: "stateDeleting",
  deleted: "stateDeleted",
  failed: "stateFailed",
};

export function auditActionLabel(action: string): MessageKey {
  return ACTION_LABELS[action] ?? "auditOtherAction";
}

export function auditStateLabel(record: AuditRecord): MessageKey | undefined {
  const actionState = ACTION_STATES[record.action];
  if (actionState) return actionState;
  if (typeof record.metadata.state === "string") return STATE_LABELS[record.metadata.state];
  if (typeof record.metadata.enabled === "boolean") return record.metadata.enabled ? "enabled" : "disabled";
  return undefined;
}

export function hasKnownAuditAction(action: string): boolean {
  return Object.hasOwn(ACTION_LABELS, action);
}
