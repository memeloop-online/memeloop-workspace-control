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
    value: "coder_rust_dev",
    label: "Coder Rust 开发",
    description: "兼容迁移自 Coder 的 Rust 开发环境。",
    highRisk: false,
  },
  {
    value: "coder_node_dev",
    label: "Coder Node.js 开发",
    description: "兼容迁移自 Coder 的 Node.js 开发环境。",
    highRisk: false,
  },
  {
    value: "coder_token_center_rust_dev",
    label: "Coder Token Center Rust 开发",
    description: "兼容 Token Center 的 Rust 开发环境。",
    highRisk: false,
  },
  {
    value: "coder_cluster_admin",
    label: "Coder 集群管理员",
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
