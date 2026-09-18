import { translate } from '@docusaurus/Translate';

export const PREVIEW_LABELS = {
  workspacePicker: translate({ id: 'preview.status.selectLabel', message: 'Choose a workspace' }),
  state: {
    provisioning: translate({ id: 'preview.status.provisioning', message: 'Provisioning' }),
    ready: translate({ id: 'preview.status.ready', message: 'Ready' }),
    starting: translate({ id: 'preview.status.starting', message: 'Starting' }),
    stopping: translate({ id: 'preview.status.stopping', message: 'Stopping' }),
    stopped: translate({ id: 'preview.status.stopped', message: 'Stopped' }),
    restarting: translate({ id: 'preview.status.restarting', message: 'Restarting' }),
    deleting: translate({ id: 'preview.status.deleting', message: 'Deleting' }),
    deleted: translate({ id: 'preview.status.deleted', message: 'Deleted' }),
    failed: translate({ id: 'preview.status.failed', message: 'Failed' }),
  },
  meter: {
    cpu: translate({ id: 'preview.meters.cpu', message: 'CPU' }),
    memory: translate({ id: 'preview.meters.memory', message: 'Memory' }),
    disk: translate({ id: 'preview.meters.disk', message: 'Persistent disk' }),
    temporary: translate({ id: 'preview.meters.ephemeral', message: 'Temporary storage' }),
    usageOfLimit: translate({ id: 'preview.meters.usageOfLimit', message: 'Usage of limit' }),
    unavailable: translate({ id: 'preview.meters.unavailable', message: 'Usage unavailable' }),
    telemetryAvailable: translate({ id: 'preview.meters.telemetryAvailable', message: 'Live usage' }),
    telemetryStale: translate({ id: 'preview.meters.telemetryStale', message: 'Last observed usage' }),
    telemetryDisabled: translate({ id: 'preview.meters.telemetryDisabled', message: 'Released when stopped' }),
    telemetryUnavailable: translate({ id: 'preview.meters.telemetryUnavailable', message: 'Usage unavailable' }),
    nodeLocalTelemetryUnavailable: translate({ id: 'preview.meters.nodeLocalTelemetryUnavailable', message: 'Node-local usage unavailable' }),
    pressureCritical: translate({ id: 'preview.meters.pressureCritical', message: 'Storage pressure is critical' }),
    pressureWarning: translate({ id: 'preview.meters.pressureWarning', message: 'Storage pressure is high' }),
    observedAt: translate({ id: 'preview.meters.observedAt', message: 'Observed' }),
    usageNotObserved: translate({ id: 'preview.meters.usageNotObserved', message: 'Usage not observed' }),
    storageUsage: translate({ id: 'preview.meters.storageUsage', message: 'storage usage' }),
  },
  scope: {
    organization: translate({ id: 'preview.scopes.organization', message: 'Organization' }),
    user: translate({ id: 'preview.scopes.user', message: 'User' }),
    workspace: translate({ id: 'preview.scopes.workspace', message: 'Workspace' }),
    ariaLabel: translate({ id: 'preview.scopes.ariaLabel', message: 'Credential scope' }),
    title: translate({ id: 'preview.scopes.listTitle', message: 'Credentials' }),
    empty: translate({ id: 'preview.scopes.empty', message: 'No credentials at this scope' }),
    searchPlaceholder: translate({ id: 'preview.scopes.searchPlaceholder', message: 'Search credentials' }),
    clearSearch: translate({ id: 'preview.scopes.clearSearch', message: 'Clear search' }),
    loading: translate({ id: 'preview.scopes.loading', message: 'Loading credentials' }),
    noFilteredResults: translate({ id: 'preview.scopes.noFilteredResults', message: 'No matching credentials' }),
    sensitiveValue: translate({ id: 'preview.scopes.sensitiveValue', message: 'Sensitive value' }),
    visibleConfiguration: translate({ id: 'preview.scopes.visibleConfiguration', message: 'Configuration' }),
    lockedState: translate({ id: 'preview.scopes.lockedState', message: 'Locked' }),
  },
  credentialKind: {
    environment_variable: translate({ id: 'preview.credentials.type.env', message: 'Environment variable' }),
    secret_file: translate({ id: 'preview.credentials.type.file', message: 'Secret file' }),
    config_file: translate({ id: 'preview.credentials.type.config', message: 'Configuration file' }),
    ssh_public_key: translate({ id: 'preview.credentials.type.sshKey', message: 'SSH public key' }),
  },
};

export function formatWorkspaceMeta(workspace) {
  return translate(
    {
      id: 'preview.status.meta',
      message: 'Short ID {id} · Template {template}',
    },
    { id: workspace.id, template: workspace.template },
  );
}

export function formatCredentialCount(count) {
  return translate(
    {
      id: 'preview.scopes.count',
      message: '{count} credentials at this scope',
    },
    { count },
  );
}
