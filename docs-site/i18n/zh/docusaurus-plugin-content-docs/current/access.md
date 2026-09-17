---
id: access
title: 访问：SSH、网页终端、端口映射
sidebar_label: 访问（SSH / 终端 / 端口）
sidebar_position: 5
---

# 访问：SSH、网页终端、端口映射

就绪的工作区可提供 SSH、网页终端和 HTTPS 端口映射。实际显示的入口由所选
模板和部署的访问配置决定。

## SSH

内网部署可通过 `workspace.internalSshHost` 发布每个工作区的 SSH 端口；公网
部署可通过 `images/ssh-jump` 中的标准 OpenSSH 跳板机转发连接。认证使用
调用方自己的密钥对：

1. 通过 `GET /api/v1/workspaces/{workspace_id}/ssh-client-public-key`
   注册或获取该工作区的客户端公钥。
2. 从工作区页面复制连接信息。公网连接使用跳板机，内网连接使用配置的主机
   和系统分配的工作区端口。

可用 `ssh_public_key` 类型的注入项向工作区注入额外公钥（见
[凭据与文件](./credentials-and-files.md)）。

## 网页终端

部署配置了网页终端入口且模板启用此访问方式时，控制台会显示基于 ttyd 的
网页终端：

1. 客户端通过 `POST /api/v1/workspaces/{workspace_id}/web-shell-tickets`
   申请一个短期、一次性票据。
2. 浏览器连接工作区的 ttyd 端点，ttyd 依据控制面的内部鉴权端点校验票据。

进入工作区的一跳使用双向 TLS；ttyd 服务端构建会校验客户端证书，因此只有
平台代理能到达终端上游。票据快速过期且不可复用。

## 端口映射

部署配置端口映射域名后，可以把工作区内的应用端口发布为带鉴权的 HTTPS
URL：

- `POST /api/v1/workspaces/{workspace_id}/port-mappings` —— 创建映射，
  提交 `internal_port` 与可选的 `display_name`。
- `GET /api/v1/workspaces/{workspace_id}/port-mappings` —— 列出映射状态
  与稳定的 `https_url`。
- `POST /api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}/open`
  —— 签发一次性浏览器启动地址。
- `DELETE /api/v1/workspaces/{workspace_id}/port-mappings/{mapping_id}` ——
  删除映射并立即使已有票据与会话失效。

工作原理：

- 每个映射获得独立主机名 `p-<mapping-id>.<portMappingDomain>`，由 Higress
  网关路由到工作区前的 ClusterIP Service。映射不使用 NodePort 或 hostPort。
- 一次性启动地址只用于一次浏览器跳转，随后设置
  `__Host-mwc-port-session` cookie（`HttpOnly`、`Secure`、`SameSite=Lax`）。
  后续请求由网关的 external-auth 检查对控制面鉴权。
- 部署需要为 `*.<portMappingDomain>` 准备通配 DNS，并在 Higress
  `credentialConfig` 中集中配置通配证书（含 `fallbackForInvalidSecret`）。
  证书私钥不会复制到工作区 Namespace。
