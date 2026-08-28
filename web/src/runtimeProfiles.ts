import type { RuntimeProfile } from "./types";

export interface RuntimeProfileOption {
  value: RuntimeProfile;
  label: string;
  description: string;
  highRisk: boolean;
}

export const RUNTIME_PROFILES: readonly RuntimeProfileOption[] = [
  {
    value: "standard",
    label: "标准工作区",
    description: "使用平台的标准运行时约束。",
    highRisk: false,
  },
  {
    value: "rust_dev",
    label: "Rust 开发",
    description: "使用现有 Rust 开发镜像与 /home/rust-dev。",
    highRisk: false,
  },
  {
    value: "node_dev",
    label: "Node.js 开发",
    description: "使用现有 Node.js 开发镜像与 /home/node-dev。",
    highRisk: false,
  },
  {
    value: "maintainance",
    label: "Maintainance",
    description: "授予集群管理能力，仅限明确授权的运维工作区。",
    highRisk: true,
  },
] as const;

export function runtimeProfileOption(profile: RuntimeProfile): RuntimeProfileOption {
  return RUNTIME_PROFILES.find((item) => item.value === profile) ?? RUNTIME_PROFILES[0];
}

export function runtimeProfileLabel(profile: RuntimeProfile): string {
  return runtimeProfileOption(profile).label;
}

export function isHighRiskRuntimeProfile(profile: RuntimeProfile): boolean {
  return runtimeProfileOption(profile).highRisk;
}
