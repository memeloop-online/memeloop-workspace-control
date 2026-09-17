---
id: plugin-development
title: 插件开发
sidebar_label: 插件开发
sidebar_position: 8
---

# 插件开发

MWC 插件是 WebAssembly 组件，可为控制面扩展创建策略、API 中间件、自定义
API 路由与控制台 UI 界面。插件在沙箱中运行；所有与宿主的交互都经过版本化
的 WIT 接口。

## 接口

契约为 `wit/workspace-control.wit`（包
`memeloop:workspace-control@0.2.0`）。插件导出 `plugin-backend` 接口：

| 导出函数 | 调用时机 | 返回 |
| --- | --- | --- |
| `admit-create(context, plan)` | 工作区即将创建时 | `decision` —— 允许或带预声明代码的拒绝 |
| `check-request(context)` | API 请求命中插件中间件时 | `decision` |
| `handle-api(request)` | 请求到达插件自有 API 路由时 | `api-response` —— 状态码、内容类型、响应体 |

拒绝代码必须在 `plugin.json` 中预声明；客户侧自由文本不会穿越 ABI。
声明 `wit_version` 在 `>=0.1.0, <0.3.0` 范围内的插件受支持。

## 包结构

插件包是一个目录（以归档形式分发），包含：

```
my-plugin/
  plugin.json     # 清单，必需
  plugin.wasm     # 编译后的组件，纯 UI 包可省略
  assets/         # 可选的 UI 静态资源
```

清单字段（`plugin.json`）：

| 字段 | 作用 |
| --- | --- |
| `id`、`name`、`version`、`description` | 标识与展示元数据。 |
| `wit_version` | 插件针对的接口版本范围。 |
| `wasm` | 组件文件名（如有）。 |
| `workspace_create_policy` | 为工作区创建启用 `admit-create`。 |
| `denial_codes` | 插件可返回的预声明代码。 |
| `configuration` | 管理员可编辑配置的 JSON Schema 与默认值。 |
| `assets` | 提供给控制台的静态资源。 |
| `ui_surfaces` | 插件渲染的控制台位置。 |
| `api_routes` | 在 `/api/v1/plugin-api/{plugin_id}/{route_id}/` 下提供的路由。 |
| `api_middleware` | 触发 `check-request` 的请求匹配规则。 |

包限制：清单 ≤ 1 MiB，组件 ≤ 64 MiB，归档 ≤ 80 MiB 且 ≤ 64 个文件。

## 安装与生命周期

插件由系统管理员通过两阶段流程安装：

1. **检查** —— 上传归档或指向 URL/GitHub Release；控制面校验包并返回将
   发生的变更：
   - `POST /api/v1/plugins/inspections/upload`
   - `POST /api/v1/plugins/inspections/url`
   - `POST /api/v1/plugins/inspections/github-release`
2. **确认** —— 携带检查令牌调用 `POST /api/v1/plugins/installs` 完成安装。

之后：`PUT /api/v1/plugins/{plugin_id}/enabled` 启停插件，
`DELETE /api/v1/plugins/{plugin_id}` 卸载，
`GET`/`PUT`/`DELETE /api/v1/plugins/{plugin_id}/configuration` 管理其配置
（按声明的 schema 校验）。

## UI 界面与 API 路由

- 控制台界面按会话创建：
  `POST /api/v1/plugins/{plugin_id}/ui-surfaces/{surface_id}/sessions`；资源
  在 `/api/v1/plugin-ui/{plugin_id}/{session_id}/` 下提供，界面通过会话桥
  （`POST .../bridge`）与后端通信。
- 插件 API 路由在 `/api/v1/plugin-api/{plugin_id}/{route_id}/{*path}` 下
  调用，请求体上限 256 KiB。

## 版本化建议

- 将 `wit_version` 固定在你测试过的接口上。
- 把 `denial_codes` 当作公开 API 对待；用户会看到它们。
- 对 `configuration-json` 做防御性校验 —— 管理员可以编辑它。
