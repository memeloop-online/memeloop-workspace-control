import { CredentialList } from "../shared";
import { useI18n } from "../i18n";
import type { StoredInjection } from "../types";
import { filterReferenceItems, injectionKindLabel } from "../forms/pickerModel";

interface Props {
  items: StoredInjection[];
  selectedKey: string | null;
  search: string;
  loading?: boolean;
  title: string;
  emptyLabel: string;
  onSearchChange: (value: string) => void;
  onSelect: (item: StoredInjection) => void;
}

/** Product adapter for the shared credential list. */
export function InjectionList({
  items,
  selectedKey,
  search,
  loading = false,
  title,
  emptyLabel,
  onSearchChange,
  onSelect,
}: Props) {
  const { t } = useI18n();
  const filteredItems = filterReferenceItems(items, search, t);
  const viewItems = filteredItems.map((item) => ({
    key: item.key,
    target: item.target,
    version: item.version,
    kind: item.kind,
    sensitive: item.sensitive,
    locked: item.locked,
    kindLabel: injectionKindLabel(item, t),
  }));

  return (
    <CredentialList
      items={viewItems}
      selectedKey={selectedKey}
      search={search}
      loading={loading}
      labels={{
        title,
        emptyLabel,
        searchPlaceholder: t("searchCredentials"),
        clearSearch: t("clearSearch"),
        loading: t("loading"),
        noFilteredResults: t("noCredentialsFiltered"),
        sensitiveValue: t("sensitiveValue"),
        visibleConfiguration: t("visibleConfiguration"),
        lockedState: t("lockedState"),
      }}
      onSearchChange={onSearchChange}
      onSelect={(selected) => {
        const item = filteredItems.find((candidate) => candidate.key === selected.key);
        if (item) onSelect(item);
      }}
    />
  );
}
