---
id: intro
title: 文档
sidebar_label: 概览
sidebar_position: 1
slug: /
---

# Memeloop Workspace Control 文档

Memeloop Workspace Control（MWC）是一个 Kubernetes 工作区控制面。它从经过
评审的模板交付隔离的、按用户分配的开发工作区，在创建时注入凭据与文件，
并通过 SSH、浏览器终端和带鉴权的 HTTP 端口映射暴露每个工作区。

本文档按使用者组织：

- **用户** —— 创建并连接工作区，管理个人凭据与 API 密钥：从
  [快速开始](./quickstart.md) 开始，然后阅读 [工作区](./workspaces.md)、
  [凭据与文件](./credentials-and-files.md) 和
  [访问：SSH、网页终端、端口映射](./access.md)。
- **管理员** —— 管理镜像、模板、用户、组织、配额与节点池：见
  [管理](./administration.md)。
- **API 使用者** —— 使用带 scope 的 API 密钥认证并通过 REST API 集成：见
  [API 参考](./api.md)。
- **插件开发者** —— 基于 WIT 接口构建 WebAssembly 策略与界面插件：见
  [插件开发](./plugin-development.md)。

跨领域主题：

- [安全模型](./security-model.md) —— 隔离、认证与数据保护保证。
- [常见问题](./faq.md) —— 常见的运维与使用问题。

权威的机器可读 API 契约由每个部署在 `GET /api/v1/openapi.json` 提供。
