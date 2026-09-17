---
id: api
title: API 参考
sidebar_label: API
sidebar_position: 7
---

# API 参考

REST API 在 `/api/v1` 下版本化。每个部署在 `GET /api/v1/openapi.json`
提供权威的机器可读契约；本页概述集成模型。

## 认证

API 使用者使用个人 API 密钥认证：

```bash
curl "$BASE/api/v1/me" -H "Authorization: Bearer $API_KEY"
```

密钥在控制台管理，或通过 `GET`/`POST /api/v1/me/api-keys` 与
`DELETE /api/v1/me/api-keys/{key_id}` 管理。规则：

- 每个密钥至少携带一个细粒度 scope。
- 过期时间是最多 365 天内的 Unix 秒时间戳。
- 密钥明文只在创建响应中出现一次。
- 密钥可选地限制到特定模板 ID：`null` 表示不额外限制，`[]` 表示该密钥
  不能使用任何模板创建工作区。

可用 scope：`create_workspace`、`read_workspace`、`connect_workspace`、
`change_workspace_state`、`delete_workspace`、`manage_organization`、
`manage_members`、`manage_locked_injections`、`manage_system`、
`manage_api_keys`。

## 分页

列表端点使用游标分页。请求接受 `limit`、`cursor`、`search`；响应返回
`items` 与可选的 `next_cursor`。游标由服务端生成，客户端按原值回传，
不依赖 offset：

- `GET /api/v1/workspaces?organization_id=<id>`
- `GET /api/v1/organizations`
- `GET /api/v1/admin/users`
- `GET /api/v1/organizations/{id}/members`

## 端点地图

| 领域 | 端点 |
| --- | --- |
| 身份 | `GET /api/v1/me`、`GET`/`PUT /api/v1/me/profile` |
| API 密钥 | `GET`/`POST /api/v1/me/api-keys`、`DELETE .../api-keys/{key_id}` |
| 工作区 | `GET`/`POST /api/v1/workspaces`、`GET /api/v1/workspaces/{id}`、`POST .../actions/{action}` |
| 模板 | `GET`/`POST /api/v1/templates`、`PUT`/`DELETE .../{id}`、`PUT .../enabled` |
| 注入 | `GET`/`PUT`/`DELETE /api/v1/injections/{scope}/{scope_id}[/{key}]`、`POST .../batch-delete`、`POST /api/v1/injections/preview` |
| 访问 | `GET .../ssh-client-public-key`、`POST .../web-shell-tickets`、`GET`/`POST`/`DELETE .../port-mappings[/{id}]`、`POST .../open` |
| 组织 | `GET`/`POST /api/v1/organizations`、`PUT`/`DELETE .../{id}`、成员、配额、usage-summary |
| 管理 | `GET`/`POST /api/v1/admin/users`、`GET`/`PUT /api/v1/admin/images`、`GET`/`PUT`/`DELETE /api/v1/admin/node-pools[/{name}]`、`GET /api/v1/audit`、`GET /api/v1/admin/scaling` |
| 插件 | `GET /api/v1/plugins`、检查、安装、配置 |
| Webhook | `GET`/`POST /api/v1/webhooks` |
| 事件 | `GET /api/v1/events`（SSE） |
| 系统 | `GET /api/v1/system/info`、`GET /api/v1/openapi.json`、`GET /livez`、`GET /readyz` |

## 事件与 Webhook

- **Server-sent events** —— `GET /api/v1/events` 推送工作区状态变化与平台
  事件，适合实时仪表盘。
- **Webhook** —— 向外部 HTTPS 端点投递，带签名以便接收方验证来源。

## 错误与幂等

错误返回带机器可读 code 与 message 的 JSON 体。在可能发生重试的变更端点
（创建、动作）上保证幂等，网络失败时客户端可安全重试。
