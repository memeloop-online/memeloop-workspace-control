import { Tab, TabList, makeStyles } from "@fluentui/react-components";

export interface CredentialScopeTabsProps<T extends string> {
  /** Scope values rendered as tabs, in order. */
  scopes: readonly T[];
  selected: T;
  /** Visible label per scope; falls back to the raw scope value when missing. */
  labels: Partial<Record<T, string>>;
  ariaLabel: string;
  onChange: (scope: T) => void;
}

const useStyles = makeStyles({
  tabs: {
    width: "fit-content",
    maxWidth: "100%",
  },
});

/** Scope switcher for credential surfaces (organization / user / workspace). */
export function CredentialScopeTabs<T extends string>({ scopes, selected, labels, ariaLabel, onChange }: CredentialScopeTabsProps<T>) {
  const styles = useStyles();
  return (
    <TabList
      className={styles.tabs}
      selectedValue={selected}
      onTabSelect={(_, data) => onChange(data.value as T)}
      aria-label={ariaLabel}
    >
      {scopes.map((scope) => <Tab key={scope} value={scope}>{labels[scope] ?? scope}</Tab>)}
    </TabList>
  );
}
