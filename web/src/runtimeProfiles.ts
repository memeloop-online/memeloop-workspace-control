import type { RuntimeProfile } from "./types";
import type { MessageKey } from "./i18n";

export interface RuntimeProfileOption {
  value: RuntimeProfile;
  labelKey: MessageKey;
  descriptionKey: MessageKey;
  highRisk: boolean;
}

export const RUNTIME_PROFILES: readonly RuntimeProfileOption[] = [
  {
    value: "standard",
    labelKey: "profileStandard",
    descriptionKey: "profileStandardDescription",
    highRisk: false,
  },
  {
    value: "rust_dev",
    labelKey: "profileRust",
    descriptionKey: "profileRustDescription",
    highRisk: false,
  },
  {
    value: "node_dev",
    labelKey: "profileNode",
    descriptionKey: "profileNodeDescription",
    highRisk: false,
  },
  {
    value: "maintainance",
    labelKey: "profileMaintenance",
    descriptionKey: "profileMaintenanceDescription",
    highRisk: true,
  },
] as const;

export function runtimeProfileOption(profile: RuntimeProfile): RuntimeProfileOption {
  return RUNTIME_PROFILES.find((item) => item.value === profile) ?? RUNTIME_PROFILES[0];
}

export function runtimeProfileLabel(profile: RuntimeProfile, t: (key: MessageKey) => string): string {
  return t(runtimeProfileOption(profile).labelKey);
}

export function runtimeProfileDescription(profile: RuntimeProfile, t: (key: MessageKey) => string): string {
  return t(runtimeProfileOption(profile).descriptionKey);
}

export function isHighRiskRuntimeProfile(profile: RuntimeProfile): boolean {
  return runtimeProfileOption(profile).highRisk;
}
