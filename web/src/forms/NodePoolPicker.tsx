import { useMemo, useState } from "react";
import { Combobox, Option, Text, makeStyles, tokens } from "@fluentui/react-components";
import type { ComboboxProps } from "@fluentui/react-components";

import { useI18n } from "../i18n";
import type { AvailableNodePool } from "../types";

const useStyles = makeStyles({
  combobox: { minWidth: 0, width: "100%" },
  option: { display: "grid", gap: tokens.spacingVerticalXXS },
  optionName: { color: tokens.colorNeutralForeground2, fontFamily: tokens.fontFamilyMonospace, fontSize: tokens.fontSizeBase200 },
});

export function nodePoolDisplayName(pools: AvailableNodePool[], name: string): string {
  return pools.find((pool) => pool.name === name)?.display_name ?? name;
}

function matches(pool: AvailableNodePool, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return true;
  return pool.name.toLowerCase().includes(needle) || pool.display_name.toLowerCase().includes(needle);
}

interface PoolOptionListProps {
  pools: AvailableNodePool[];
  /** Extra pool names (for example from an existing record) that must stay selectable. */
  extraNames?: string[];
  query: string;
}

function PoolOptions({ pools, extraNames = [], query }: PoolOptionListProps) {
  const styles = useStyles();
  const { t } = useI18n();
  const known = useMemo(() => new Set(pools.map((pool) => pool.name)), [pools]);
  const extras = extraNames.filter((name) => name && !known.has(name));
  const visible = pools.filter((pool) => matches(pool, query));
  return <>
    {visible.map((pool) => (
      <Option key={pool.name} value={pool.name} text={pool.display_name}>
        <span className={styles.option}><Text>{pool.display_name}</Text><Text className={styles.optionName}>{pool.name}</Text></span>
      </Option>
    ))}
    {extras.map((name) => <Option key={name} value={name} text={name}>{name}</Option>)}
    {visible.length === 0 && extras.length === 0 && <Option disabled value="__no_match" text={t("noMatchingNodePools")}>{t("noMatchingNodePools")}</Option>}
  </>;
}

interface AllowedNodePoolsPickerProps {
  pools: AvailableNodePool[];
  selected: string[];
  onChange: (selected: string[]) => void;
  disabled?: boolean;
  id?: string;
}

/** Searchable multi-select for the node pools a template allows. */
export function AllowedNodePoolsPicker({ pools, selected, onChange, disabled = false, id }: AllowedNodePoolsPickerProps) {
  const { t } = useI18n();
  const styles = useStyles();
  const [query, setQuery] = useState("");
  const displayValue = selected.map((name) => nodePoolDisplayName(pools, name)).join(", ");
  const onOptionSelect: ComboboxProps["onOptionSelect"] = (_, data) => {
    onChange(data.selectedOptions);
    setQuery("");
  };
  return (
    <Combobox
      id={id}
      className={styles.combobox}
      multiselect
      editable
      disabled={disabled}
      placeholder={t("selectNodePools")}
      value={query || displayValue}
      selectedOptions={selected}
      onChange={(event) => setQuery(event.currentTarget.value)}
      onOptionSelect={onOptionSelect}
      aria-label={t("allowedNodePools")}
    >
      <PoolOptions pools={pools} extraNames={selected} query={query} />
    </Combobox>
  );
}

interface NodePoolSelectProps {
  pools: AvailableNodePool[];
  /** Pool names that may be chosen; defaults to every known pool. */
  allowed?: string[];
  value: string;
  onChange: (name: string) => void;
  disabled?: boolean;
  placeholder?: string;
  id?: string;
  "aria-label"?: string;
}

/** Searchable single-select for one node pool out of an allowed set. */
export function NodePoolSelect({ pools, allowed, value, onChange, disabled = false, placeholder, id, ...rest }: NodePoolSelectProps) {
  const { t } = useI18n();
  const styles = useStyles();
  const [query, setQuery] = useState("");
  const scoped = useMemo(() => {
    if (!allowed) return pools;
    const allowedSet = new Set(allowed);
    return pools.filter((pool) => allowedSet.has(pool.name));
  }, [pools, allowed]);
  const extras = useMemo(() => {
    const known = new Set(scoped.map((pool) => pool.name));
    return (allowed ?? []).filter((name) => !known.has(name));
  }, [scoped, allowed]);
  const onOptionSelect: ComboboxProps["onOptionSelect"] = (_, data) => {
    if (data.optionValue) onChange(data.optionValue);
    setQuery("");
  };
  return (
    <Combobox
      id={id}
      className={styles.combobox}
      editable
      disabled={disabled}
      placeholder={placeholder ?? t("selectNodePool")}
      value={query || (value ? nodePoolDisplayName(pools, value) : "")}
      selectedOptions={value ? [value] : []}
      onChange={(event) => setQuery(event.currentTarget.value)}
      onOptionSelect={onOptionSelect}
      {...rest}
    >
      <PoolOptions pools={scoped} extraNames={[...extras, ...(value ? [value] : [])]} query={query} />
    </Combobox>
  );
}
