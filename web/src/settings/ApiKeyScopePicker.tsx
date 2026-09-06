import type { ApiKeyScope } from "../types";
import type { MessageKey } from "../i18n";

export interface GrantableApiKeyScope {
  scope: ApiKeyScope;
  label: MessageKey;
  description: MessageKey;
  risk?: "high";
}

interface Props {
  scopes: readonly GrantableApiKeyScope[];
  selected: readonly ApiKeyScope[];
  onChange: (scopes: ApiKeyScope[]) => void;
  legend: string;
  translate: (key: MessageKey) => string;
}

/**
 * The native checkbox remains in the tab order. The card label expands its
 * target to a comfortable touch size without hiding the selected state.
 */
export function ApiKeyScopePicker({ scopes, selected, onChange, legend, translate }: Props) {
  function toggle(scope: ApiKeyScope) {
    onChange(selected.includes(scope)
      ? selected.filter((item) => item !== scope)
      : [...selected, scope]);
  }

  return <fieldset className="api-key-scope-picker">
    <legend>{legend}</legend>
    <div className="api-key-scope-grid">
      {scopes.map(({ scope, label, description, risk }) => {
        const checked = selected.includes(scope);
        return <label className="api-key-scope-card" data-risk={risk} data-selected={checked} key={scope}>
          <input
            checked={checked}
            type="checkbox"
            onChange={() => toggle(scope)}
          />
          <span className="api-key-scope-mark" aria-hidden="true">{checked ? "✓" : ""}</span>
          <span className="api-key-scope-copy">
            <strong>{translate(label)}</strong>
            <small>{translate(description)}</small>
          </span>
        </label>;
      })}
    </div>
  </fieldset>;
}
