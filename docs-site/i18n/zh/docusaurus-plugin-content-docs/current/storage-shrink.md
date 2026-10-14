---
id: storage-shrink
title: 工作区存储缩容
sidebar_label: 存储缩容
---

# 工作区存储缩容

Home 持久盘和 CSI 临时盘可以离线缩容，并保留当前内容。后台自动停机、
新建小卷、复制、逐文件 SHA-256 与元数据校验、原子交换绑定；原先 Ready 的
工作区会自动恢复，原先已停止的工作区保持停止。关闭页面不会中断后台任务。
工作区处于 `starting`/`stopping` 过渡态时控制台入口暂时隐藏，状态稳定后
重新打开即可继续查看进度；后端会拒绝重叠的缩容请求，操作本身不受影响。

## 管理员启用

在镜像 CI 中构建、审核 `images/storage-shrink` 专用镜像，并在 Helm 配置：

```yaml
workspace:
  storageShrink:
    helperImage: registry.example/mwc-storage-shrink@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    xattrPolicy: posix-user
    longhornNamespace: longhorn-system
```

示例 digest 必须替换成真实审核产物。缺少镜像或明确的 xattr 策略时，API 返回
`503`，不会停机；不会回退使用工作区镜像，也不会在运行时安装依赖。
Helm 仅在启用时授予控制面 PV、StorageClass 和 Longhorn Volume 所需权限。
复制 Pod 没有工作区凭据注入和 ServiceAccount 令牌，源挂载始终只读。

仅支持文件系统 CSI 卷。现有 generic ephemeral scratch 会先确认 StatefulSet
缩放/删除均为 Retain、校验并移除原 Pod UID 对应的 ownerReference，再停机，
随后通过显式 PVC 绑定保留内容。不能安全保留的 `emptyDir` 会被拒绝。
已停止工作区若无保留的 scratch，应先启动再申请 scratch 缩容。

支持数字 uid/gid、权限、纳秒修改时间、符号链接、硬链接组、POSIX ACL 和
`user.*` xattrs。可见的不支持命名空间会导致校验失败。`trusted.*` 不能在无
高级权限时完整认证，因此 `posix-user` 是管理员对平台不使用该命名空间的
明确声明；不会授予 `SYS_ADMIN`，不能把这个功能当任意文件系统的完整备份。

## 一键操作与 API

在初始确认中明确同意“新卷校验且工作区恢复成功后不可逆回收旧卷”，并发送：

```json
{"home_gib":20,"temporary_storage_gib":8,"expected_generation":12,"reclaim_source":true}
```

请求路径为 `POST /api/v1/workspaces/{workspace_id}/storage-shrink`，需要 Bearer
认证和 `Idempotency-Key`，返回 `202`。至少指定一个正整数 GiB 目标，必须小于
当前容量、严格大于离线实测使用量，还需通过新卷实际可用空间/inode 检查。
Owner、组织管理员或系统管理员需要 `change_workspace_state` scope，并遵守
模板限制。读取状态需要 `read_workspace`，且不能窥探其他 owner 的操作。

同路径 `GET` 返回最近操作或 `null`，包含阶段、错误、使用量和卷身份。
阶段通常为 `queued/stopping/measuring/copying/verifying/switching/resuming`。
工具预检在停机前完成；迁移期间生命周期、镜像、放置和其他存储更改返回 `409`。

API 的 `reclaim_source` 默认是 `false`：此时 `completed` 仅表示迁移与恢复成功，
`sources_retained: true / reclaimable: true` 表示旧卷仍占容量，**不能称已省盘**。
确认文件和应用后，可带幂等键调用
`POST /api/v1/workspaces/{workspace_id}/storage-shrink/confirm`，body 为
`{"operation_id":"OPERATION_UUID"}`。

`completed` 的可回收备份未处理前，再次缩容会返回 `409`，避免最近操作状态
遮蔽上一份备份。删除工作区前会补齐并持久化缺失的 PV 身份，再清理绑定卷
与备份；包括创建请求已落地、但尚未记录返回结果的专属目的卷。

初始同意自动回收或随后确认都会进入 `reclaiming`。回收核验 PV UID、claimRef
UID、StorageClass 和 CSI 身份，仅针对原卷改 Delete，并以 UID 前置条件删除。
等待 PVC/PV 和 Longhorn 后端 Volume 全部消失后才进入 `reclaimed`，此时
`sources_retained: false`，才代表旧容量已回收。

## 失败恢复

预检失败先清理辅助 Pod/NetworkPolicy，保留工作区原状态。复制或校验失败不会
切换绑定，恢复源卷并保持停止；切换后恢复失败则保留已验证的新卷绑定和原卷，
不会偷偷丢弃新卷写入。回收失败返回 `completed + error`，可重新确认，运行中的
新工作区不受影响，不能回滚到部分已删除的源卷。

`rolling_back` 会一直保持互斥，直到确认离线安全；外部挂载、UID/所有权变更
需要管理员介入，不能强行删数据。工作区删除会清理本功能的绑定卷和备份。
NetworkPolicy 会验证规则/选择器，但其规则与集群其他允许规则取并集，CNI 和
集群级网络策略仍由管理员负责，不能宣称绝对网络隔离。
