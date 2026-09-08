import { useEffect, useId, useMemo, useRef, useState } from "react";

import type { MessageKey } from "../i18n";
import type { WorkspaceTemplate } from "../types";
import { shortTemplateId, toggleTemplateSelection } from "./apiKeyTemplatePickerModel";

interface Props {
  templates: readonly WorkspaceTemplate[];
  selected: readonly string[];
  restricted: boolean;
  disabled?: boolean;
  restrictionDisabled?: boolean;
  translate: (key: MessageKey) => string;
  onRestrictedChange: (restricted: boolean) => void;
  onSelectedChange: (selected: string[]) => void;
}

/**
 * Template policy picker for an API key. The trigger/popover follows the
 * existing credential reference picker so search and multi-select controls
 * behave consistently throughout the workspace administration UI.
 */
export function ApiKeyTemplatePicker({
  templates,
  selected,
  restricted,
  disabled = false,
  restrictionDisabled = false,
  translate,
  onRestrictedChange,
  onSelectedChange,
}: Props) {
  const popupId = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const normalizedSearch = search.trim().toLocaleLowerCase();
  const filteredTemplates = useMemo(
    () => normalizedSearch
      ? templates.filter((template) => [template.name, template.id, shortTemplateId(template.id)]
        .some((value) => value.toLocaleLowerCase().includes(normalizedSearch)))
      : templates,
    [normalizedSearch, templates],
  );
  const selectedCount = templates.filter((template) => selected.includes(template.id)).length;

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

  // A create request may finish while the picker is open. Closing it prevents
  // stale controls from remaining interactive during the next form state.
  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  function close(restoreFocus = false) {
    setOpen(false);
    setSearch("");
    if (restoreFocus) requestAnimationFrame(() => triggerRef.current?.focus());
  }

  function toggle(id: string) {
    onSelectedChange(toggleTemplateSelection(selected, id));
  }

  function changeRestriction(nextRestricted: boolean) {
    onRestrictedChange(nextRestricted);
    if (!nextRestricted) onSelectedChange([]);
  }

  return <fieldset className="api-key-template-picker" data-disabled={disabled || undefined}>
    <legend>{translate("apiKeyTemplateRestriction")}</legend>
    <label className="api-key-template-toggle">
      <input
        type="checkbox"
        checked={restricted}
        disabled={disabled || restrictionDisabled}
        onChange={(event) => changeRestriction(event.target.checked)}
      />
      <span>{translate("apiKeyRestrictTemplates")}</span>
    </label>
    {restricted && <div className="credential-reference-picker api-key-template-control" ref={rootRef}>
      <span className="compact-field-label">{translate("templates")}</span>
      <button
        ref={triggerRef}
        type="button"
        className="credential-reference-trigger"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={popupId}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        <span>{selectedCount > 0
          ? `${translate("selectedReferences")} · ${selectedCount}`
          : translate("chooseTemplate")}</span>
        <span aria-hidden="true">{open ? "▴" : "▾"}</span>
      </button>
      {open && <div id={popupId} className="credential-reference-popover api-key-template-popover" role="dialog" aria-label={translate("apiKeyTemplateRestriction")}>
        <div className="credential-reference-toolbar">
          <input
            ref={searchRef}
            type="search"
            value={search}
            disabled={disabled}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={translate("templates")}
            aria-label={translate("templates")}
          />
          <button type="button" className="text-button" disabled={disabled} onClick={() => onSelectedChange([])}>
            {translate("clearSelectedReferences")}
          </button>
        </div>
        <div className="credential-reference-options api-key-template-options" role="group" aria-label={translate("templates")}>
          {filteredTemplates.length === 0 && <small>{translate("noTemplates")}</small>}
          {filteredTemplates.map((template) => <label key={template.id}>
            <input
              type="checkbox"
              checked={selected.includes(template.id)}
              disabled={disabled}
              onChange={() => toggle(template.id)}
            />
            <span>
              <b>{template.name}</b>
              <small>{shortTemplateId(template.id)}</small>
            </span>
          </label>)}
        </div>
      </div>}
    </div>}
  </fieldset>;
}
