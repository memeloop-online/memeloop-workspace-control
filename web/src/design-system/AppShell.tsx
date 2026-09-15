import {
  Avatar,
  Body1,
  Body1Strong,
  Button,
  Card,
  CardFooter,
  CardHeader,
  CardPreview,
  Dropdown,
  Drawer,
  DrawerBody,
  DrawerHeader,
  DrawerHeaderTitle,
  Field,
  Input,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MessageBar,
  MessageBarBody,
  Option,
  Spinner,
  Subtitle1,
  Toast,
  ToastTitle,
  Toaster,
  Tooltip,
  useId,
  useToastController,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import {
  AppsRegular,
  ArrowExitRegular,
  DismissRegular,
  DarkThemeRegular,
  HistoryRegular,
  HomeRegular,
  KeyRegular,
  PanelLeftRegular,
  PersonRegular,
  PuzzlePieceRegular,
  SettingsRegular,
  WeatherSunnyRegular,
} from "@fluentui/react-icons";
import { useEffect, useRef, useState, type FormEvent, type ReactElement, type ReactNode } from "react";
import type { Locale, MessageKey } from "../i18n";
import { BrandIcon } from "../BrandIcon";
import { UserAvatar } from "../UserAvatar";
import type { Organization, Principal } from "../types";

export type AppView = "workspaces" | "injections" | "plugins" | "administration" | "audit" | "settings";

const useStyles = makeStyles({
  app: {
    display: "grid",
    gridTemplateColumns: "minmax(232px, 272px) minmax(0, 1fr)",
    minHeight: "100vh",
    backgroundColor: tokens.colorNeutralBackground2,
    color: tokens.colorNeutralForeground1,
  },
  sidebar: {
    position: "sticky",
    top: 0,
    alignSelf: "start",
    height: "100vh",
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXL,
    padding: tokens.spacingVerticalXXL + " " + tokens.spacingHorizontalL,
    borderRight: tokens.strokeWidthThin + " solid " + tokens.colorNeutralStroke2,
    backgroundColor: tokens.colorNeutralBackground1,
  },
  brand: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    padding: "0 " + tokens.spacingHorizontalS,
  },
  brandCopy: {
    minWidth: 0,
    display: "grid",
    gap: tokens.spacingVerticalXXS,
  },
  brandName: { fontSize: tokens.fontSizeBase400, lineHeight: tokens.lineHeightBase400 },
  brandCaption: { color: tokens.colorNeutralForeground3, fontSize: tokens.fontSizeBase200 },
  nav: { display: "grid", gap: tokens.spacingVerticalXS },
  navButton: { justifyContent: "flex-start", width: "100%", minHeight: "42px" },
  sidebarFooter: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    marginTop: "auto",
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
  statusDot: {
    width: "8px",
    height: "8px",
    flex: "0 0 auto",
    borderRadius: tokens.borderRadiusCircular,
    backgroundColor: tokens.colorPaletteGreenBackground3,
  },
  content: { minWidth: 0 },
  topbar: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: tokens.spacingHorizontalL,
    minHeight: "76px",
    padding: tokens.spacingVerticalM + " " + tokens.spacingHorizontalXXL,
    borderBottom: tokens.strokeWidthThin + " solid " + tokens.colorNeutralStroke2,
    backgroundColor: tokens.colorNeutralBackground1,
  },
  org: { minWidth: 0, display: "grid", gap: tokens.spacingVerticalXXS },
  orgLabel: { color: tokens.colorNeutralForeground3, fontSize: tokens.fontSizeBase200 },
  orgName: { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  topbarActions: { display: "flex", alignItems: "center", gap: tokens.spacingHorizontalS },
  userTrigger: { maxWidth: "240px", justifyContent: "flex-start", textAlign: "left" },
  userTriggerContent: { minWidth: 0, display: "grid", gap: tokens.spacingVerticalXXS },
  userName: { overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  userRole: { color: tokens.colorNeutralForeground3, fontSize: tokens.fontSizeBase200 },
  main: { minWidth: 0 },
  loginPage: {
    minHeight: "100vh",
    display: "grid",
    placeItems: "center",
    padding: tokens.spacingHorizontalL,
    backgroundColor: tokens.colorNeutralBackground2,
  },
  loginCard: { width: "min(440px, 100%)" },
  loginError: { width: "100%" },
  loginHeader: { display: "grid", gap: tokens.spacingVerticalM },
  loginBrand: { display: "flex", alignItems: "center", gap: tokens.spacingHorizontalM },
  loginControls: { display: "flex", justifyContent: "flex-end", gap: tokens.spacingHorizontalS },
  loginForm: { display: "grid", gap: tokens.spacingVerticalL },
  loginFooter: { display: "flex", justifyContent: "flex-end", gap: tokens.spacingHorizontalS },
  loading: {
    minHeight: "min(56vh, 480px)",
    display: "grid",
    placeItems: "center",
    gap: tokens.spacingVerticalM,
    padding: tokens.spacingHorizontalL,
    color: tokens.colorNeutralForeground2,
  },
  loadingCard: { display: "grid", justifyItems: "center", gap: tokens.spacingVerticalM, padding: tokens.spacingHorizontalXXL },
  emptyPage: {
    minHeight: "calc(100vh - 76px)",
    display: "grid",
    placeContent: "center",
    justifyItems: "center",
    gap: tokens.spacingVerticalM,
    padding: tokens.spacingHorizontalL,
    textAlign: "center",
  },
  emptyText: { maxWidth: "440px", color: tokens.colorNeutralForeground2 },
  drawerNav: { display: "grid", gap: tokens.spacingVerticalXS },
  mobileMenu: { display: "none" },
  desktopOnly: { display: "inline-flex" },
  "@media (max-width: 900px)": {
    app: { display: "block" },
    sidebar: { display: "none" },
    topbar: { paddingInline: tokens.spacingHorizontalL },
    mobileMenu: { display: "inline-flex" },
    desktopOnly: { display: "none" },
  },
  "@media (max-width: 560px)": {
    topbar: { alignItems: "flex-start", paddingBlock: tokens.spacingVerticalM },
    org: { maxWidth: "calc(100% - 52px)" },
    topbarActions: { gap: tokens.spacingHorizontalXS },
    userTrigger: { maxWidth: "48px", paddingInline: tokens.spacingHorizontalXS },
    userTriggerContent: { display: "none" },
    loginCard: { boxShadow: "none" },
    loginPage: { padding: tokens.spacingHorizontalS },
  },
});

const viewIcons: Record<AppView, ReactElement> = {
  workspaces: <HomeRegular />,
  injections: <KeyRegular />,
  plugins: <PuzzlePieceRegular />,
  administration: <AppsRegular />,
  audit: <HistoryRegular />,
  settings: <SettingsRegular />,
};

type Translation = (key: MessageKey) => string;

interface LanguagePickerProps {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: Translation;
  compact?: boolean;
}

function LanguagePicker({ locale, setLocale, t, compact = false }: LanguagePickerProps) {
  return (
    <Dropdown
      aria-label={t("language")}
      value={locale === "zh-CN" ? t("languageChinese") : locale === "ru" ? t("languageRussian") : t("languageEnglish")}
      selectedOptions={[locale]}
      onOptionSelect={(_, data) => { if (data.optionValue) setLocale(data.optionValue as Locale); }}
      size="small"
      appearance={compact ? "underline" : "outline"}
    >
      <Option value="zh-CN">{t("languageChinese")}</Option>
      <Option value="en">{t("languageEnglish")}</Option>
      <Option value="ru">{t("languageRussian")}</Option>
    </Dropdown>
  );
}

interface LoginScreenProps {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  themeMode: "light" | "dark";
  onToggleTheme: () => void;
  tokenDraft: string;
  setTokenDraft: (value: string) => void;
  onSubmit: (event: FormEvent) => void;
  loading: boolean;
  fatal: string;
  t: Translation;
}

export function LoginScreen(props: LoginScreenProps) {
  const classes = useStyles();
  return (
    <main className={classes.loginPage}>
      <Card className={classes.loginCard} size="large">
        <CardHeader
          header={
            <div className={classes.loginHeader}>
              <div className={classes.loginBrand}><BrandIcon size={48} /><div className={classes.brandCopy}><Body1Strong>Memeloop</Body1Strong><span className={classes.brandCaption}>Workspace Control</span></div></div>
              <Subtitle1>{props.t("loginTitle")}</Subtitle1>
            </div>
          }
          action={<div className={classes.loginControls}><LanguagePicker locale={props.locale} setLocale={props.setLocale} t={props.t} compact /><Tooltip content={props.themeMode === "dark" ? props.t("themeLight") : props.t("themeDark")} relationship="label"><Button aria-label={props.themeMode === "dark" ? props.t("themeLight") : props.t("themeDark")} appearance="subtle" icon={props.themeMode === "dark" ? <WeatherSunnyRegular /> : <DarkThemeRegular />} onClick={props.onToggleTheme} /></Tooltip></div>}
        />
        <form className={classes.loginForm} onSubmit={props.onSubmit}>
          <Field label={props.t("token")} required hint={props.t("tokenPlaceholder")}>
            <Input autoFocus type="password" minLength={32} required value={props.tokenDraft} onChange={(_, data) => props.setTokenDraft(data.value)} />
          </Field>
          <Button type="submit" appearance="primary" size="large" disabled={props.loading}>{props.loading ? props.t("signingIn") : props.t("signIn")}</Button>
        </form>
        {props.fatal && <CardFooter><MessageBar className={classes.loginError} intent="error"><MessageBarBody>{props.fatal}</MessageBarBody></MessageBar></CardFooter>}
      </Card>
    </main>
  );
}

interface AppShellProps {
  children: ReactNode;
  view: AppView;
  onViewChange: (view: AppView) => void;
  locale: Locale;
  setLocale: (locale: Locale) => void;
  themeMode: "light" | "dark";
  onToggleTheme: () => void;
  principal: Principal;
  currentOrganization?: Organization;
  organizationRole?: string;
  canOpenAdministration: boolean;
  canManageGlobalState: boolean;
  canManageOrganizationState: boolean;
  onLogout: () => void;
  notice: string;
  t: Translation;
}

export function AppShell(props: AppShellProps) {
  const classes = useStyles();
  const toasterId = useId("mwc-toaster");
  const { dispatchToast } = useToastController(toasterId);
  const [mobileOpen, setMobileOpen] = useState(false);
  const lastNotice = useRef("");

  useEffect(() => {
    if (props.notice && props.notice !== lastNotice.current) {
      dispatchToast(<Toast><ToastTitle>{props.notice}</ToastTitle></Toast>, { intent: "info", timeout: 5000 });
      lastNotice.current = props.notice;
    } else if (!props.notice) {
      lastNotice.current = "";
    }
  }, [dispatchToast, props.notice]);

  const canShowPlugins = props.canManageGlobalState || props.canManageOrganizationState;
  const nav = (
    <nav className={classes.nav} aria-label={props.t("primaryNavigation")}>
      <NavItem view="workspaces" current={props.view} onSelect={props.onViewChange} t={props.t} />
      <NavItem view="injections" current={props.view} onSelect={props.onViewChange} t={props.t} />
      {canShowPlugins && <NavItem view="plugins" current={props.view} onSelect={props.onViewChange} t={props.t} />}
      {props.canOpenAdministration && <NavItem view="administration" current={props.view} onSelect={props.onViewChange} t={props.t} />}
      {canShowPlugins && <NavItem view="audit" current={props.view} onSelect={props.onViewChange} t={props.t} />}
      <NavItem view="settings" current={props.view} onSelect={props.onViewChange} t={props.t} />
    </nav>
  );

  return (
    <div className={classes.app}>
      <aside className={classes.sidebar} aria-label="Memeloop Workspace Control">
        <Brand classes={classes} />
        {nav}
        <div className={classes.sidebarFooter}><span className={classes.statusDot} aria-hidden="true" />{props.t("apiOnline")}</div>
      </aside>
      <div className={classes.content}>
        <header className={classes.topbar}>
          <Tooltip content={props.t("menu")} relationship="label"><Button className={classes.mobileMenu} appearance="subtle" icon={<PanelLeftRegular />} aria-label={props.t("menu")} onClick={() => setMobileOpen(true)} /></Tooltip>
          <div className={classes.org}><span className={classes.orgLabel}>{props.t("currentOrganization")}</span><Body1Strong className={classes.orgName}>{props.currentOrganization?.name ?? props.t("notEnabled")}</Body1Strong></div>
          <div className={classes.topbarActions}>
            <div className={classes.desktopOnly}><LanguagePicker locale={props.locale} setLocale={props.setLocale} t={props.t} compact /></div>
            <Tooltip content={props.themeMode === "dark" ? props.t("themeLight") : props.t("themeDark")} relationship="label"><Button appearance="subtle" icon={props.themeMode === "dark" ? <WeatherSunnyRegular /> : <DarkThemeRegular />} aria-label={props.themeMode === "dark" ? props.t("themeLight") : props.t("themeDark")} onClick={props.onToggleTheme} /></Tooltip>
            <UserMenu principal={props.principal} organizationRole={props.organizationRole} onSettings={() => props.onViewChange("settings")} onLogout={props.onLogout} t={props.t} classes={classes} />
          </div>
        </header>
        <main className={classes.main}>{props.children}</main>
      </div>
      <Drawer type="overlay" separator open={mobileOpen} onOpenChange={(_, data) => setMobileOpen(data.open)} position="start">
        <DrawerHeader><DrawerHeaderTitle action={<Button appearance="subtle" icon={<DismissRegular />} aria-label={props.t("close")} onClick={() => setMobileOpen(false)} />}>Memeloop</DrawerHeaderTitle></DrawerHeader>
        <DrawerBody><div className={classes.drawerNav}>{nav}</div></DrawerBody>
      </Drawer>
      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

function Brand({ classes }: { classes: ReturnType<typeof useStyles> }) {
  return <div className={classes.brand}><BrandIcon /><div className={classes.brandCopy}><Body1Strong className={classes.brandName}>Memeloop</Body1Strong><span className={classes.brandCaption}>Workspace Control</span></div></div>;
}

function NavItem({ view, current, onSelect, t }: { view: AppView; current: AppView; onSelect: (view: AppView) => void; t: Translation }) {
  const labels: Record<AppView, MessageKey> = { workspaces: "workspaces", injections: "credentials", plugins: "pluginsTitle", administration: "administration", audit: "audit", settings: "settings" };
  return <Button className={useStyles().navButton} appearance={view === current ? "secondary" : "subtle"} icon={viewIcons[view]} aria-current={view === current ? "page" : undefined} onClick={() => onSelect(view)}>{t(labels[view])}</Button>;
}

function UserMenu({ principal, organizationRole, onSettings, onLogout, t, classes }: { principal: Principal; organizationRole?: string; onSettings: () => void; onLogout: () => void; t: Translation; classes: ReturnType<typeof useStyles> }) {
  return <Menu><MenuTrigger disableButtonEnhancement><Button className={classes.userTrigger} appearance="subtle" icon={<Avatar name={principal.display_name} image={{ src: principal.avatar_url ?? undefined }} />} iconPosition="before"><span className={classes.userTriggerContent}><Body1Strong className={classes.userName}>{principal.display_name}</Body1Strong><span className={classes.userRole}>{principal.system_admin ? t("systemAdmin") : organizationRole === "organization_admin" ? t("organizationAdmin") : t("organizationMember")}</span></span></Button></MenuTrigger><MenuPopover><MenuList><MenuItem icon={<PersonRegular />} onClick={onSettings}>{t("settings")}</MenuItem><MenuItem icon={<ArrowExitRegular />} onClick={onLogout}>{t("logout")}</MenuItem></MenuList></MenuPopover></Menu>;
}

export function LoadingView({ label }: { label: string }) {
  const classes = useStyles();
  return <div className={classes.loading} role="status" aria-label={label}><Card className={classes.loadingCard}><Spinner size="medium" /><Body1>{label}</Body1></Card></div>;
}

export function EmptyOrganization({ systemAdmin, t }: { systemAdmin: boolean; t: Translation }) {
  const classes = useStyles();
  return <div className={classes.emptyPage}><Card><CardPreview><BrandIcon size={54} /></CardPreview><CardHeader header={<Subtitle1>{t("noOrganization")}</Subtitle1>} description={<span className={classes.emptyText}>{systemAdmin ? t("noOrganizationAdmin") : t("noOrganizationMember")}</span>} /><CardFooter><Button as="a" href="/api/v1/openapi.json" target="_blank" rel="noreferrer" appearance="primary">{t("viewOpenApi")}</Button></CardFooter></Card></div>;
}
