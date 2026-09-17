---
id: credentials-and-files
title: 凭据与文件
sidebar_label: 凭据与文件
sidebar_position: 4
---

# 凭据与文件

MWC 通过**注入项**向工作区注入凭据与文件。注入项只需定义一次，即可在
启动时解析进所有匹配的工作区，因此轮换密钥不需要重建工作区。

## 作用域与级联

注入项存在于三个作用域，按级联顺序应用：

1. **组织** —— 组织内所有工作区共享。
2. **用户** —— 个人凭据，应用到该用户的工作区。
3. **工作区** —— 单个工作区的一次性取值。

同一键上更具体的作用域覆盖更宽泛的作用域。创建前可用
`POST /api/v1/injections/preview` 解析某个计划中的工作区的有效注入集合。

## 注入项类型

| 类型 | 在工作区中的结果 |
| --- | --- |
| `environment_variable` | 名为 `target` 的环境变量。 |
| `secret_file` | 位于路径 `target`、权限受限的文件。 |
| `config_file` | 位于路径 `target` 的文件。 |
| `ssh_public_key` | 加入工作区授权密钥列表的公钥。 |

每个注入项包含：

- `key` —— 作用域内唯一名称。
- `target` —— 变量名或绝对文件路径。
- `value` —— `{ "encoding": "utf8", "value": "..." }` 或
  `{ "encoding": "base64", "value": "..." }`。
- `sensitive` —— 作为只写敏感值保护。普通配置可在控制台查看、编辑和复制。
- `locked` —— 组织管理员可锁定组织级注入项；锁定项始终注入，工作区创建者
  无法取消其选择。
- `file_mode`、`owner`、`group` —— 文件类型的可选文件元数据。
- `template_selector` 与 `labels` —— 限制注入项适用的模板。

配置 `MWC_ENCRYPTION_KEY` 后，取值以 AES-256-GCM 信封加密静态存储。

## 管理注入项

```
GET    /api/v1/injections/{scope}/{scope_id}
PUT    /api/v1/injections/{scope}/{scope_id}/{key}
DELETE /api/v1/injections/{scope}/{scope_id}/{key}
POST   /api/v1/injections/{scope}/{scope_id}/batch-delete
POST   /api/v1/injections/preview
```

`scope` 为 `organization`、`user` 或 `workspace`。

## 按工作区选择

创建工作区时可携带 `organization_injection_refs` 与 `user_injection_refs`：

- 省略或 `null` —— 注入所有经模板与标签选择器匹配的注入项。
- 空数组 `[]` —— 不引用该作用域的任何注入项。
- 键数组 —— 只注入列出的注入项。

锁定的组织级注入项无论选择如何都始终注入。选择随工作区原子持久化，并用
于预览、后续协调与响应来源标注。
