import React from 'react';
import Layout from '@theme/Layout';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
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

const STORIES = [
  {
    eyebrow: translate({ id: 'home.story.workspaces.eyebrow', message: 'Workspaces' }),
    title: translate({ id: 'home.story.workspaces.title', message: 'Every workspace at a glance' }),
    body: translate({
      id: 'home.story.workspaces.body',
      message:
        'The console rolls up fleet-wide CPU, memory, and storage, then lets you drill into each workspace for live usage, lifecycle actions, and event history.',
    }),
    points: [
      translate({
        id: 'home.story.workspaces.point.1',
        message: 'Organization-level resource meters with per-workspace breakdowns',
      }),
      translate({
        id: 'home.story.workspaces.point.2',
        message: 'Start, stop, restart, and delete with guardrails from reviewed templates',
      }),
      translate({
        id: 'home.story.workspaces.point.3',
        message: 'One-click SSH, browser terminal, and authenticated port mappings',
      }),
    ],
    image: 'img/screenshots/workspaces-desktop.png',
    alt: translate({
      id: 'home.story.workspaces.alt',
      message: 'Workspace list showing resource meters, status badges, and lifecycle actions',
    }),
    width: 1440,
    height: 1000,
  },
  {
    eyebrow: translate({ id: 'home.story.credentials.eyebrow', message: 'Credentials & files' }),
    title: translate({ id: 'home.story.credentials.title', message: 'Secrets, scoped and injected' }),
    body: translate({
      id: 'home.story.credentials.body',
      message:
        'Define credentials once at the organization, user, or workspace scope. MWC injects them as environment variables or files at start — encrypted at rest, versioned on change.',
    }),
    points: [
      translate({
        id: 'home.story.credentials.point.1',
        message: 'Three-scope cascade with template selectors and organization-locked items',
      }),
      translate({
        id: 'home.story.credentials.point.2',
        message: 'File mode, ownership, and target path controls for POSIX-correct delivery',
      }),
      translate({
        id: 'home.story.credentials.point.3',
        message: 'Versioned updates on change, with effective-source previews',
      }),
    ],
    image: 'img/screenshots/credentials-desktop.png',
    alt: translate({
      id: 'home.story.credentials.alt',
      message: 'Credential library with scoped items and a creation form for file-based secrets',
    }),
    width: 1440,
    height: 1000,
    reversed: true,
  },
  {
    eyebrow: translate({ id: 'home.story.mobile.eyebrow', message: 'Anywhere' }),
    title: translate({ id: 'home.story.mobile.title', message: 'Full control from any screen' }),
    body: translate({
      id: 'home.story.mobile.body',
      message:
        'The console is responsive end to end: meters, search, lifecycle actions, and terminals stay usable on narrow displays, so a phone is enough to unblock a teammate.',
    }),
    points: [
      translate({
        id: 'home.story.mobile.point.1',
        message: 'Adaptive layout from wide dashboards down to handset widths',
      }),
      translate({
        id: 'home.story.mobile.point.2',
        message: 'Keyboard-navigable controls and accessible contrast in both themes',
      }),
    ],
    image: 'img/screenshots/workspaces-mobile.png',
    alt: translate({
      id: 'home.story.mobile.alt',
      message: 'Workspace console on a narrow mobile viewport with stacked meters and actions',
    }),
    width: 390,
    height: 1154,
    narrow: true,
  },
];

function StorySection({ story }) {
  const imageUrl = useBaseUrl(story.image);
  return (
    <section
      className={
        story.reversed ? `${styles.story} ${styles.storyReversed}` : styles.story
      }>
      <div className={styles.storyCopy}>
        <p className={styles.eyebrow}>{story.eyebrow}</p>
        <h2 className={styles.storyTitle}>{story.title}</h2>
        <p className={styles.storyBody}>{story.body}</p>
        <ul className={styles.storyPoints}>
          {story.points.map((point) => (
            <li key={point}>{point}</li>
          ))}
        </ul>
      </div>
      <div className={story.narrow ? `${styles.storyFrame} ${styles.storyFrameNarrow}` : styles.storyFrame}>
        <img
          src={imageUrl}
          alt={story.alt}
          width={story.width}
          height={story.height}
          loading="lazy"
          className={styles.storyImage}
        />
      </div>
    </section>
  );
}

export default function Home() {
  return (
    <Layout
      title={translate({ id: 'home.title', message: 'Memeloop Workspace Control' })}
      description={translate({
        id: 'home.description',
        message: 'A Kubernetes control plane for isolated, per-user development workspaces',
      })}>
      <header className={styles.hero}>
        <div className={styles.heroInner}>
          <p className={styles.heroBadge}>
            <Translate id="home.hero.badge">Kubernetes-native workspace control</Translate>
          </p>
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
            <Link className="button button--secondary button--lg" to="/preview">
              <Translate id="home.hero.cta.preview">Try the preview</Translate>
            </Link>
            <Link className="button button--outline button--lg" to="/docs/">
              <Translate id="home.hero.cta.docs">Read the docs</Translate>
            </Link>
          </div>
        </div>
      </header>
      <main>
        <div className={styles.stories}>
          {STORIES.map((story) => (
            <StorySection key={story.title} story={story} />
          ))}
        </div>
        <section className={styles.features}>
          <p className={styles.eyebrow}>
            <Translate id="home.features.eyebrow">Platform</Translate>
          </p>
          <h2 className={styles.featuresTitle}>
            <Translate id="home.features.title">Built for users, admins, and integrators</Translate>
          </h2>
          <div className={styles.featureGrid}>
            {FEATURES.map((feature) => (
              <div key={feature.title} className={styles.featureCard}>
                <h3>{feature.title}</h3>
                <p>{feature.body}</p>
              </div>
            ))}
          </div>
        </section>
        <section className={styles.ctaBand}>
          <h2 className={styles.ctaTitle}>
            <Translate id="home.cta.title">Ship workspaces your team can trust</Translate>
          </h2>
          <p className={styles.ctaBody}>
            <Translate id="home.cta.body">
              Deploy the control plane, connect a cluster, and launch the first workspace in
              minutes — or explore the read-only preview first.
            </Translate>
          </p>
          <div className={styles.heroButtons}>
            <Link className="button button--primary button--lg" to="/docs/quickstart">
              <Translate id="home.hero.cta.quickstart">Quick start</Translate>
            </Link>
            <Link className="button button--secondary button--lg" to="/preview">
              <Translate id="home.hero.cta.preview">Try the preview</Translate>
            </Link>
          </div>
        </section>
      </main>
    </Layout>
  );
}
