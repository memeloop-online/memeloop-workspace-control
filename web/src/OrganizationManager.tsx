import {
  Button,
  Field,
  Input,
  Text,
} from "@fluentui/react-components";

import type { Organization } from "./types";
import { useI18n } from "./i18n";
import { AdminCard, AdminToolbar, SaveButton, useAdminStyles } from "./admin/fluentAdmin";

export interface OrganizationManagerProps {
  organization: Organization | undefined;
  organizationName: string;
  newOrganizationName: string;
  canCreate: boolean;
  canEdit: boolean;
  canDelete: boolean;
  onOrganizationNameChange: (value: string) => void;
  onNewOrganizationNameChange: (value: string) => void;
  onSave: () => void;
  onDelete: () => void;
  onCreate: () => void;
}

export function OrganizationManager({
  organization,
  organizationName,
  newOrganizationName,
  canCreate,
  canEdit,
  canDelete,
  onOrganizationNameChange,
  onNewOrganizationNameChange,
  onSave,
  onDelete,
  onCreate,
}: OrganizationManagerProps) {
  const { t } = useI18n();
  const styles = useAdminStyles();

  return <AdminCard title={t("organizationManagement")}>
    {organization ? <div className={styles.stack}>
      <Field label={t("currentOrganizationName")}>
        <Input value={organizationName} onChange={(event) => onOrganizationNameChange(event.target.value)} />
      </Field>
      <AdminToolbar action={<div className={styles.actions}>
        <SaveButton disabled={!canEdit || !organizationName.trim() || organizationName.trim() === organization.name} onClick={onSave}>{t("saveOrganization")}</SaveButton>
        {canDelete && <Button appearance="secondary" onClick={onDelete}>{t("deleteOrganization")}</Button>}
      </div>}>
        <Text size={300}>{organization.name}</Text>
      </AdminToolbar>
    </div> : <Text>{t("organizationUnavailable")}</Text>}
    {canCreate && <div className={styles.stack}>
      <Field label={t("newOrganizationName")}>
        <Input value={newOrganizationName} onChange={(event) => onNewOrganizationNameChange(event.target.value)} />
      </Field>
      <div className={styles.actions}>
        <SaveButton disabled={!newOrganizationName.trim()} onClick={onCreate}>{t("createOrganization")}</SaveButton>
      </div>
    </div>}
  </AdminCard>;
}
