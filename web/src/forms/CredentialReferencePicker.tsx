import { useMemo, useState } from "react";
import {
  Badge,
  Button,
  Checkbox,
  Field,
  Input,
  Popover,
  PopoverSurface,
  PopoverTrigger,
  Text,
  Tooltip,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { DismissRegular, KeyRegular, SearchRegular } from "@fluentui/react-icons";

import { useI18n } from "../i18n";
import type { StoredInjection } from "../types";
import { filterReferenceItems, injectionKindLabel } from "./pickerModel";

interface Props {
  organizationItems: StoredInjection[];
  userItems: StoredInjection[];
  organizationSelected: string[];
  userSelected: string[];
  onOrganizationSelected: (keys: string[]) => void;
  onUserSelected: (keys: string[]) => void;
}

const useStyles = makeStyles({
  root: {
    width: "100%",
    maxWidth: "42rem",
  },
  trigger: {
    width: "100%",
    justifyContent: "space-between",
  },
  surface: {
    width: "min(34rem, calc(100vw - 2rem))",
    maxWidth: "34rem",
    padding: tokens.spacingHorizontalM,
  },
  toolbar: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    alignItems: "center",
    marginBottom: tokens.spacingVerticalS,
  },
  search: {
    flex: 1,
    minWidth: 0,
  },
  groups: {
    display: "grid",
    gap: tokens.spacingVerticalM,
    maxHeight: "min(24rem, 52vh)",
    overflowY: "auto",
  },
  group: {
    display: "grid",
    gap: tokens.spacingVerticalXS,
  },
  groupTitle: {
    display: "flex",
    gap: tokens.spacingHorizontalXS,
    alignItems: "center",
  },
  options: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
  },
  option: {
    display: "grid",
    gridTemplateColumns: "auto minmax(0, 1fr)",
    gap: tokens.spacingHorizontalS,
    alignItems: "start",
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalXS}`,
    borderRadius: tokens.borderRadiusMedium,
    ":hover": {
      backgroundColor: tokens.colorSubtleBackgroundHover,
    },
  },
  optionDetails: {
    display: "grid",
    minWidth: 0,
    gap: tokens.spacingVerticalXXS,
  },
  optionMeta: {
    overflow: "hidden",
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  empty: {
    color: tokens.colorNeutralForeground2,
    padding: tokens.spacingVerticalS,
  },
  help: {
    marginTop: tokens.spacingVerticalS,
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
});

export function CredentialReferencePicker({
  organizationItems,
  userItems,
  organizationSelected,
  userSelected,
  onOrganizationSelected,
  onUserSelected,
}: Props) {
  const { t } = useI18n();
  const styles = useStyles();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const filteredOrganization = useMemo(() => filterReferenceItems(organizationItems, search, t), [organizationItems, search, t]);
  const filteredUser = useMemo(() => filterReferenceItems(userItems, search, t), [userItems, search, t]);
  const selectedCount = organizationItems.filter((item) => item.locked || organizationSelected.includes(item.key)).length
    + userItems.filter((item) => userSelected.includes(item.key)).length;

  function clearAll() {
    onOrganizationSelected([]);
    onUserSelected([]);
  }

  function close() {
    setOpen(false);
    setSearch("");
  }

  return (
    <Field className={styles.root} label={t("organizationAndUserCredentials")}>
      <Popover open={open} onOpenChange={(_, data) => { setOpen(data.open); if (!data.open) setSearch(""); }}>
        <PopoverTrigger disableButtonEnhancement>
          <Button
            className={styles.trigger}
            type="button"
            appearance="outline"
            aria-label={t("organizationAndUserCredentials")}
            aria-expanded={open}
            icon={<KeyRegular aria-hidden="true" />}
          >
            <span>{selectedCount > 0 ? `${t("selectedReferences")} · ${selectedCount}` : t("chooseCredentialReferences")}</span>
          </Button>
        </PopoverTrigger>
        <PopoverSurface className={styles.surface} tabIndex={-1}>
          <div className={styles.toolbar}>
            <Input
              className={styles.search}
              value={search}
              onChange={(event) => setSearch(event.currentTarget.value)}
              placeholder={t("searchCredentialReferences")}
              aria-label={t("searchCredentialReferences")}
              contentBefore={<SearchRegular aria-hidden="true" />}
              contentAfter={search ? <Button appearance="transparent" size="small" icon={<DismissRegular aria-hidden="true" />} aria-label={t("clearSelectedReferences")} onClick={() => setSearch("")} /> : undefined}
            />
            <Button type="button" appearance="subtle" onClick={clearAll}>{t("clearSelectedReferences")}</Button>
          </div>
          <div className={styles.groups}>
            <ReferenceGroup
              title={t("scopeOrganization")}
              items={filteredOrganization}
              selected={organizationSelected}
              onToggle={(key) => toggle(key, organizationSelected, onOrganizationSelected)}
              emptyLabel={t("noMatchingCredentials")}
            />
            <ReferenceGroup
              title={t("scopeUser")}
              items={filteredUser}
              selected={userSelected}
              onToggle={(key) => toggle(key, userSelected, onUserSelected)}
              emptyLabel={t("noMatchingCredentials")}
            />
          </div>
          <Text className={styles.help}>{t("selectedReferenceHelp")}</Text>
          <Button type="button" appearance="subtle" onClick={close}>{t("close")}</Button>
        </PopoverSurface>
      </Popover>
    </Field>
  );
}

function ReferenceGroup({
  title,
  items,
  selected,
  onToggle,
  emptyLabel,
}: {
  title: string;
  items: StoredInjection[];
  selected: string[];
  onToggle: (key: string) => void;
  emptyLabel: string;
}) {
  const { t } = useI18n();
  const styles = useStyles();
  return (
    <section className={styles.group} aria-label={title}>
      <div className={styles.groupTitle}>
        <Text weight="semibold">{title}</Text>
        <Badge appearance="tint">{items.length}</Badge>
      </div>
      <div className={styles.options}>
        {items.length === 0 && <Text className={styles.empty}>{emptyLabel}</Text>}
        {items.map((item) => {
          const locked = item.locked;
          const label = (
            <span className={styles.optionDetails}>
              <span>{item.key}</span>
              <span className={styles.optionMeta}>{injectionKindLabel(item, t)} · {item.target}{locked ? ` · ${t("locked")}` : ""}</span>
            </span>
          );
          const checkbox = <Checkbox checked={locked || selected.includes(item.key)} disabled={locked} onChange={() => onToggle(item.key)} label={label} />;
          return <div className={styles.option} key={item.key}>{locked ? <Tooltip content={t("lockedHelp")} relationship="description">{checkbox}</Tooltip> : checkbox}</div>;
        })}
      </div>
    </section>
  );
}

function toggle(key: string, selected: string[], update: (keys: string[]) => void) {
  update(selected.includes(key) ? selected.filter((item) => item !== key) : [...selected, key]);
}
