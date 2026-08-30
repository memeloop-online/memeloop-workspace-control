import { useEffect, useId, useMemo, useRef, useState } from "react";

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

export function CredentialReferencePicker({
  organizationItems,
  userItems,
  organizationSelected,
  userSelected,
  onOrganizationSelected,
  onUserSelected,
}: Props) {
  const { t } = useI18n();
  const popupId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const selectedCount = organizationItems.filter((item) => item.locked || organizationSelected.includes(item.key)).length + userItems.filter((item) => userSelected.includes(item.key)).length;
  const filteredOrganization = useMemo(() => filterReferenceItems(organizationItems, search, t), [organizationItems, search, t]);
  const filteredUser = useMemo(() => filterReferenceItems(userItems, search, t), [userItems, search, t]);

  useEffect(() => {
    if (!open) return;
    searchRef.current?.focus();
    const pointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) close();
    };
    const keyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close(true);
      }
    };
    document.addEventListener("pointerdown", pointerDown);
    document.addEventListener("keydown", keyDown);
    return () => {
      document.removeEventListener("pointerdown", pointerDown);
      document.removeEventListener("keydown", keyDown);
    };
  }, [open]);

  function close(restoreFocus = false) {
    setOpen(false);
    setSearch("");
    if (restoreFocus) requestAnimationFrame(() => triggerRef.current?.focus());
  }

  function toggle(key: string, selected: string[], update: (keys: string[]) => void) {
    update(selected.includes(key) ? selected.filter((item) => item !== key) : [...selected, key]);
  }

  return <div className="credential-reference-picker wide" ref={rootRef}>
    <span className="compact-field-label">{t("organizationAndUserCredentials")}</span>
    <button ref={triggerRef} type="button" className="credential-reference-trigger" aria-haspopup="dialog" aria-expanded={open} aria-controls={popupId} onClick={() => setOpen((value) => !value)}>
      <span>{selectedCount > 0 ? `${t("selectedReferences")} · ${selectedCount}` : t("chooseCredentialReferences")}</span>
      <span aria-hidden="true">{open ? "▴" : "▾"}</span>
    </button>
    {open && <div id={popupId} className="credential-reference-popover" role="dialog" aria-label={t("organizationAndUserCredentials")}>
      <div className="credential-reference-toolbar">
        <input ref={searchRef} type="search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t("searchCredentialReferences")} aria-label={t("searchCredentialReferences")} />
        <button type="button" className="text-button" onClick={() => { onOrganizationSelected([]); onUserSelected([]); }}>{t("clearSelectedReferences")}</button>
      </div>
      <div className="credential-reference-groups">
        <ReferenceGroup title={t("scopeOrganization")} items={filteredOrganization} selected={organizationSelected} onToggle={(key) => toggle(key, organizationSelected, onOrganizationSelected)} emptyLabel={t("noMatchingCredentials")} />
        <ReferenceGroup title={t("scopeUser")} items={filteredUser} selected={userSelected} onToggle={(key) => toggle(key, userSelected, onUserSelected)} emptyLabel={t("noMatchingCredentials")} />
      </div>
      <p>{t("selectedReferenceHelp")}</p>
    </div>}
  </div>;
}

function ReferenceGroup({ title, items, selected, onToggle, emptyLabel }: { title: string; items: StoredInjection[]; selected: string[]; onToggle: (key: string) => void; emptyLabel: string }) {
  const { t } = useI18n();
  return <section className="credential-reference-group" aria-label={title}>
    <strong>{title}</strong>
    <div className="credential-reference-options" role="group" aria-label={title}>
      {items.length === 0 && <small>{emptyLabel}</small>}
      {items.map((item) => <label key={item.key}>
        <input type="checkbox" checked={item.locked || selected.includes(item.key)} disabled={item.locked} onChange={() => onToggle(item.key)} />
        <span><b>{item.key}</b><small>{injectionKindLabel(item, t)} · {item.target}{item.locked ? ` · ${t("locked")}` : ""}</small></span>
      </label>)}
    </div>
  </section>;
}
