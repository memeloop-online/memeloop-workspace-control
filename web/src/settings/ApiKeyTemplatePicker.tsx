import {
  Button,
  Checkbox,
  Combobox,
  Field,
  Option,
  Text,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

import type { MessageKey } from "../i18n";
import type { WorkspaceTemplate } from "../types";
import { shortTemplateId } from "./apiKeyTemplatePickerModel";

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

const useStyles = makeStyles({
  root: {
    display: "grid",
    gap: tokens.spacingVerticalM,
    paddingBlockStart: tokens.spacingVerticalL,
    borderTop: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
  },
  options: {
    display: "grid",
    gap: tokens.spacingVerticalS,
  },
  hint: {
    color: tokens.colorNeutralForeground2,
  },
  selection: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalXS,
  },
  picker: {
    display: "grid",
    gap: tokens.spacingVerticalS,
  },
  selectionItem: {
    maxWidth: "100%",
  },
  selectionLabel: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
});

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
  const classes = useStyles();
  const selectedOptions = templates.filter((template) => selected.includes(template.id)).map((template) => template.id);

  return <fieldset className={classes.root} disabled={disabled}>
    <div className={classes.options}>
      <Checkbox
        checked={restricted}
        disabled={restrictionDisabled}
        label={translate("apiKeyRestrictTemplates")}
        onChange={(_, data) => {
          onRestrictedChange(Boolean(data.checked));
          if (!data.checked) onSelectedChange([]);
        }}
      />
      <span className={classes.hint}>{translate("apiKeyTemplateRestriction")}</span>
    </div>
    {restricted && <Field label={translate("templates")}>
      <div className={classes.picker}>
        <Combobox
          multiselect
          selectedOptions={selectedOptions}
          placeholder={selectedOptions.length > 0 ? translate("chooseMoreTemplates") : translate("chooseTemplate")}
          onOptionSelect={(_, data) => onSelectedChange(data.selectedOptions)}
        >
          {templates.map((template) => <Option key={template.id} value={template.id} text={template.name}>
            {template.name} · {shortTemplateId(template.id)}
          </Option>)}
        </Combobox>
        {selectedOptions.length > 0 && <div className={classes.selection} aria-label={translate("selectedTemplates")}>
          {templates.filter((template) => selectedOptions.includes(template.id)).map((template) => (
            <Button
              className={classes.selectionItem}
              key={template.id}
              appearance="secondary"
              size="small"
              type="button"
              aria-label={`${translate("removeTemplateSelection")} ${template.name}`}
              onClick={() => onSelectedChange(selectedOptions.filter((id) => id !== template.id))}
            >
              <Text className={classes.selectionLabel}>{template.name}</Text> ×
            </Button>
          ))}
        </div>}
      </div>
    </Field>}
  </fieldset>;
}
