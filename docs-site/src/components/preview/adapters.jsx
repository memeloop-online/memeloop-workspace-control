import React, { useMemo, useState } from 'react';
import {
  Card,
  Field,
  FluentProvider,
  Select,
  Text,
  webDarkTheme,
  webLightTheme,
} from '@fluentui/react-components';
import { useColorMode } from '@docusaurus/theme-common';
import {
  CredentialList,
  CredentialScopeTabs,
  ResourceMeter,
  StorageMeter,
  WorkspaceStatusCard,
} from '@mwc/shared';
import { CREDENTIALS, CREDENTIAL_SCOPES, WORKSPACES } from './fixtures';
import { PREVIEW_LABELS, formatCredentialCount, formatWorkspaceMeta } from './labels';
import styles from './preview.module.css';

function ProductTheme({ children }) {
  const { colorMode } = useColorMode();
  return <FluentProvider theme={colorMode === 'dark' ? webDarkTheme : webLightTheme}>{children}</FluentProvider>;
}

function meterLabels() {
  return {
    telemetryStale: PREVIEW_LABELS.meter.telemetryStale,
    telemetryDisabled: PREVIEW_LABELS.meter.telemetryDisabled,
    telemetryAvailable: PREVIEW_LABELS.meter.telemetryAvailable,
    telemetryUnavailable: PREVIEW_LABELS.meter.telemetryUnavailable,
    nodeLocalTelemetryUnavailable: PREVIEW_LABELS.meter.nodeLocalTelemetryUnavailable,
    pressureCritical: PREVIEW_LABELS.meter.pressureCritical,
    pressureWarning: PREVIEW_LABELS.meter.pressureWarning,
    observedAt: PREVIEW_LABELS.meter.observedAt,
    usageNotObserved: PREVIEW_LABELS.meter.usageNotObserved,
    storageUsage: PREVIEW_LABELS.meter.storageUsage,
    usageOfLimit: PREVIEW_LABELS.meter.usageOfLimit,
  };
}

function renderMeters(workspace) {
  const storageLabels = meterLabels();
  return workspace.meters.map((meter) => {
    if (meter.kind === 'storage') {
      return (
        <StorageMeter
          key={meter.key}
          label={meter.key === 'disk' ? PREVIEW_LABELS.meter.disk : PREVIEW_LABELS.meter.temporary}
          telemetry={meter.telemetry}
          configuredGiB={meter.configuredGiB}
          locale={globalThis.document?.documentElement?.lang || 'en'}
          labels={storageLabels}
        />
      );
    }
    return (
      <ResourceMeter
        key={meter.key}
        label={meter.key === 'cpu' ? PREVIEW_LABELS.meter.cpu : PREVIEW_LABELS.meter.memory}
        actual={meter.actual}
        requested={meter.requested}
        percent={meter.percent}
        unavailableLabel={PREVIEW_LABELS.meter.unavailable}
        usageOfLimitLabel={PREVIEW_LABELS.meter.usageOfLimit}
      />
    );
  });
}

export function WorkspacePreview() {
  const [selectedId, setSelectedId] = useState(WORKSPACES[0].id);
  const selected = WORKSPACES.find((workspace) => workspace.id === selectedId) ?? WORKSPACES[0];

  return (
    <ProductTheme>
      <Card className={styles.productPanel} appearance="outline">
        <Field label={PREVIEW_LABELS.workspacePicker}>
          <Select value={selected.id} onChange={(event) => setSelectedId(event.currentTarget.value)}>
            {WORKSPACES.map((workspace) => (
              <option key={workspace.id} value={workspace.id}>{workspace.name}</option>
            ))}
          </Select>
        </Field>
        <WorkspaceStatusCard
          title={selected.name}
          shortId={selected.id}
          state={selected.status}
          stateLabels={PREVIEW_LABELS.state}
          metadata={[formatWorkspaceMeta(selected)]}
          meters={renderMeters(selected)}
        />
      </Card>
    </ProductTheme>
  );
}

function toCredentialItem(credential) {
  return {
    key: credential.name,
    target: credential.target,
    version: credential.version,
    kind: credential.kind,
    sensitive: credential.sensitive,
    locked: credential.locked,
    kindLabel: PREVIEW_LABELS.credentialKind[credential.kind],
  };
}

export function CredentialPreview() {
  const [scope, setScope] = useState('user');
  const [search, setSearch] = useState('');
  const [selectedKey, setSelectedKey] = useState(null);
  const visible = useMemo(() => CREDENTIALS
    .filter((credential) => credential.scope === scope)
    .map(toCredentialItem)
    .filter((credential) => `${credential.key} ${credential.target}`.toLowerCase().includes(search.trim().toLowerCase())), [scope, search]);

  return (
    <ProductTheme>
      <Card className={styles.productPanel} appearance="outline">
        <CredentialScopeTabs
          scopes={CREDENTIAL_SCOPES}
          selected={scope}
          labels={PREVIEW_LABELS.scope}
          ariaLabel={PREVIEW_LABELS.scope.ariaLabel}
          onChange={(nextScope) => { setScope(nextScope); setSelectedKey(null); }}
        />
        <Text className={styles.scopeCount}>{formatCredentialCount(visible.length)}</Text>
        <CredentialList
          items={visible}
          selectedKey={selectedKey}
          search={search}
          labels={{
            title: PREVIEW_LABELS.scope.title,
            emptyLabel: PREVIEW_LABELS.scope.empty,
            searchPlaceholder: PREVIEW_LABELS.scope.searchPlaceholder,
            clearSearch: PREVIEW_LABELS.scope.clearSearch,
            loading: PREVIEW_LABELS.scope.loading,
            noFilteredResults: PREVIEW_LABELS.scope.noFilteredResults,
            sensitiveValue: PREVIEW_LABELS.scope.sensitiveValue,
            visibleConfiguration: PREVIEW_LABELS.scope.visibleConfiguration,
            lockedState: PREVIEW_LABELS.scope.lockedState,
          }}
          onSearchChange={setSearch}
          onSelect={(item) => setSelectedKey((current) => current === item.key ? null : item.key)}
        />
      </Card>
    </ProductTheme>
  );
}
