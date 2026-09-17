import React from 'react';
import Layout from '@theme/Layout';
import Link from '@docusaurus/Link';
import Translate, { translate } from '@docusaurus/Translate';
import styles from './index.module.css';

const FEATURES = [
  {
    title: translate({ id: 'home.feature.workspaces.title', message: 'Isolated workspaces' }),
    body: translate({
      id: 'home.feature.workspaces.body',
      message: 'Per-user Kubernetes workloads from reviewed templates and an image allowlist.',
    }),
  },
  {
    title: translate({ id: 'home.feature.injections.title', message: 'Credential injection' }),
    body: translate({
      id: 'home.feature.injections.body',
      message: 'Environment variables, files, and SSH keys injected at start, encrypted at rest.',
    }),
  },
  {
    title: translate({ id: 'home.feature.access.title', message: 'Workspace access' }),
    body: translate({
      id: 'home.feature.access.body',
      message: 'SSH, a browser terminal, and authenticated port mappings configured for each deployment.',
    }),
  },
  {
    title: translate({ id: 'home.feature.governance.title', message: 'Administration' }),
    body: translate({
      id: 'home.feature.governance.body',
      message: 'Image allowlists, templates, quotas, node pools, and audit logging.',
    }),
  },
  {
    title: translate({ id: 'home.feature.api.title', message: 'API-first' }),
    body: translate({
      id: 'home.feature.api.body',
      message: 'Scoped API keys, cursor pagination, SSE events, and signed webhooks.',
    }),
  },
  {
    title: translate({ id: 'home.feature.plugins.title', message: 'WebAssembly plugins' }),
    body: translate({
      id: 'home.feature.plugins.body',
      message: 'Creation policies, API middleware, routes, and console surfaces over a versioned WIT ABI.',
    }),
  },
];

export default function Home() {
  return (
    <Layout
      title={translate({ id: 'home.title', message: 'Memeloop Workspace Control' })}
      description={translate({
        id: 'home.description',
        message: 'A Kubernetes control plane for isolated, per-user development workspaces',
      })}>
      <header className={styles.hero}>
        <h1 className={styles.heroTitle}>
          <Translate id="home.hero.title">Memeloop Workspace Control</Translate>
        </h1>
        <p className={styles.heroTagline}>
          <Translate id="home.hero.tagline">
            Provision isolated development workspaces on Kubernetes, inject credentials safely,
            and reach them over SSH, a browser terminal, or authenticated port mappings.
          </Translate>
        </p>
        <div className={styles.heroButtons}>
          <Link className="button button--primary button--lg" to="/docs/quickstart">
            <Translate id="home.hero.cta.quickstart">Quick start</Translate>
          </Link>
          <Link className="button button--secondary button--lg" to="/features">
            <Translate id="home.hero.cta.features">Explore features</Translate>
          </Link>
        </div>
      </header>
      <main className={styles.features}>
        <div className={styles.featureGrid}>
          {FEATURES.map((feature) => (
            <div key={feature.title} className={styles.featureCard}>
              <h3>{feature.title}</h3>
              <p>{feature.body}</p>
            </div>
          ))}
        </div>
      </main>
    </Layout>
  );
}
