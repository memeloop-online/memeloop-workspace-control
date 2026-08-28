import type { RuntimeProfile } from "./types";

export interface RuntimeProfileOption {
  value: RuntimeProfile;
  label: string;
  labelEn: string;
  description: string;
  descriptionEn: string;
  highRisk: boolean;
}

export const RUNTIME_PROFILES: readonly RuntimeProfileOption[] = [
  {
    value: "standard",
    label: "标准工作区",
    labelEn: "Standard workspace",
    description: "使用平台的标准运行时约束。",
    descriptionEn: "Uses the platform standard runtime contract.",
    highRisk: false,
  },
  {
    value: "rust_dev",
    label: "Rust 开发",
    labelEn: "Rust development",
    description: "使用现有 Rust 开发镜像与 /home/rust-dev。",
    descriptionEn: "Uses the existing Rust development image and /home/rust-dev.",
    highRisk: false,
  },
  {
    value: "node_dev",
    label: "Node.js 开发",
    labelEn: "Node.js development",
    description: "使用现有 Node.js 开发镜像与 /home/node-dev。",
    descriptionEn: "Uses the existing Node.js development image and /home/node-dev.",
    highRisk: false,
  },
  {
    value: "maintainance",
    label: "Maintainance",
    labelEn: "Maintainance",
    description: "授予集群管理能力，仅限明确授权的运维工作区。",
    descriptionEn: "Grants cluster management capabilities to explicitly authorized maintenance workspaces.",
    highRisk: true,
  },
] as const;

export function runtimeProfileOption(profile: RuntimeProfile): RuntimeProfileOption {
  return RUNTIME_PROFILES.find((item) => item.value === profile) ?? RUNTIME_PROFILES[0];
}

export function runtimeProfileLabel(profile: RuntimeProfile, locale: "zh-CN" | "en" = "zh-CN"): string {
  const option = runtimeProfileOption(profile);
  return locale === "en" ? option.labelEn : option.label;
}

export function runtimeProfileDescription(profile: RuntimeProfile, locale: "zh-CN" | "en" = "zh-CN"): string {
  const option = runtimeProfileOption(profile);
  return locale === "en" ? option.descriptionEn : option.description;
}

export function isHighRiskRuntimeProfile(profile: RuntimeProfile): boolean {
  return runtimeProfileOption(profile).highRisk;
}
