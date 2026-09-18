import { makeStyles, tokens } from "@fluentui/react-components";

/** Shared Fluent 2 layout tokens for the stateless shared components. */
export const useSharedStyles = makeStyles({
  card: {
    display: "grid",
    gap: tokens.spacingVerticalL,
    minWidth: 0,
    padding: tokens.spacingHorizontalL,
    borderRadius: tokens.borderRadiusLarge,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorNeutralStroke2}`,
    backgroundColor: tokens.colorNeutralBackground1,
    boxShadow: tokens.shadow4,
    transitionProperty: "box-shadow, border-color",
    transitionDuration: tokens.durationNormal,
    "@media (max-width: 760px)": { padding: tokens.spacingHorizontalM },
  },
  cardHeader: { display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: tokens.spacingHorizontalM, minWidth: 0 },
  cardTitle: { display: "grid", gap: tokens.spacingVerticalXXS, minWidth: 0 },
  titleText: { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  idText: { color: tokens.colorNeutralForeground3, fontFamily: tokens.fontFamilyMonospace, fontSize: tokens.fontSizeBase200 },
  metadata: { display: "flex", alignItems: "center", flexWrap: "wrap", gap: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalM}`, color: tokens.colorNeutralForeground2 },
  resourceGrid: { display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 160px), 1fr))", gap: tokens.spacingHorizontalS },
  meter: { display: "grid", gap: tokens.spacingVerticalXS, minWidth: 0, padding: tokens.spacingHorizontalM, borderRadius: tokens.borderRadiusMedium, backgroundColor: tokens.colorNeutralBackground2 },
  meterHeader: { display: "flex", justifyContent: "space-between", alignItems: "baseline", gap: tokens.spacingHorizontalXS },
  meterValue: { fontFamily: tokens.fontFamilyMonospace, fontWeight: tokens.fontWeightSemibold, whiteSpace: "nowrap" },
  meterHint: { color: tokens.colorNeutralForeground3 },
});
