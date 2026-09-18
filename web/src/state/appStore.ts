import { create } from "zustand";
import type { AppView } from "../design-system/AppShell";

export type AppTheme = "light" | "dark";

const TOKEN_KEY = "mwc.api-token";
const ORGANIZATION_KEY = "mwc.organization-id";
const THEME_KEY = "mwc.theme";
const VIEW_KEY = "mwc.view";

function sessionValue(key: string): string {
  return typeof sessionStorage === "undefined" ? "" : sessionStorage.getItem(key) ?? "";
}

function localValue(key: string): string {
  return typeof localStorage === "undefined" ? "" : localStorage.getItem(key) ?? "";
}

function setSessionValue(key: string, value: string): void {
  if (typeof sessionStorage === "undefined") return;
  if (value) sessionStorage.setItem(key, value);
  else sessionStorage.removeItem(key);
}

function setLocalValue(key: string, value: string): void {
  if (typeof localStorage === "undefined") return;
  if (value) localStorage.setItem(key, value);
  else localStorage.removeItem(key);
}

function initialTheme(): AppTheme {
  const saved = localValue(THEME_KEY);
  if (saved === "light" || saved === "dark") return saved;
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const appViews: readonly AppView[] = ["workspaces", "injections", "plugins", "administration", "audit", "settings"];

function initialView(): AppView {
  const saved = localValue(VIEW_KEY) as AppView;
  return appViews.includes(saved) ? saved : "workspaces";
}

interface AppState {
  token: string;
  organizationId: string;
  theme: AppTheme;
  view: AppView;
  setToken: (token: string) => void;
  setOrganizationId: (organizationId: string) => void;
  setTheme: (theme: AppTheme) => void;
  setView: (view: AppView) => void;
  clearSession: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  token: sessionValue(TOKEN_KEY),
  organizationId: localValue(ORGANIZATION_KEY),
  theme: initialTheme(),
  view: initialView(),
  setToken: (token) => {
    setSessionValue(TOKEN_KEY, token);
    set({ token });
  },
  setOrganizationId: (organizationId) => {
    setLocalValue(ORGANIZATION_KEY, organizationId);
    set({ organizationId });
  },
  setTheme: (theme) => {
    setLocalValue(THEME_KEY, theme);
    set({ theme });
  },
  setView: (view) => {
    setLocalValue(VIEW_KEY, view);
    set({ view });
  },
  clearSession: () => {
    setSessionValue(TOKEN_KEY, "");
    set({ token: "", organizationId: "", view: "workspaces" });
  },
}));
