export const WORKSPACES = [
  {
    id: 'ws-01J4EX',
    name: 'web-dev-demo',
    template: 'Node Dev',
    status: 'ready',
    meters: [
      { key: 'cpu', used: '4.7 m', limit: '6000 m', percent: 1 },
      { key: 'memory', used: '234 MiB', limit: '4 GiB', percent: 6 },
      { key: 'disk', used: '3.0 GiB', limit: '30 GiB', percent: 10 },
      { key: 'ephemeral', used: '223 MiB', limit: '22 GiB', percent: 1 },
    ],
  },
  {
    id: 'ws-01J4FQ',
    name: 'maintenance-demo',
    template: 'Maintenance',
    status: 'ready',
    meters: [
      { key: 'cpu', used: '3.2 m', limit: '1000 m', percent: 1 },
      { key: 'memory', used: '49 MiB', limit: '1 GiB', percent: 5 },
      { key: 'disk', used: '543 MiB', limit: '1.9 GiB', percent: 27 },
      { key: 'ephemeral', used: '68 MiB', limit: '5 GiB', percent: 2 },
    ],
  },
  {
    id: 'ws-01J4GS',
    name: 'ml-training-demo',
    template: 'GPU Training',
    status: 'starting',
    meters: [
      { key: 'cpu', used: '120 m', limit: '8000 m', percent: 2 },
      { key: 'memory', used: '512 MiB', limit: '32 GiB', percent: 2 },
      { key: 'disk', used: '6.4 GiB', limit: '100 GiB', percent: 7 },
      { key: 'ephemeral', used: '1.1 GiB', limit: '40 GiB', percent: 3 },
    ],
  },
  {
    id: 'ws-01J4HT',
    name: 'docs-archive-demo',
    template: 'Maintenance',
    status: 'stopped',
    meters: [
      { key: 'cpu', used: '0 m', limit: '1000 m', percent: 0 },
      { key: 'memory', used: '0 MiB', limit: '1 GiB', percent: 0 },
      { key: 'disk', used: '1.2 GiB', limit: '1.9 GiB', percent: 63 },
      { key: 'ephemeral', released: true },
    ],
  },
];

export const CREDENTIAL_SCOPES = ['organization', 'user', 'workspace'];

export const CREDENTIALS = [
  {
    name: 'example-service-token',
    type: 'env',
    scope: 'organization',
    target: 'MWC_SERVICE_TOKEN',
    version: 'v2',
  },
  {
    name: 'example-registry-credentials',
    type: 'file',
    scope: 'organization',
    target: '/home/user/.docker/config.json',
    version: 'v5',
  },
  {
    name: 'example-agent-profile',
    type: 'file',
    scope: 'user',
    target: '/home/user/.codex/agents/luna-worker.toml',
    version: 'v3',
  },
  {
    name: 'example-environment',
    type: 'env',
    scope: 'user',
    target: '/home/user/.codex/.env',
    version: 'v2',
  },
  {
    name: 'example-git-config',
    type: 'file',
    scope: 'user',
    target: '/home/user/.config/gh/config.yml',
    version: 'v2',
  },
  {
    name: 'example-deploy-key',
    type: 'sshKey',
    scope: 'workspace',
    target: '/home/user/.ssh/id_ed25519',
    version: 'v1',
  },
  {
    name: 'example-tool-config',
    type: 'file',
    scope: 'workspace',
    target: '/home/user/.codex/config.toml',
    version: 'v4',
  },
];
