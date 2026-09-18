import React, { useState } from 'react';
import Translate, { translate } from '@docusaurus/Translate';
import { CREDENTIAL_SCOPES, CREDENTIALS } from './fixtures';
import styles from './preview.module.css';

const SCOPE_LABELS = {
  organization: translate({ id: 'preview.scopes.organization', message: 'Organization' }),
  user: translate({ id: 'preview.scopes.user', message: 'User' }),
  workspace: translate({ id: 'preview.scopes.workspace', message: 'Workspace' }),
};

const SCOPE_DESCRIPTIONS = {
  organization: translate({
    id: 'preview.scopes.organization.desc',
    message: 'Shared with every member and workspace in the organization. Admins can lock items.',
  }),
  user: translate({
    id: 'preview.scopes.user.desc',
    message: 'Private to one user across all of their workspaces.',
  }),
  workspace: translate({
    id: 'preview.scopes.workspace.desc',
    message: 'Applies to a single workspace, layered over organization and user items.',
  }),
};

const TYPE_LABELS = {
  env: translate({ id: 'preview.credentials.type.env', message: 'Environment' }),
  file: translate({ id: 'preview.credentials.type.file', message: 'File' }),
  sshKey: translate({ id: 'preview.credentials.type.sshKey', message: 'SSH key' }),
};

export default function CredentialScopes() {
  const [scope, setScope] = useState('user');
  const visible = CREDENTIALS.filter((credential) => credential.scope === scope);
  return (
    <div className={styles.panel}>
      <fieldset className={styles.scopeFieldset}>
        <legend className={styles.panelLabel}>
          <Translate id="preview.scopes.selectLabel">Select a credential scope</Translate>
        </legend>
        <div className={styles.scopePicker}>
          {CREDENTIAL_SCOPES.map((scopeKey) => (
            <label
              key={scopeKey}
              className={
                scope === scopeKey
                  ? `${styles.scopeOption} ${styles.scopeOptionActive}`
                  : styles.scopeOption
              }>
              <input
                type="radio"
                name="preview-credential-scope"
                value={scopeKey}
                checked={scope === scopeKey}
                onChange={() => setScope(scopeKey)}
                className={styles.scopeInput}
              />
              <span>{SCOPE_LABELS[scopeKey]}</span>
            </label>
          ))}
        </div>
        <p className={styles.scopeDescription}>{SCOPE_DESCRIPTIONS[scope]}</p>
      </fieldset>
      <p className={styles.scopeCount} aria-live="polite">
        <Translate id="preview.scopes.count" values={{ count: visible.length }}>
          {'{count} credentials at this scope'}
        </Translate>
      </p>
      <ul className={styles.credentialList}>
        {visible.map((credential) => (
          <li key={credential.name} className={styles.credentialItem}>
            <div className={styles.credentialHeading}>
              <span className={styles.credentialName}>{credential.name}</span>
              <span className={styles.credentialVersion}>{credential.version}</span>
            </div>
            <code className={styles.credentialTarget}>{credential.target}</code>
            <span className={styles.credentialType}>{TYPE_LABELS[credential.type]}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
