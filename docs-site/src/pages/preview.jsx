import React from 'react';
import Layout from '@theme/Layout';
import Link from '@docusaurus/Link';
import Translate, { translate } from '@docusaurus/Translate';
import WorkspaceStatus from '@site/src/components/preview/WorkspaceStatus';
import CredentialScopes from '@site/src/components/preview/CredentialScopes';
import styles from '@site/src/components/preview/preview.module.css';

export default function Preview() {
  return (
    <Layout
      title={translate({ id: 'preview.title', message: 'Product preview' })}
      description={translate({
        id: 'preview.description',
        message: 'A read-only tour of the Memeloop Workspace Control console with local sample data',
      })}>
      <header className={styles.hero}>
        <h1 className={styles.heroTitle}>
          <Translate id="preview.heading">Console preview</Translate>
        </h1>
        <p className={styles.heroIntro}>
          <Translate id="preview.intro">
            Explore the surfaces workspace users touch every day: live status and resource meters,
            plus the credential scopes that decide what gets injected where.
          </Translate>
        </p>
        <p className={styles.heroNote}>
          <Translate id="preview.note">
            This preview uses local sample data and is read-only.
          </Translate>
        </p>
      </header>
      <main className={styles.main}>
        <section aria-labelledby="preview-status-heading">
          <h2 className={styles.sectionHeading} id="preview-status-heading">
            <Translate id="preview.status.heading">Workspace status & resources</Translate>
          </h2>
          <p className={styles.sectionCaption}>
            <Translate id="preview.status.caption">
              Pick a workspace to see its lifecycle state and CPU, memory, disk, and ephemeral
              usage against template limits.
            </Translate>
          </p>
          <WorkspaceStatus />
        </section>
        <section aria-labelledby="preview-scopes-heading">
          <h2 className={styles.sectionHeading} id="preview-scopes-heading">
            <Translate id="preview.scopes.heading">Credential scopes</Translate>
          </h2>
          <p className={styles.sectionCaption}>
            <Translate id="preview.scopes.caption">
              Switch between organization, user, and workspace scopes to see which credentials
              each level provides.
            </Translate>
          </p>
          <CredentialScopes />
        </section>
        <section aria-labelledby="preview-next-heading">
          <h2 className={styles.sectionHeading} id="preview-next-heading">
            <Translate id="preview.next.heading">Keep exploring</Translate>
          </h2>
          <p className={styles.sectionCaption}>
            <Translate id="preview.next.caption">
              Ready for the real thing? The quick start walks through deploying the control plane
              and launching your first workspace.
            </Translate>
          </p>
          <p>
            <Link className="button button--primary" to="/docs/quickstart">
              <Translate id="home.hero.cta.quickstart">Quick start</Translate>
            </Link>{' '}
            <Link className="button button--secondary" to="/docs/">
              <Translate id="home.hero.cta.docs">Read the docs</Translate>
            </Link>
          </p>
        </section>
      </main>
    </Layout>
  );
}
