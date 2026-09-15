import { useEffect, useMemo, useRef, useState } from "react";
import {
  Combobox,
  Field,
  MessageBar,
  MessageBarBody,
  Option,
  Spinner,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

import { useI18n } from "../i18n";
import type { WorkspaceResponse } from "../types";
import { filterWorkspaces, mergeWorkspaces, shouldClearWorkspaceDraft } from "./pickerModel";

interface Props {
  items: WorkspaceResponse[];
  loadItems?: (query: string) => Promise<WorkspaceResponse[]>;
  selectedId: string;
  onChange: (id: string) => void;
}

const useStyles = makeStyles({
  field: {
    width: "100%",
    maxWidth: "42rem",
  },
  combobox: {
    width: "100%",
  },
  option: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
  },
  optionMeta: {
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
  status: {
    marginTop: tokens.spacingVerticalXS,
  },
});

export function WorkspaceCombobox({ items, loadItems, selectedId, onChange }: Props) {
  const { t } = useI18n();
  const styles = useStyles();
  const requestIdRef = useRef(0);
  const knownSelectedRef = useRef<WorkspaceResponse | null>(null);
  const restoreSelectionRef = useRef<WorkspaceResponse | null>(null);
  const editingRef = useRef(false);
  const [remoteItems, setRemoteItems] = useState<WorkspaceResponse[]>([]);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);

  const sourceItems = useMemo(() => {
    const selected = knownSelectedRef.current?.workspace.id === selectedId
      ? knownSelectedRef.current
      : undefined;
    return mergeWorkspaces(items, remoteItems, selected ? [selected] : []);
  }, [items, remoteItems, selectedId]);
  const selected = sourceItems.find((item) => item.workspace.id === selectedId);
  const matches = useMemo(
    () => filterWorkspaces(sourceItems, editingRef.current ? query : "").slice(0, 30),
    [sourceItems, query],
  );

  useEffect(() => {
    if (selected) {
      knownSelectedRef.current = selected;
      if (!editingRef.current) setQuery(workspaceLabel(selected));
      return;
    }
    if (!selectedId && !editingRef.current) {
      knownSelectedRef.current = null;
      setQuery("");
    }
  }, [selected, selectedId]);

  useEffect(() => {
    if (!open || !loadItems) return;
    const requestId = ++requestIdRef.current;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setLoading(true);
      setLoadError(false);
      loadItems(editingRef.current ? query.trim() : "")
        .then((next) => {
          if (cancelled || requestIdRef.current !== requestId) return;
          setRemoteItems(next);
          setLoading(false);
        })
        .catch(() => {
          if (cancelled || requestIdRef.current !== requestId) return;
          setLoading(false);
          setLoadError(true);
        });
    }, 180);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [loadItems, open, query]);

  function choose(item: WorkspaceResponse) {
    editingRef.current = false;
    restoreSelectionRef.current = null;
    knownSelectedRef.current = item;
    setQuery(workspaceLabel(item));
    setOpen(false);
    onChange(item.workspace.id);
  }

  function clear() {
    editingRef.current = false;
    restoreSelectionRef.current = null;
    knownSelectedRef.current = null;
    setQuery("");
    setRemoteItems([]);
    setOpen(false);
    onChange("");
  }

  function restoreOrClearDraft() {
    if (!editingRef.current) return;
    const previous = restoreSelectionRef.current;
    editingRef.current = false;
    restoreSelectionRef.current = null;
    if (shouldClearWorkspaceDraft(query)) {
      setQuery("");
      if (selectedId) onChange("");
      return;
    }
    if (previous) {
      setQuery(workspaceLabel(previous));
      if (selectedId !== previous.workspace.id) onChange(previous.workspace.id);
    } else {
      setQuery("");
      if (selectedId) onChange("");
    }
  }

  function handleInputChange(value: string) {
    if (!editingRef.current) restoreSelectionRef.current = selected ?? knownSelectedRef.current;
    editingRef.current = true;
    setQuery(value);
    if (selectedId) onChange("");
    setOpen(true);
  }

  return (
    <Field className={styles.field} label={t("workspaces")} hint={t("workspaceAutocomplete")}>
      <Combobox
        className={styles.combobox}
        freeform
        clearable
        open={open}
        value={query}
        placeholder={t("workspaceAutocomplete")}
        onOpenChange={(_, data) => {
          setOpen(data.open);
          if (!data.open) restoreOrClearDraft();
        }}
        onChange={(event) => handleInputChange(event.currentTarget.value)}
        onOptionSelect={(_, data) => {
          if (!data.optionValue) {
            clear();
            return;
          }
          const item = sourceItems.find((candidate) => candidate.workspace.id === data.optionValue);
          if (item) choose(item);
        }}
        selectedOptions={selectedId ? [selectedId] : []}
        aria-label={t("workspaceAutocomplete")}
      >
        {matches.map((item) => (
          <Option key={item.workspace.id} value={item.workspace.id} text={workspaceLabel(item)}>
            <span className={styles.option}>
              <span>{item.workspace.name}</span>
              <span className={styles.optionMeta}>{item.workspace.short_id} · {item.workspace.workspace_user}</span>
            </span>
          </Option>
        ))}
      </Combobox>
      {loading && <Spinner className={styles.status} size="tiny" label={t("loadingWorkspaces")} />}
      {loadError && <MessageBar className={styles.status} intent="error"><MessageBarBody>{t("workspaceSearchError")}</MessageBarBody></MessageBar>}
      {!loading && !loadError && open && editingRef.current && matches.length === 0 && <MessageBar className={styles.status} intent="info"><MessageBarBody>{t("noMatchingWorkspaces")}</MessageBarBody></MessageBar>}
    </Field>
  );
}

function workspaceLabel(item: WorkspaceResponse) {
  return `${item.workspace.name} · ${item.workspace.short_id}`;
}
