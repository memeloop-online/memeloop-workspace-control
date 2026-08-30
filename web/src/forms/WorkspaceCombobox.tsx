import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";

import { useI18n } from "../i18n";
import type { WorkspaceResponse } from "../types";
import { filterWorkspaces } from "./pickerModel";

interface Props {
  items: WorkspaceResponse[];
  selectedId: string;
  onChange: (id: string) => void;
}

export function WorkspaceCombobox({ items, selectedId, onChange }: Props) {
  const { t } = useI18n();
  const listId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const selected = items.find((item) => item.workspace.id === selectedId);
  const [query, setQuery] = useState(selected ? workspaceLabel(selected) : "");
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const matches = useMemo(() => filterWorkspaces(items, query).slice(0, 30), [items, query]);

  useEffect(() => {
    setQuery(selected ? workspaceLabel(selected) : "");
  }, [selectedId, selected?.workspace.name, selected?.workspace.short_id]);

  useEffect(() => {
    if (!open) return;
    const pointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", pointerDown);
    return () => document.removeEventListener("pointerdown", pointerDown);
  }, [open]);

  function choose(item: WorkspaceResponse) {
    onChange(item.workspace.id);
    setQuery(workspaceLabel(item));
    setOpen(false);
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
        onBlur={() => {
          if (selected) setQuery(workspaceLabel(selected));
        }}
        onChange={(event) => {
          const value = event.target.value;
          setQuery(value);
          if (!value) onChange("");
          setActiveIndex(-1);
          setOpen(true);
        }}
        onKeyDown={keyDown}
        placeholder={t("workspaceAutocomplete")}
        autoComplete="off"
      />
      {query && <button type="button" aria-label={t("clearWorkspaceSelection")} onClick={() => { setQuery(""); onChange(""); setOpen(true); }}>×</button>}
      {open && <div id={listId} className="compact-combobox-options" role="listbox">
        {matches.length === 0 && <p>{t("noMatchingWorkspaces")}</p>}
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
