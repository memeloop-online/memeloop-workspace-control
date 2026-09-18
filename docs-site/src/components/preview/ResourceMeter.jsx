import React from 'react';
import { translate } from '@docusaurus/Translate';
import styles from './preview.module.css';

const METER_LABELS = {
  cpu: translate({ id: 'preview.meters.cpu', message: 'CPU' }),
  memory: translate({ id: 'preview.meters.memory', message: 'Memory' }),
  disk: translate({ id: 'preview.meters.disk', message: 'Persistent disk' }),
  ephemeral: translate({ id: 'preview.meters.ephemeral', message: 'Ephemeral storage' }),
};

export default function ResourceMeter({ meter }) {
  const label = METER_LABELS[meter.key] ?? meter.key;
  const valueText = translate(
    {
      id: 'preview.meters.value',
      message: '{used} of {limit} ({percent} percent)',
    },
    { used: meter.used, limit: meter.limit, percent: meter.percent },
  );
  return (
    <div className={styles.meter}>
      <div className={styles.meterHeader}>
        <span className={styles.meterLabel}>{label}</span>
        <span className={styles.meterValue}>
          {meter.used} / {meter.limit}
        </span>
      </div>
      <div
        className={styles.meterTrack}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={meter.percent}
        aria-valuetext={valueText}>
        <div
          className={
            meter.percent >= 80 ? `${styles.meterFill} ${styles.meterFillHigh}` : styles.meterFill
          }
          style={{ width: `${Math.max(meter.percent, meter.percent > 0 ? 2 : 0)}%` }}
        />
      </div>
      <span className={styles.meterPercent}>{meter.percent}%</span>
    </div>
  );
}
