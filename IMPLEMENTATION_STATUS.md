# 待办

- [ ] 跨节点入口：处理 SNAT 下来源选择器失效的边界；网络可达不能替代 mTLS 身份验证。
- [ ] 网络隔离：补齐动态节点公网地址阻断，完成产品出口、最小来源入口和外部沙箱端到端验收。
- [ ] 网络配置更新：等待启动刷新修复的 CI，通过后 GitOps 发布；确认已有 Ready 工作区策略更新且 Pod 不重启。
- [ ] 沙箱验收：验证网络逃逸、权限/凭据边界、CPU/内存/磁盘/PID 压力、SSH/Web Shell、重启和重调度，并保持失败关闭。
- [ ] 完成 gVisor 真实工作区验收及正式 RuntimeClass 注册。
- [ ] 操作员流程：补齐符合条件节点的注册、canary 和恢复检查。
- [ ] 最后 Coder TOKEN center dev：按 `docs/FINAL-MIGRATION-RUNBOOK.md` 由外部代理迁移，当前工作区不可自停。
