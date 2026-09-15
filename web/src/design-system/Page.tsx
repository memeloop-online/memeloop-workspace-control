import { Body1, Title2, makeStyles, mergeClasses, tokens } from "@fluentui/react-components";
import { useId, type ReactNode } from "react";

const usePageStyles = makeStyles({
  page: {
    display: "grid",
    alignContent: "start",
    gap: tokens.spacingVerticalXXL,
    width: "100%",
    maxWidth: "1200px",
    margin: "0 auto",
    padding: `${tokens.spacingVerticalXL} ${tokens.spacingHorizontalXXL} ${tokens.spacingVerticalXXXL}`,
    boxSizing: "border-box",
    "@media (max-width: 760px)": {
      gap: tokens.spacingVerticalXL,
      padding: `${tokens.spacingVerticalL} ${tokens.spacingHorizontalM} ${tokens.spacingVerticalXXL}`,
    },
  },
  pageWide: {
    maxWidth: "1440px",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalL}`,
    flexWrap: "wrap",
  },
  headerText: {
    display: "grid",
    gap: tokens.spacingVerticalXS,
    minWidth: 0,
  },
  title: {
    margin: 0,
  },
  description: {
    color: tokens.colorNeutralForeground2,
    maxWidth: "70ch",
  },
  headerActions: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    flexWrap: "wrap",
  },
});

interface PageProps {
  title: string;
  description?: string;
  actions?: ReactNode;
  wide?: boolean;
  children: ReactNode;
}

/** Standard page container: consistent padding, width, and header hierarchy. */
export function Page({ title, description, actions, wide = false, children }: PageProps) {
  const styles = usePageStyles();
  const titleId = useId();
  return (
    <section className={mergeClasses(styles.page, wide && styles.pageWide)} aria-labelledby={titleId}>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <Title2 id={titleId} className={styles.title}>{title}</Title2>
          {description ? <Body1 className={styles.description}>{description}</Body1> : null}
        </div>
        {actions ? <div className={styles.headerActions}>{actions}</div> : null}
      </header>
      {children}
    </section>
  );
}
