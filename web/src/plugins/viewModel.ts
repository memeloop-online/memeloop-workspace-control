import type { PluginConfigurationScope } from "./types";

export type PluginCatalogState = "loading" | "error" | "empty" | "ready";

export function pluginCatalogState(loading: boolean, error: string, count: number): PluginCatalogState {
  if (loading) return "loading";
  if (error) return "error";
  return count === 0 ? "empty" : "ready";
}

export function nextConfigurationScope(
  current: PluginConfigurationScope,
  key: string,
  systemAdmin: boolean,
): PluginConfigurationScope {
  if (!systemAdmin) return "organization";
  if (key !== "ArrowLeft" && key !== "ArrowRight" && key !== "Home" && key !== "End") return current;
  if (key === "Home" || key === "ArrowLeft") return "installation";
  return "organization";
}

export type PluginErrorMessageKey =
  | "pluginVersionConflict"
  | "pluginInvalidConfiguration"
  | "pluginNotFound"
  | "pluginRequestFailed";

export function pluginErrorMessageKey(code: string | undefined): PluginErrorMessageKey {
  if (code === "plugin_configuration_version_conflict") return "pluginVersionConflict";
  if (code === "invalid_plugin_configuration") return "pluginInvalidConfiguration";
  if (code === "plugin_not_found") return "pluginNotFound";
  return "pluginRequestFailed";
}

