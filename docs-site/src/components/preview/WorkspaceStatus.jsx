import React, { useState } from 'react';
import Translate, { translate } from '@docusaurus/Translate';
import ResourceMeter from './ResourceMeter';
import { WORKSPACES } from './fixtures';
import styles from './preview.module.css';

const STATUS_LABELS = {
  ready: translate({ id: 'preview.status.ready', message: 'Ready' }),
  starting: translate({ id: 'preview.status.starting', message: 'Starting' }),
  stopped: translate({ id: 'preview.status.stopped', message: 'Stopped' }),
};

export default function WorkspaceStatus() {
  const [selectedId, setSelectedId] = useState(WORKSPACES[0].id);
  const selected = WORKSPACES.find((workspace) => workspace.id === selectedId) ?? WORKSPACES[0];

  return (
    <div className={styles.panel}>
      <p className={styles.panelLabel} id="preview-workspace-picker-label">
        <Translate id="preview.status.selectLabel">Choose a workspace</Translate>
      </p>
      <div
        className={styles.workspacePicker}
        role="group"
        aria-labelledby="preview-workspace-picker-label">
        {WORKSPACES.map((workspace) => (
          <button
            key={workspace.id}
            type="button"
            className={
              workspace.id === selectedId
                ? `${styles.workspaceChip} ${styles.workspaceChipActive}`
                : styles.workspaceChip
            }
            aria-pressed={workspace.id === selectedId}
            onClick={() => setSelectedId(workspace.id)}>
            <span className={styles.workspaceChipName}>{workspace.name}</span>
            <span
              className={`${styles.statusBadge} ${styles[`status_${workspace.status}`]}`}>
              {STATUS_LABELS[workspace.status]}
            </span>
          </button>
        ))}
      </div>
      <div className={styles.workspaceDetail} aria-live="polite">
        <div className={styles.workspaceDetailHeader}>
          <div>
            <h3 className={styles.workspaceName}>{selected.name}</h3>
            <p className={styles.workspaceMeta}>
              <Translate
                id="preview.status.meta"
                values={{ id: selected.id, template: selected.template }}>
                {'Short ID {id} · Template {template}'}
              </Translate>
            </p>
          </div>
          <span className={`${styles.statusBadge} ${styles[`status_${selected.status}`]}`}>
            {STATUS_LABELS[selected.status]}
          </span>
        </div>
        <div className={styles.meterGrid}>
          {selected.meters.map((meter) =>
            meter.released ? (
              <div key={meter.key} className={styles.meter}>
                <div className={styles.meterHeader}>
                  <span className={styles.meterLabel}>
                    <Translate id="preview.meters.ephemeral">Ephemeral storage</Translate>
                  </span>
                  <span className={styles.meterValue}>
                    <Translate id="preview.meters.ephemeral.released">
                      Released on stop
                    </Translate>
                  </span>
                </div>
              </div>
            ) : (
              <ResourceMeter key={meter.key} meter={meter} />
            ),
          )}
        </div>
      </div>
    </div>
  );
}
