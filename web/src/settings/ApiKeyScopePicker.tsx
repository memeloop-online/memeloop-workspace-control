import {
  Body2,
  Checkbox,
  Text,
  makeStyles,
  tokens,
} from "@fluentui/react-components";

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

const useStyles = makeStyles({
  root: {
    display: "grid",
    gap: tokens.spacingVerticalS,
    margin: 0,
    padding: 0,
    border: 0,
  },
  label: {
    color: tokens.colorNeutralForeground1,
    fontWeight: tokens.fontWeightSemibold,
  },
  grid: {
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: tokens.spacingHorizontalS,
    "@media (max-width: 900px)": {
      gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    },
    "@media (max-width: 640px)": {
      gridTemplateColumns: "1fr",
    },
  },
  option: {
    display: "grid",
    gap: tokens.spacingVerticalXS,
    minWidth: 0,
    padding: tokens.spacingHorizontalM,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground1,
    transitionProperty: "border-color, background-color, box-shadow",
    transitionDuration: tokens.durationNormal,
    ":hover": {
      border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke1Hover}`,
      backgroundColor: tokens.colorNeutralBackground1Hover,
    },
  },
  selected: {
    border: `${tokens.strokeWidthThin} solid ${tokens.colorBrandStroke1}`,
    backgroundColor: tokens.colorBrandBackground2,
    boxShadow: tokens.shadow2,
  },
  risk: {
    border: `${tokens.strokeWidthThin} solid ${tokens.colorPaletteRedBorder2}`,
  },
  description: {
    paddingInlineStart: tokens.spacingHorizontalXXL,
    color: tokens.colorNeutralForeground2,
  },
});

export function ApiKeyScopePicker({ scopes, selected, onChange, legend, translate }: Props) {
  const classes = useStyles();

  function toggle(scope: ApiKeyScope) {
    onChange(selected.includes(scope)
      ? selected.filter((item) => item !== scope)
      : [...selected, scope]);
  }

  return <fieldset className={classes.root} aria-required="true">
    <legend className={classes.label}>{legend}</legend>
    <div className={classes.grid}>
      {scopes.map(({ scope, label, description, risk }) => {
        const checked = selected.includes(scope);
        const id = `api-key-scope-${scope}`;
        return <div className={`${classes.option} ${checked ? classes.selected : ""} ${risk === "high" && checked ? classes.risk : ""}`} key={scope}>
          <Checkbox
            id={id}
            checked={checked}
            label={translate(label)}
            onChange={() => toggle(scope)}
          />
          <Body2 className={classes.description}>{translate(description)}</Body2>
        </div>;
      })}
      {scopes.length === 0 && <Text>{translate("scopeUnknown")}</Text>}
    </div>
  </fieldset>;
}
