---
id: security-model
title: 安全模型
sidebar_label: 安全模型
sidebar_position: 9
---

# 安全模型

本页概述平台设计提供的保证，以及仍由部署方承担的责任。

## 隔离

- **工作负载隔离** —— 每个工作区是独立的 Kubernetes 工作负载，拥有独立
  持久卷。模板可选择沙箱化 `runtime_class_name`（例如 gVisor
  RuntimeClass）以获得更强的内核隔离。
- **网络隔离** —— 模板的 `egress_policy` 选择 `unrestricted` 或
  `internet_only`。平台渲染选中该工作区自身 Pod 标签的 NetworkPolicy，
  即使工作区共享命名空间也成立。命名空间边界本身不是隔离机制。
- **集群访问** —— `cluster_access: false` 的模板不会向工作区下发任何
  Kubernetes API 凭据。

## 供应链

- **默认拒绝镜像** —— 未经系统管理员显式放行的镜像无法创建工作区
  （Image Contract v1）。
- **评审模板** —— 用户只能从模板创建工作区；无法注入任意镜像、资源或
  主机路径。

## 认证与授权

- **RBAC** —— 系统管理员、组织管理员、成员三种角色，权限各不相同。
- **带 scope 的 API 密钥** —— 密钥携带细粒度 scope、强制过期时间
  （≤ 365 天）与可选模板限制。明文密钥只在创建时展示一次；服务端只存
  哈希。
- **短期票据** —— 网页终端与端口映射会话通过一次性票据建立，随后由
  `HttpOnly`、`Secure`、`SameSite=Lax` 会话 cookie 维持。

## 数据保护

- **静态加密** —— 配置 `MWC_ENCRYPTION_KEY` 后，敏感注入值以
  AES-256-GCM 信封加密存储。
- **双向 TLS** —— 从网关/代理进入工作区网页终端的一跳校验客户端证书。
- **密钥卫生** —— 端口映射的通配证书只保存在网关侧；私钥不会复制到
  工作区 Namespace。
- **审计** —— 特权管理操作记入审计日志。

## 部署方责任

- 从 Secret 注入 `MWC_ENCRYPTION_KEY` 与 `MWC_INTERNAL_AUTH_TOKEN`，不要
  写入 shell 历史。
- 对外部租户，将模板的 `egress_policy`、`cluster_access` 与 runtime class
  限制为经过评审的取值。
- 将自动化 API 密钥绑定到其所需的特定模板 ID。
- 将网关 external-auth 与通配证书配置纳入变更管理。
