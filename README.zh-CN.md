# Memeloop Workspace Control

**[English](README.md)** · **[文档站点](https://memeloop-online.github.io/memeloop-workspace-control/)**

Memeloop Workspace Control（MWC）是一个开源的 Kubernetes 工作区控制面，
用于交付隔离的按用户开发工作区。它通过经过评审的模板和镜像白名单创建
工作区，在启动时注入加密的凭据与文件，并通过 SSH、网页终端和带鉴权的
端口映射提供访问。

![工作区资源总览](docs-site/static/img/screenshots/workspaces-desktop.png)

## 功能

- **工作区生命周期** —— 基于模板的创建、启动、停止、重启和删除，支持
  持久 Home 卷、受限临时存储与节点池调度。
- **凭据与文件** —— 组织、用户、工作区三级作用域，静态加密存储，支持
  组织锁定项。
- **工作区访问** —— OpenSSH、网页终端和鉴权 HTTPS 端口映射，按部署与
  模板启用。
- **管理面** —— 镜像白名单、模板、组织与成员、两级配额和审计日志。
- **API 优先** —— 细粒度 scope 的 API 密钥、游标分页、SSE 事件、签名
  Webhook 和 OpenAPI 契约。
- **WebAssembly 插件** —— 创建策略、API 中间件、自定义路由和控制台
  界面，基于版本化 WIT 接口。

## 架构

MWC 以单一控制面服务的形式运行，由 SQL 数据库支撑。它调用 Kubernetes
API，基于经过评审的模板和白名单镜像为每个工作区创建隔离环境，挂载持久
Home 卷，并在启动时从加密存储注入凭据与文件。用户流量经 SSH、浏览器
终端或鉴权端口映射代理进入工作区。所有操作都通过 REST API 完成，网页
控制台、带 scope 的 API 密钥以及沙箱化的 WebAssembly 插件均构建于其上。

## 快速开始

```bash
cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local
```

Kubernetes 部署请使用 `deploy/helm/memeloop-workspace-control` 中的
Helm Chart。加密注入与 Webhook 需要 `MWC_ENCRYPTION_KEY` 和
`MWC_INTERNAL_AUTH_TOKEN`（从 Secret 注入）。详见
[快速开始文档](docs/quickstart.md)。

## 文档

完整的中英双语文档发布于
<https://memeloop-online.github.io/memeloop-workspace-control/>。

- [快速开始](docs/quickstart.md)
- [工作区](docs/workspaces.md) · [凭据与文件](docs/credentials-and-files.md) · [访问](docs/access.md)
- [管理](docs/administration.md) · [API](docs/api.md) · [插件开发](docs/plugin-development.md)
- [安全模型](docs/security-model.md) · [常见问题](docs/faq.md)

## 许可证

本项目采用 Apache License 2.0 许可协议，详见 [LICENSE](LICENSE) 和
[NOTICE](NOTICE)。
