import type { Organization } from "./types";
import { useI18n } from "./i18n";

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
  return <div className="system-card">
    <h3>{t("organizationManagement")}</h3>
    {organization ? <>
      <label>{t("currentOrganizationName")}<input value={organizationName} onChange={(event) => onOrganizationNameChange(event.target.value)} /></label>
      <div className="form-actions"><button className="button" disabled={!canEdit || !organizationName.trim() || organizationName.trim() === organization.name} onClick={onSave}>{t("saveOrganization")}</button>{canDelete && <button className="button danger" onClick={onDelete}>{t("deleteOrganization")}</button>}</div>
    </> : <p>{t("organizationUnavailable")}</p>}
    {canCreate && <div className="quota-editor"><label>{t("newOrganizationName")}<input value={newOrganizationName} onChange={(event) => onNewOrganizationNameChange(event.target.value)} /></label><div className="quota-actions"><button className="button primary" disabled={!newOrganizationName.trim()} onClick={onCreate}>{t("createOrganization")}</button></div></div>}
  </div>;
}
