import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";

import { useI18n } from "../i18n";
import type { WorkspaceResponse } from "../types";
import { filterWorkspaces, mergeWorkspaces } from "./pickerModel";

interface Props {
  items: WorkspaceResponse[];
  loadItems?: (query: string) => Promise<WorkspaceResponse[]>;
  selectedId: string;
  onChange: (id: string) => void;
}

export function WorkspaceCombobox({ items, loadItems, selectedId, onChange }: Props) {
  const { t } = useI18n();
  const listId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const requestIdRef = useRef(0);
  const knownSelectedRef = useRef<WorkspaceResponse | null>(null);
  const restoreSelectionRef = useRef<WorkspaceResponse | null>(null);
  const editingRef = useRef(false);
  const [remoteItems, setRemoteItems] = useState<WorkspaceResponse[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const baseItems = useMemo(() => mergeWorkspaces(items, remoteItems), [items, remoteItems]);
  const selectedFromItems = useMemo(
    () => baseItems.find((item) => item.workspace.id === selectedId),
    [baseItems, selectedId],
  );
  const selected = selectedFromItems
    ?? (knownSelectedRef.current?.workspace.id === selectedId ? knownSelectedRef.current : undefined);
  const [query, setQuery] = useState(() => selected ? workspaceLabel(selected) : "");
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const preservedSelection = selected ?? restoreSelectionRef.current;
  const sourceItems = useMemo(
    () => mergeWorkspaces(items, remoteItems, preservedSelection ? [preservedSelection] : []),
    [items, remoteItems, preservedSelection],
  );
  const matches = useMemo(() => filterWorkspaces(sourceItems, query).slice(0, 30), [sourceItems, query]);

  // Remember a selected option even when a remote response does not contain
  // it. This keeps the current value visible while another query is loading.
  useEffect(() => {
    if (selectedFromItems) {
      knownSelectedRef.current = selectedFromItems;
      return;
    }
    if (selectedId && knownSelectedRef.current?.workspace.id !== selectedId) {
      knownSelectedRef.current = null;
    } else if (!selectedId && !editingRef.current && !restoreSelectionRef.current) {
      knownSelectedRef.current = null;
    }
  }, [selectedFromItems, selectedId]);

  // The input is normally a projection of the committed selection. During an
  // edit, however, selectedId is cleared immediately and the draft text must
  // not be erased by this synchronization effect.
  useEffect(() => {
    if (selected) {
      setQuery(workspaceLabel(selected));
      editingRef.current = false;
      restoreSelectionRef.current = null;
    } else if (!selectedId && !editingRef.current && !restoreSelectionRef.current) {
      setQuery("");
    }
  }, [selectedId, selected?.workspace.name, selected?.workspace.short_id]);

  useEffect(() => {
    if (!loadItems || !open) return;
    const requestId = ++requestIdRef.current;
    let cancelled = false;
    setLoading(true);
    setLoadError(false);
    // A response for an older query should not remain as a visible option
    // while the newer request is in flight. Local items and the preserved
    // selection still remain available through sourceItems.
    setRemoteItems([]);
    const timer = window.setTimeout(() => {
      // The API currently searches workspace names. The displayed committed
      // label is not a search query, so opening a selected picker loads the
      // unfiltered page; user edits send the actual draft text.
      const requestQuery = editingRef.current ? query.trim() : "";
      loadItems(requestQuery)
        .then((next) => {
          if (cancelled || requestIdRef.current !== requestId) return;
          setRemoteItems(next);
          setLoading(false);
        })
        .catch(() => {
          if (cancelled || requestIdRef.current !== requestId) return;
          setLoadError(true);
          setLoading(false);
        });
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      if (requestIdRef.current === requestId) {
        requestIdRef.current += 1;
        setLoading(false);
      }
    };
  }, [loadItems, open, query]);

  useEffect(() => {
    if (!open) return;
    const pointerDown = (event: PointerEvent) => {
      if (rootRef.current?.contains(event.target as Node)) return;
      restoreOrClearDraft();
      setOpen(false);
    };
    document.addEventListener("pointerdown", pointerDown);
    return () => document.removeEventListener("pointerdown", pointerDown);
  }, [open, selectedId]);

  function choose(item: WorkspaceResponse) {
    editingRef.current = false;
    restoreSelectionRef.current = null;
    setQuery(workspaceLabel(item));
    setActiveIndex(-1);
    setOpen(false);
    onChange(item.workspace.id);
  }

  function clear() {
    // Clearing is an explicit user action, so there is no prior selection to
    // restore on blur or Escape.
    editingRef.current = false;
    restoreSelectionRef.current = null;
    setQuery("");
    setActiveIndex(-1);
    setOpen(true);
    onChange("");
  }

  function restoreOrClearDraft() {
    if (!editingRef.current) return;
    const previous = restoreSelectionRef.current;
    editingRef.current = false;
    restoreSelectionRef.current = null;
    setActiveIndex(-1);
    if (previous) {
      setQuery(workspaceLabel(previous));
      if (selectedId !== previous.workspace.id) onChange(previous.workspace.id);
    } else {
      setQuery("");
      if (selectedId) onChange("");
    }
  }

  function keyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setOpen(true);
      setActiveIndex((value) => Math.min(value + 1, matches.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((value) => Math.max(value - 1, 0));
    } else if (event.key === "Enter" && open && matches[activeIndex]) {
      event.preventDefault();
      choose(matches[activeIndex]);
    } else if (event.key === "Escape") {
      event.preventDefault();
      restoreOrClearDraft();
      setOpen(false);
    }
  }

  return <label className="compact-workspace-picker">
    <span>{t("workspaces")}</span>
    <div className="compact-combobox" ref={rootRef}>
      <input
        type="text"
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls={listId}
        aria-activedescendant={open && matches[activeIndex] ? `${listId}-${matches[activeIndex].workspace.id}` : undefined}
        value={query}
        onFocus={() => setOpen(true)}
        onBlur={(event) => {
          // Option and clear-button clicks keep focus in the combobox. This
          // guard also protects browsers that still emit blur for those clicks.
          if (event.relatedTarget && rootRef.current?.contains(event.relatedTarget as Node)) return;
          restoreOrClearDraft();
          setOpen(false);
        }}
        onChange={(event) => {
          const value = event.target.value;
          if (!editingRef.current) {
            restoreSelectionRef.current = selected ?? knownSelectedRef.current ?? null;
          }
          editingRef.current = true;
          if (selectedId) onChange("");
          setQuery(value);
          setActiveIndex(-1);
          setOpen(true);
        }}
        onKeyDown={keyDown}
        placeholder={t("workspaceAutocomplete")}
        autoComplete="off"
      />
      {query && <button type="button" aria-label={t("clearWorkspaceSelection")} onMouseDown={(event) => event.preventDefault()} onClick={clear}>×</button>}
      {open && <div id={listId} className="compact-combobox-options" role="listbox">
        {loading && <p role="status">{t("loadingWorkspaces")}</p>}
        {loadError && <p role="alert">{t("workspaceSearchError")}</p>}
        {!loading && !loadError && matches.length === 0 && <p>{t("noMatchingWorkspaces")}</p>}
        {matches.map((item, index) => <button
          type="button"
          id={`${listId}-${item.workspace.id}`}
          role="option"
          aria-selected={item.workspace.id === selectedId}
          className={index === activeIndex ? "active" : ""}
          key={item.workspace.id}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => choose(item)}
        >
          <span>{item.workspace.name}</span><code>{item.workspace.short_id}</code>
        </button>)}
      </div>}
    </div>
  </label>;
}

function workspaceLabel(item: WorkspaceResponse) {
  return `${item.workspace.name} · ${item.workspace.short_id}`;
}
