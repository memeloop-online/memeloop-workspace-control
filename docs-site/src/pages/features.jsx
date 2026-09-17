import React from 'react';
import Layout from '@theme/Layout';
import Link from '@docusaurus/Link';
import Translate, { translate } from '@docusaurus/Translate';
import styles from './index.module.css';

const SECTIONS = [
  {
    title: translate({ id: 'features.workspaces.title', message: 'Workspaces' }),
    body: translate({
      id: 'features.workspaces.body',
      message: 'Full lifecycle (start, stop, restart, delete) from reviewed templates, with durable home volumes, bounded temporary storage, node-pool placement, and optional sandboxed runtime classes.',
    }),
    to: '/docs/workspaces',
  },
  {
    title: translate({ id: 'features.credentials.title', message: 'Credentials and files' }),
    body: translate({
      id: 'features.credentials.body',
      message: 'Three-scope injection cascade (organization, user, workspace) with encrypted storage, template selectors, locked organization items, and per-workspace selection.',
    }),
    to: '/docs/credentials-and-files',
  },
  {
    title: translate({ id: 'features.access.title', message: 'Access' }),
    body: translate({
      id: 'features.access.body',
      message: 'SSH, a browser terminal with single-use tickets, and authenticated HTTPS port mappings when enabled by the deployment and template.',
    }),
    to: '/docs/access',
  },
  {
    title: translate({ id: 'features.admin.title', message: 'Administration' }),
    body: translate({
      id: 'features.admin.body',
      message: 'Default-deny image allowlist, template catalog, organizations and members, two-level quotas, node pools, audit events, and platform metrics.',
    }),
    to: '/docs/administration',
  },
  {
    title: translate({ id: 'features.api.title', message: 'API' }),
    body: translate({
      id: 'features.api.body',
      message: 'Scoped API keys with mandatory expiry, cursor pagination, SSE event stream, signed webhooks, and an OpenAPI contract served by every deployment.',
    }),
    to: '/docs/api',
  },
  {
    title: translate({ id: 'features.plugins.title', message: 'Plugin development' }),
    body: translate({
      id: 'features.plugins.body',
      message: 'Sandboxed WebAssembly components that enforce creation policies, intercept requests, add API routes, and render console UI surfaces.',
    }),
    to: '/docs/plugin-development',
  },
  {
    title: translate({ id: 'features.security.title', message: 'Security model' }),
    body: translate({
      id: 'features.security.body',
      message: 'Workload and network isolation, supply-chain control, scoped authentication, encryption at rest, and clear deployer responsibilities.',
    }),
    to: '/docs/security-model',
  },
];

export default function Features() {
  return (
    <Layout
      title={translate({ id: 'features.title', message: 'Features' })}
      description={translate({
        id: 'features.description',
        message: 'What Memeloop Workspace Control provides for users, administrators, and integrators',
      })}>
      <main className={styles.features}>
        <h1>
          <Translate id="features.heading">Features</Translate>
        </h1>
        <p>
          <Translate id="features.intro">
            MWC serves four audiences: workspace users, platform administrators, API
            consumers, and plugin developers. Each section below links to the
            matching documentation.
          </Translate>
        </p>
        <div className={styles.featureGrid}>
          {SECTIONS.map((section) => (
            <div key={section.title} className={styles.featureCard}>
              <h3>{section.title}</h3>
              <p>{section.body}</p>
              <Link to={section.to}>
                <Translate id="features.readMore">Read more</Translate>
              </Link>
            </div>
          ))}
        </div>
      </main>
    </Layout>
  );
}
