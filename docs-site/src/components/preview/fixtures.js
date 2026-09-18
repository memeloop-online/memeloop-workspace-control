export const WORKSPACES = [
  {
    id: 'ws-01J4EX',
    name: 'web-dev-demo',
    template: 'Node Dev',
    status: 'ready',
    meters: [
      { key: 'cpu', actual: '4.7 m', requested: '6000 m', percent: 1 },
      { key: 'memory', actual: '234 MiB', requested: '4 GiB', percent: 6 },
      { key: 'disk', kind: 'storage', configuredGiB: 30, telemetry: storageTelemetry(3, 30, 'persistent_volume') },
      { key: 'ephemeral', kind: 'storage', configuredGiB: 22, telemetry: storageTelemetry(0.223, 22, 'node_local') },
    ],
  },
  {
    id: 'ws-01J4FQ',
    name: 'maintenance-demo',
    template: 'Maintenance',
    status: 'ready',
    meters: [
      { key: 'cpu', actual: '3.2 m', requested: '1000 m', percent: 1 },
      { key: 'memory', actual: '49 MiB', requested: '1 GiB', percent: 5 },
      { key: 'disk', kind: 'storage', configuredGiB: 1.9, telemetry: storageTelemetry(0.543, 1.9, 'persistent_volume') },
      { key: 'ephemeral', kind: 'storage', configuredGiB: 5, telemetry: storageTelemetry(0.068, 5, 'node_local') },
    ],
  },
  {
    id: 'ws-01J4GS',
    name: 'ml-training-demo',
    template: 'GPU Training',
    status: 'starting',
    meters: [
      { key: 'cpu', actual: '120 m', requested: '8000 m', percent: 2 },
      { key: 'memory', actual: '512 MiB', requested: '32 GiB', percent: 2 },
      { key: 'disk', kind: 'storage', configuredGiB: 100, telemetry: storageTelemetry(6.4, 100, 'persistent_volume') },
      { key: 'ephemeral', kind: 'storage', configuredGiB: 40, telemetry: storageTelemetry(1.1, 40, 'node_local') },
    ],
  },
  {
    id: 'ws-01J4HT',
    name: 'docs-archive-demo',
    template: 'Maintenance',
    status: 'stopped',
    meters: [
      { key: 'cpu', actual: '0 m', requested: '1000 m', percent: 0 },
      { key: 'memory', actual: '0 MiB', requested: '1 GiB', percent: 0 },
      { key: 'disk', kind: 'storage', configuredGiB: 1.9, telemetry: storageTelemetry(1.2, 1.9, 'persistent_volume') },
      { key: 'ephemeral', kind: 'storage', configuredGiB: 5, telemetry: { configured_bytes: 5 * 1024 ** 3, used_bytes: null, capacity_bytes: null, available_bytes: null, observed_at: null, used_percent: null, pressure: null, coverage: 'disabled', backing: 'node_local' } },
    ],
  },
];

function storageTelemetry(usedGiB, capacityGiB, backing) {
  const gib = 1024 ** 3;
  return {
    configured_bytes: capacityGiB * gib,
    used_bytes: usedGiB * gib,
    capacity_bytes: capacityGiB * gib,
    available_bytes: (capacityGiB - usedGiB) * gib,
    observed_at: 1789728000,
    used_percent: (usedGiB / capacityGiB) * 100,
    pressure: 'normal',
    coverage: 'exact',
    backing,
  };
}

export const CREDENTIAL_SCOPES = ['organization', 'user', 'workspace'];

export const CREDENTIALS = [
  {
    name: 'example-service-token',
    kind: 'environment_variable',
    scope: 'organization',
    target: 'MWC_SERVICE_TOKEN',
    version: 2,
    sensitive: true,
    locked: true,
  },
  {
    name: 'example-registry-credentials',
    kind: 'secret_file',
    scope: 'organization',
    target: '/home/user/.docker/config.json',
    version: 5,
    sensitive: true,
    locked: false,
  },
  {
    name: 'example-agent-profile',
    kind: 'config_file',
    scope: 'user',
    target: '/home/user/.codex/agents/luna-worker.toml',
    version: 3,
    sensitive: false,
    locked: false,
  },
  {
    name: 'example-environment',
    kind: 'environment_variable',
    scope: 'user',
    target: '/home/user/.codex/.env',
    version: 2,
    sensitive: false,
    locked: false,
  },
  {
    name: 'example-git-config',
    kind: 'config_file',
    scope: 'user',
    target: '/home/user/.config/gh/config.yml',
    version: 2,
    sensitive: false,
    locked: false,
  },
  {
    name: 'example-deploy-key',
    kind: 'ssh_public_key',
    scope: 'workspace',
    target: '/home/user/.ssh/id_ed25519',
    version: 1,
    sensitive: false,
    locked: false,
  },
  {
    name: 'example-tool-config',
    kind: 'config_file',
    scope: 'workspace',
    target: '/home/user/.codex/config.toml',
    version: 4,
    sensitive: false,
    locked: false,
  },
];
