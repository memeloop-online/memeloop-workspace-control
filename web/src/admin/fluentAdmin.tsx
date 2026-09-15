import type { ReactElement, ReactNode } from "react";
import {
  Body1,
  Button,
  Card,
  CardHeader,
  Caption1,
  Text,
  makeStyles,
  shorthands,
  tokens,
} from "@fluentui/react-components";
import type { ButtonProps } from "@fluentui/react-components";

export const useAdminStyles = makeStyles({
  sectionGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 320px), 1fr))",
    gap: tokens.spacingHorizontalL,
    alignItems: "start",
  },
  wide: {
    gridColumn: "1 / -1",
  },
  card: {
    minWidth: 0,
    height: "100%",
    ...shorthands.padding(tokens.spacingVerticalL, tokens.spacingHorizontalL),
  },
  cardBody: {
    display: "grid",
    rowGap: tokens.spacingVerticalM,
    minWidth: 0,
  },
  toolbar: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalS,
  },
  toolbarGroup: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalS,
  },
  formGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 220px), 1fr))",
    gap: tokens.spacingVerticalM,
    alignItems: "start",
  },
  full: {
    gridColumn: "1 / -1",
  },
  actions: {
    display: "flex",
    alignItems: "center",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalS,
  },
  stack: {
    display: "grid",
    rowGap: tokens.spacingVerticalS,
  },
  muted: {
    color: tokens.colorNeutralForeground3,
  },
  code: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    overflowWrap: "anywhere",
  },
  table: {
    overflowX: "auto",
    minWidth: 0,
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },
  empty: {
    color: tokens.colorNeutralForeground3,
    textAlign: "center",
    ...shorthands.padding(tokens.spacingVerticalXXL, tokens.spacingHorizontalL),
  },
  status: {
    display: "inline-flex",
    alignItems: "center",
    ...shorthands.padding(tokens.spacingVerticalXXS, tokens.spacingHorizontalS),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    backgroundColor: tokens.colorNeutralBackground3,
  },
  dangerText: {
    color: tokens.colorPaletteRedForeground1,
  },
  list: {
    display: "grid",
    rowGap: tokens.spacingVerticalXS,
    maxHeight: "min(60vh, 520px)",
    overflowY: "auto",
    ...shorthands.padding(tokens.spacingVerticalXS),
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
  },
  listButton: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: tokens.spacingHorizontalM,
    width: "100%",
    textAlign: "start",
    ...shorthands.padding(tokens.spacingVerticalS, tokens.spacingHorizontalM),
  },
  group: {
    ...shorthands.border("1px", "solid", tokens.colorNeutralStroke2),
    ...shorthands.borderRadius(tokens.borderRadiusMedium),
    ...shorthands.padding(tokens.spacingVerticalM, tokens.spacingHorizontalM),
    ...shorthands.margin("0"),
    display: "grid",
    rowGap: tokens.spacingVerticalS,
    minWidth: 0,
  },
  groupLegend: {
    ...shorthands.padding("0", tokens.spacingHorizontalXS),
  },
  yaml: {
    minHeight: "26rem",
    width: "100%",
    resize: "vertical",
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    lineHeight: tokens.lineHeightBase300,
  },
  dialogBody: {
    display: "grid",
    rowGap: tokens.spacingVerticalM,
    minWidth: "min(72vw, 680px)",
    maxWidth: "min(92vw, 760px)",
    maxHeight: "min(76vh, 720px)",
    overflowY: "auto",
  },
});

export function AdminCard({
  title,
  description,
  action,
  className,
  children,
}: {
  title: ReactNode;
  description?: ReactElement;
  action?: ReactElement;
  className?: string;
  children: ReactNode;
}) {
  const styles = useAdminStyles();
  return <Card appearance="outline" className={`${styles.card}${className ? ` ${className}` : ""}`}>
    <CardHeader
      header={<Text weight="semibold" size={400}>{title}</Text>}
      description={description ? <Caption1 className={styles.muted}>{description}</Caption1> : undefined}
      action={action}
    />
    <div className={styles.cardBody}>{children}</div>
  </Card>;
}

export function AdminToolbar({ children, action }: { children: ReactNode; action?: ReactNode }) {
  const styles = useAdminStyles();
  return <div className={styles.toolbar}><div className={styles.toolbarGroup}>{children}</div>{action}</div>;
}

export function SaveButton({ children, ...props }: ButtonProps) {
  return <Button appearance="primary" {...props}>{children}</Button>;
}

export function StatusText({ children, danger = false }: { children: ReactNode; danger?: boolean }) {
  const styles = useAdminStyles();
  return <Body1 className={danger ? styles.dangerText : styles.status}>{children}</Body1>;
}
