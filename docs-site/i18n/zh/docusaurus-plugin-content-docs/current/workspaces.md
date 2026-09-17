---
id: workspaces
title: 工作区
sidebar_label: 工作区
sidebar_position: 3
---

# 工作区

工作区是从经过评审的模板创建的、隔离的按用户分配的 Kubernetes 工作负载。
控制面拥有其完整生命周期：供给、凭据注入、状态转换、协调与拆除。

## 生命周期

工作区支持四种动作，可在控制台操作，或通过
`POST /api/v1/workspaces/{workspace_id}/actions/{action}`：

- `start` —— 供给工作区工作负载并注入解析后的凭据。
- `stop` —— 停止工作负载，保留持久数据。
- `restart` —— 先停止再启动。
- `delete` —— 移除工作负载及其临时资源。

工作区状态变化通过 SSE 端点 `GET /api/v1/events` 实时推送给客户端。

## 模板

模板定义一类工作区。`spec` 的关键字段：

| 字段 | 作用 |
| --- | --- |
| `image` | OCI 镜像引用；必须在白名单内（Image Contract v1）。 |
| `access_mode` | 用户访问工作区的方式。 |
| `resources` / `pod_requests` | CPU、内存、GPU 与磁盘配额。 |
| `workspace_user` / `workspace_home` | 容器内用户与 Home 路径。 |
| `buildkit` | 启用工作区内 BuildKit 守护进程用于镜像构建。 |
| `storage_policy.temporary_storage_gib` | 临时存储总容量。 |
| `cluster_access` | 工作区是否获得 Kubernetes API 凭据。 |
| `egress_policy` | 网络出口策略：`unrestricted` 或 `internet_only`。 |
| `runtime_class_name` | 可选的 Kubernetes RuntimeClass（例如沙箱运行时）。 |
| `placement` | `allowed_node_pools` 与 `default_node_pool` 调度约束。 |
| `desktop` | 可选的浏览器桌面端点。 |

模板可以在不删除的情况下启用或停用，并通过
`PUT /api/v1/templates/{template_id}` 更新。

## 存储

MWC 将持久用户数据与可再生数据分离：

- **持久 Home** —— 挂载在模板 `workspace_home` 的持久卷。它在停止/启动后
  保留，容量独立设置。
- **临时存储** —— 配置临时 StorageClass 后，`temporary_storage_gib` 成为
  Pod 持有的通用临时 PVC `workspace-scratch` 的容量请求。所选存储类应提供
  硬容量约束；容器还保留较小的本地 `ephemeral-storage` 请求与限制，避免
  可写层或日志压力悄悄消耗 PVC 容量。

## 调度与节点池

管理员定义节点池（`PUT /api/v1/admin/node-pools/{name}`），映射到
Kubernetes 节点选择器。模板通过 `placement.allowed_node_pools` 限制调度；
用户可在创建时选择允许的节点池，也可事后通过
`PUT /api/v1/workspaces/{workspace_id}/placement` 调整。
`GET /api/v1/node-pools` 列出调用方可见的节点池。

## 更新镜像

工作区镜像可通过 `PUT /api/v1/workspaces/{workspace_id}/image` 原地更新，
同样受 Image Contract v1 白名单约束。变更在下一次启动时生效。

## 运行时观测

- `GET /api/v1/workspaces/{workspace_id}` —— 权威工作区状态。
- `GET /api/v1/workspaces/{workspace_id}/runtime` —— 实时运行时详情。
- `GET /api/v1/workspace-runtimes` —— 可见工作区的运行时列表。
