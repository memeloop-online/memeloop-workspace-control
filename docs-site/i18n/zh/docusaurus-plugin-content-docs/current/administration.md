---
id: administration
title: 管理
sidebar_label: 管理
sidebar_position: 6
---

# 管理

本页覆盖系统级与组织级的管理面。所有操作都可在控制台和 REST API 中完成。

## 角色

MWC 有三种角色：

- **系统管理员** —— 完全控制：镜像、用户、组织、节点池、审计与平台设置。
- **组织管理员** —— 管理本组织的成员、配额、锁定注入项与工作区。
- **成员** —— 创建并操作自己的工作区与个人凭据。

## 镜像白名单（Image Contract v1）

工作区镜像默认拒绝。系统管理员必须先显式放行镜像，模板才能引用它：

- `GET /api/v1/admin/images` —— 列出已放行镜像。
- `PUT /api/v1/admin/images` —— 放行或更新镜像条目。

可放行平台标准镜像或经过评审的第三方 OCI 镜像。从白名单移除镜像不会停止
运行中的工作区，但会阻止新工作区引用它。

## 模板

模板是用户创建工作区的单元（见[工作区](./workspaces.md)）。管理员可创建、
更新、启用、停用与删除模板：

- `GET /api/v1/templates` / `POST /api/v1/templates`
- `PUT /api/v1/templates/{template_id}` / `DELETE /api/v1/templates/{template_id}`
- `PUT /api/v1/templates/{template_id}/enabled`

## 用户与组织

- 用户：`GET /api/v1/admin/users`、`POST /api/v1/admin/users`、
  `PUT /api/v1/admin/users/{user_id}`。管理员还可列出并吊销用户的 API 密钥。
- 组织：`GET /api/v1/organizations`、`POST /api/v1/organizations`、
  `PUT`/`DELETE /api/v1/organizations/{organization_id}`。
- 成员：`GET /api/v1/organizations/{organization_id}/members`、
  `PUT`/`DELETE .../members/{user_id}`。
- 用量：`GET /api/v1/organizations/{organization_id}/usage-summary`。

列表端点使用游标分页（`limit`、`cursor`、`search`），返回 `items` 与可选的
`next_cursor`。

## 配额

配额在两个层级限制资源消耗：

- 组织：`GET`/`PUT /api/v1/organizations/{organization_id}/quota`。
- 用户：`GET`/`PUT /api/v1/admin/users/{user_id}/quota`。

## 节点池与扩缩容

- `GET`/`PUT`/`DELETE /api/v1/admin/node-pools[/{name}]` —— 管理模板与调度
  约束引用的节点池。
- `GET /api/v1/admin/scaling` —— 容量与扩缩容概览。

## 审计与可观测性

- `GET /api/v1/audit` —— 特权操作的管理审计日志。
- `GET /api/v1/events` —— 工作区与平台事件的 SSE 流。
- `GET /livez` / `GET /readyz` —— 两个监听器上的探针端点。
- `GET /metrics` —— 仅内部监听器提供的 Prometheus 指标。

## Webhook

签名出站 Webhook 将平台事件通知外部系统：

- `GET /api/v1/webhooks` / `POST /api/v1/webhooks` —— 列出与创建订阅。

投递带有签名，接收方可验证来源真实性。
