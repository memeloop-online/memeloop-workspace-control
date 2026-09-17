import {
  Badge,
  Button,
  Card,
  CardHeader,
  Input,
  Text,
  Tooltip,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import {
  CodeRegular,
  DismissRegular,
  DocumentTextRegular,
  FingerprintRegular,
  LockClosedRegular,
  SearchRegular,
} from "@fluentui/react-icons";

import { useI18n } from "../i18n";
import type { StoredInjection } from "../types";
import { filterReferenceItems, injectionKindLabel } from "../forms/pickerModel";

interface Props {
  items: StoredInjection[];
  selectedKey: string | null;
  search: string;
  loading?: boolean;
  title: string;
  emptyLabel: string;
  onSearchChange: (value: string) => void;
  onSelect: (item: StoredInjection) => void;
}

const useStyles = makeStyles({
  card: {
    minWidth: 0,
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
  },
  header: {
    flexShrink: 0,
  },
  search: {
    padding: `0 ${tokens.spacingHorizontalM} ${tokens.spacingVerticalS}`,
  },
  results: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
    minHeight: "4rem",
    maxHeight: "min(31rem, 52vh)",
    overflowY: "auto",
    padding: `0 ${tokens.spacingHorizontalS} ${tokens.spacingVerticalS}`,
  },
  row: {
    display: "grid",
    gridTemplateColumns: "auto minmax(0, 1fr) auto",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    width: "100%",
    minWidth: 0,
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalS}`,
    textAlign: "start",
    borderRadius: tokens.borderRadiusMedium,
    border: `1px solid transparent`,
    ":hover": {
      backgroundColor: tokens.colorNeutralBackground1Hover,
    },
    ":focus-visible": {
      outline: `${tokens.strokeWidthThick} solid ${tokens.colorBrandBackground}`,
      outlineOffset: "-2px",
    },
  },
  selected: {
    backgroundColor: tokens.colorBrandBackground2,
    border: `${tokens.strokeWidthThin} solid ${tokens.colorBrandBackground}`,
    ":hover": {
      backgroundColor: tokens.colorBrandBackground2,
    },
  },
  icon: {
    display: "grid",
    placeItems: "center",
    width: "2rem",
    height: "2rem",
    borderRadius: tokens.borderRadiusMedium,
  },
  iconFile: {
    color: tokens.colorBrandForeground1,
    backgroundColor: tokens.colorBrandBackground2,
  },
  iconVariable: {
    color: tokens.colorPaletteGreenForeground1,
    backgroundColor: tokens.colorPaletteGreenBackground1,
  },
  iconKey: {
    color: tokens.colorPalettePurpleForeground1,
    backgroundColor: tokens.colorPalettePurpleBackground1,
  },
  iconSensitive: {
    color: tokens.colorPaletteYellowForeground1,
    backgroundColor: tokens.colorPaletteYellowBackground1,
  },
  details: {
    display: "grid",
    gap: tokens.spacingVerticalXXS,
    minWidth: 0,
  },
  key: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    fontWeight: tokens.fontWeightSemibold,
  },
  target: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
  version: {
    whiteSpace: "nowrap",
    flexShrink: 0,
  },
  empty: {
    display: "grid",
    placeItems: "center",
    minHeight: "4rem",
    padding: tokens.spacingVerticalL,
    color: tokens.colorNeutralForeground2,
    textAlign: "center",
  },
});

export function InjectionList({
  items,
  selectedKey,
  search,
  loading = false,
  title,
  emptyLabel,
  onSearchChange,
  onSelect,
}: Props) {
  const { t } = useI18n();
  const styles = useStyles();
  const filteredItems = filterReferenceItems(items, search, t);

  return (
    <Card className={styles.card} appearance="outline">
      <CardHeader
        className={styles.header}
        header={<Text weight="semibold">{title}</Text>}
        description={
          <Badge appearance="tint" color="informative">
            {filteredItems.length}
          </Badge>
        }
      />
      <div className={styles.search}>
        <Input
          value={search}
          onChange={(event) => onSearchChange(event.currentTarget.value)}
          placeholder={t("searchCredentials")}
          aria-label={t("searchCredentials")}
          contentBefore={<SearchRegular aria-hidden="true" />}
          contentAfter={search ? (
            <Button
              appearance="transparent"
              size="small"
              icon={<DismissRegular aria-hidden="true" />}
              aria-label={t("clearSearch")}
              onClick={() => onSearchChange("")}
            />
          ) : undefined}
        />
      </div>
      <div className={styles.results} aria-busy={loading}>
        {loading && <Text className={styles.empty}>{t("loading")}</Text>}
        {!loading && filteredItems.length === 0 && <Text className={styles.empty}>{emptyLabel}</Text>}
        {!loading && filteredItems.map((item) => {
          const selected = selectedKey === item.key;
          const kindDescription = `${injectionKindLabel(item, t)} · ${item.sensitive || item.kind === "secret_file" ? t("sensitiveValue") : t("visibleConfiguration")}`;
          return (
            <Button
              key={item.key}
              type="button"
              appearance="subtle"
              aria-pressed={selected}
              className={`${styles.row} ${selected ? styles.selected : ""}`}
              onClick={() => onSelect(item)}
            >
              <Tooltip content={kindDescription} relationship="description">
                <span className={`${styles.icon} ${kindIconStyle(item, styles)}`} aria-hidden="true">{kindIcon(item)}</span>
              </Tooltip>
              <span className={styles.details}>
                <span className={styles.key}>{item.key}</span>
                <Tooltip content={item.target} relationship="description">
                  <span className={styles.target}>{item.target}</span>
                </Tooltip>
              </span>
              <Badge className={styles.version} appearance="tint" color={item.locked ? "warning" : "informative"}>
                v{item.version}{item.locked ? ` · ${t("locked")}` : ""}
              </Badge>
            </Button>
          );
        })}
      </div>
    </Card>
  );
}

function kindIcon(item: StoredInjection) {
  if (item.sensitive) return <LockClosedRegular />;
  if (item.kind === "environment_variable") return <CodeRegular />;
  if (item.kind === "ssh_public_key") return <FingerprintRegular />;
  if (item.kind === "secret_file") return <LockClosedRegular />;
  return <DocumentTextRegular />;
}

function kindIconStyle(item: StoredInjection, styles: Record<"iconFile" | "iconVariable" | "iconKey" | "iconSensitive", string>) {
  if (item.sensitive) return styles.iconSensitive;
  if (item.kind === "environment_variable") return styles.iconVariable;
  if (item.kind === "ssh_public_key") return styles.iconKey;
  return styles.iconFile;
}
