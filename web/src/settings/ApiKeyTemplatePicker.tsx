import {
  Checkbox,
  Combobox,
  Field,
  Option,
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
      <Combobox
        multiselect
        selectedOptions={selectedOptions}
        placeholder={translate("chooseTemplate")}
        onOptionSelect={(_, data) => onSelectedChange(data.selectedOptions)}
      >
        {templates.map((template) => <Option key={template.id} value={template.id} text={template.name}>
          {template.name} · {shortTemplateId(template.id)}
        </Option>)}
      </Combobox>
    </Field>}
  </fieldset>;
}
