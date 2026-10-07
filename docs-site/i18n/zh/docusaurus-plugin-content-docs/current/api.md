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

本人可在 **设置 → 个人 → API 密钥** 中管理密钥，无需先选择组织。获授权的管理员
可在 **设置 → 组织 → 用户与角色 → 凭据配置** 中管理其他用户的密钥。
对应的本人端点包括 `GET`/`POST /api/v1/me/api-keys`、
`GET /api/v1/me/api-keys/{key_id}/token` 与
`DELETE /api/v1/me/api-keys/{key_id}`。规则：

- 每个密钥至少携带一个细粒度 scope。
- 过期时间是最多 365 天内的 Unix 秒时间戳。
- 本人和获授权管理员可以再次复制已保留明文的密钥；个人密钥列表仍只返回摘要。
  管理其他用户的密钥需要相应管理权限，不能通过本人端点跨用户读取。
- 密钥可选地限制到特定模板 ID：`null` 表示不额外限制，`[]` 表示该密钥
  不能使用任何模板创建工作区。

`GET /api/v1/me/api-keys/{key_id}/token` 仅在明确请求时取回调用者自己的、
仍有效且未吊销的密钥。调用此端点使用的认证密钥必须具有 `manage_api_keys`
scope；这不是正常使用仅有 `read_workspace` scope 密钥的前提。成功响应为
`{"token":"..."}`，并带 `Cache-Control: no-store`；客户端不应把明文存入浏览器
持久存储。不存在、不属于本人或当前安装、已吊销及已过期的密钥返回 `404`；
历史仅存哈希、未保留原始明文的密钥返回 `409 api_key_token_unavailable`，无法
反推，取回操作也不会自动轮换或替换它。认证失败返回 `401`，scope 不足返回
`403`。取回操作的记录元数据只包含密钥 ID，不包含明文。

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
| API 密钥 | `GET`/`POST /api/v1/me/api-keys`、`GET .../api-keys/{key_id}/token`、`DELETE .../api-keys/{key_id}` |
| 工作区 | `GET`/`POST /api/v1/workspaces`、`GET /api/v1/workspaces/{id}`、`POST .../actions/{action}` |
| 模板 | `GET`/`POST /api/v1/templates`、`PUT`/`DELETE .../{id}`、`PUT .../enabled` |
| 注入 | `GET`/`PUT`/`DELETE /api/v1/injections/{scope}/{scope_id}[/{key}]`、`POST .../batch-delete`、`POST /api/v1/injections/preview` |
| 访问 | `POST .../web-shell-tickets`、`GET`/`POST`/`DELETE .../port-mappings[/{id}]`、`POST .../open` |
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
