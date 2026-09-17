---
id: quickstart
title: 快速开始
sidebar_label: 快速开始
sidebar_position: 2
---

# 快速开始

本页让控制面在本地运行起来，并走完创建第一个工作区的流程。在 Kubernetes
上的生产部署请使用仓库中的 Helm Chart
`deploy/helm/memeloop-workspace-control`。

## 本地运行控制面

MWC 是单个 Rust 可执行文件。先创建数据库目录和首位系统管理员：

```bash
mkdir -p data
export MWC_ADMIN_TOKEN="$(openssl rand -base64 32)"

cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local \
  admin create-user --display-name Administrator --system-admin
```

`MWC_ADMIN_TOKEN` 是首个 API 密钥。请将它保存到密码管理器。启用加密注入
与签名 Webhook 时，先设置两个独立的随机值：

```bash
export MWC_ENCRYPTION_KEY="$(openssl rand -base64 32)"
export MWC_INTERNAL_AUTH_TOKEN="$(openssl rand -base64 32)"
```

在同一终端使用相同的数据库和安装参数启动服务：

```bash
cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local
```

打开第二个终端，设置服务地址和已保存的首个 API 密钥：

```bash
export BASE='http://127.0.0.1:8080'
export API_KEY='<保存的 MWC_ADMIN_TOKEN 值>'
curl "$BASE/api/v1/me" -H "Authorization: Bearer $API_KEY"
```

Web 控制台由同一监听器提供。健康端点：

- `GET /livez` —— 进程存活。
- `GET /readyz` —— 数据库连通性检查。
- `GET /api/v1/system/info` —— 版本与安装元数据。

工作区调度还需要 Kubernetes 访问。通过有效的 kubeconfig 或服务账户运行
进程，并设置：

```bash
export MWC_KUBERNETES_ENABLED=true
export MWC_TTYD_IMAGE='tsl0922/ttyd:1.7.7'
```

Kubernetes 部署应从 Secret 注入加密密钥与内部鉴权令牌；Helm Chart 会将
这些值传入控制面。

## 创建第一个工作区

1. **放行镜像。** 工作区镜像默认拒绝（Image Contract v1）。系统管理员必须
   先通过管理控制台或 `PUT /api/v1/admin/images` 明确放行镜像。
2. **创建模板。** 模板描述一类工作区的镜像、资源、存储、网络出口策略与
   调度约束。可在控制台管理，或通过 `POST /api/v1/templates`。
3. **添加凭据。** 在组织、用户或工作区作用域定义环境变量、文件或 SSH
   公钥。见[凭据与文件](./credentials-and-files.md)。
4. **创建工作区。** 从控制台或相应 API 响应中复制组织、所有者和模板 ID，
   然后运行：

   ```bash
   export ORG_ID='<organization-id>'
   export OWNER_ID='<user-id>'
   export TEMPLATE_ID='<template-id>'

   curl -X POST "$BASE/api/v1/workspaces" \
     -H "Authorization: Bearer $API_KEY" \
     -H "Idempotency-Key: workspace-$(date +%s)" \
     -H 'Content-Type: application/json' \
     -d "{\"organization_id\":\"$ORG_ID\",\"owner_id\":\"$OWNER_ID\",\"name\":\"my-workspace\",\"template_id\":\"$TEMPLATE_ID\"}"
   ```

5. **连接。** 工作区就绪后，通过 SSH、浏览器终端连接，或发布应用端口。
   见[访问](./access.md)。

## 下一步

- [工作区](./workspaces.md) —— 生命周期、状态、存储、调度。
- [API 参考](./api.md) —— API 密钥、scope、分页、事件、Webhook。
- [安全模型](./security-model.md) —— 平台提供的保证。
