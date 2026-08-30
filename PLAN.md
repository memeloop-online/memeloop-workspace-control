# Rust Kubernetes 工作区管理平台实施方案

## 总体架构

服务端采用模块化单体架构，同一个 Rust 可执行文件包含：

- REST API 与 OpenAPI。
- 用户、组织、RBAC、额度和审计。
- 内嵌的 React/TypeScript 管理界面。
- Kubernetes 资源协调。
- Secret 与文件级联、加密和物化。
- Higress 路由配置。
- Webhook、SSE 和后台任务。
- 数据库迁移与管理命令。

不拆分独立 Operator、Workspace Agent、自研 SSH Gateway 或自研 Web Terminal。

代码采用单个 Rust crate，按 `auth`、`workspaces`、`injections`、`kubernetes`、`higress`、`quota`、`events`、`admin` 等业务域组织。普通源码文件约 400 行软上限，只在存在独立完整职责时拆分。

## 部署、共存与横向扩展

### 内部轻量模式

- 单副本 StatefulSet。
- SQLite 数据库保存在独立 PVC，启用 WAL。
- 默认关闭用户和组织聚合额度。
- 适合内部、小规模部署。
- SQLite 文件不在多个 Pod 间共享，因此不能横向扩展。

### 公网可扩展模式

- PostgreSQL 保存全部权威状态。
- 服务端以 Deployment 运行多个相同副本。
- HPA 根据 CPU、内存、请求量和任务积压扩缩容。
- API 请求可由任意副本处理。
- 后台任务通过 PostgreSQL `FOR UPDATE SKIP LOCKED` 和过期租约分配给不同副本。
- 每个工作区使用数据库互斥租约，避免重复协调。
- SSE 事件保存在数据库，通过 PostgreSQL通知机制分发。
- 少量全局操作使用 advisory lock，不依赖固定副本。

SQLite 模式可通过内置导出和迁移命令升级到 PostgreSQL 模式。迁移后改用 Deployment，即可增加服务副本分摊 CPU 负载。

### 同集群多实例共存

每套安装必须拥有不可变的 `installation_id`，并执行以下隔离：

- 使用独立 SQLite 文件或独立 PostgreSQL Database/Schema。
- 使用独立 ServiceAccount、RoleBinding、Service、PVC 和配置。
- 工作区 Namespace 名称包含安装前缀，例如 `ws-&lt;installation&gt;-&lt;workspace&gt;`。
- 所有受管资源标记 `owner-installation=&lt;installation_id&gt;`。
- 协调器只查询并修改带有自身所有权标签的资源。
- 删除操作必须同时匹配数据库记录和 Kubernetes 所有权标签。
- API、Web Shell 和管理界面使用不同域名或路径前缀。
- SQLite 实例使用内网 SSH 时，不创建公网跳板路由。
- 公网实例独占其 SSH 域名对应的 Higress TCP 22 入口。

一个内部 SQLite 实例与一个公网 PostgreSQL 实例可正常共存。若多套实例都需要公网 SSH，它们不能同时独占同一个公网 `IP:22`；需使用不同 LoadBalancer IP，或共同接入一套支持多安装来源的共享跳板机。

由于 Kubernetes RBAC 无法按 Namespace 标签限制集群级 Namespace 创建权限，同集群多实例只提供逻辑所有权隔离。互不信任的运营方必须使用独立集群或 vCluster。

## 工作区生命周期

- 每个工作区拥有独立 Namespace。
- 使用单副本 StatefulSet：启动时副本数为 `1`；停止时缩容到 `0`；停止保留 PVC、配置和 SSH 身份。
- 每个工作区创建 ClusterIP Service，SSH 在 Pod 内监听 `2222`。
- 工作区镜像包含标准 OpenSSH Server，并接受平台挂载的 host key、`authorized_keys`、`sshd_config`、Secret 和配置文件。
- 平台提供标准基础镜像；第三方镜像必须通过 Image Contract 检查。

删除流程：

1. 禁止生成新的 Web Shell 和 SSH 授权。
2. 删除 Higress HTTP 路由及跳板机目标授权。
3. 删除工作区 Namespace。
4. 等待 StatefulSet、Pod、Service、Secret、ConfigMap 和 PVC 全部消失。
5. 确认资源清理后才标记为 `deleted`。
6. 数据库只保留不含敏感值的审计墓碑。
7. 管理型 StorageClass 必须使用 `reclaimPolicy: Delete`。

## 工作区存储韧性

工作区必须把持久数据、可再生成数据和连接运行时分开，禁止再把 Home PVC 下的目录
通过 `subPath` 挂载到 `/tmp`、`/var/tmp`、BuildKit 状态目录或 SSH 运行目录：

- Longhorn Home PVC 只保存仓库、用户配置、凭据、Codex 会话和 SQLite 状态。
- `/tmp`、`/var/tmp`、OpenSSH 配置/密钥副本、ttyd 临时文件使用有上限的内存
  `emptyDir`，并与 Home 的剩余空间解耦。
- 编译器临时目录、Cargo target/registry/git cache 和通用包缓存使用有上限的节点本地
  `emptyDir`；现存迁移缓存不自动删除，停机清理为空后再链接到该层。
- BuildKit 配置、状态、socket、缓存和 `buildctl` 使用独立的节点本地 `emptyDir`，其
  就绪状态不能阻止 OpenSSH 提供恢复通道。
- `.codex` 整体保持持久化，只把 `.codex/.tmp` 放入有上限的临时卷。平台不得通过
  sidecar、定时任务或在线 `VACUUM` 并发修改 Codex SQLite。

所有 `emptyDir` 随工作区 Pod 停止或重建而整体清空，不按 mtime 删除活跃文件。模板
声明临时卷上限、容器 `ephemeral-storage` 申请/上限和 Home 紧急预留；平台在 Home
低于 80% 时创建自身拥有的预留文件，在达到 90% 时只释放该预留，为 SQLite 收尾和
人工清理提供空间。Home 写满时，文件注入可以进入明确的降级状态，但授权公钥、host
key、sshd 配置、kubeconfig、SSH 和 ttyd 仍必须依赖独立运行时卷并可启动；Home 缺失、
只读或挂载无效仍是硬错误。

Prometheus 必须记录每个 Home PVC 的实际使用率和承载工作区节点的临时存储申请率，
在 80%/90% 触发 warning/critical，并同时监控节点 `DiskPressure`。Grafana 展示整体、
用户和工作区维度，Loki 复用 Pod 标签收集控制面与工作区日志。上述能力直接复用现有
Kubernetes、Longhorn、Prometheus Operator、Grafana、Alertmanager 和 Loki，不依赖
Tailscale 或 Coder Premium。

Codex 日志数据库的历史膨胀只能在工作区完全停止、确认无进程持有数据库、完成
Longhorn 快照并通过 SQLite 完整性检查后离线压缩。任何自动清理不得删除对话/线程
数据库；即使 Home 已满，平台只承诺 SSH/网页终端恢复通道可用，不承诺 Codex 能继续
写入持久状态。

## 控制面可观测性与诊断

控制面公开 `/livez` 和 `/readyz`：存活检查只验证进程能够服务请求；就绪检查在两秒
内验证权威数据库，不把 Kubernetes 或 Prometheus 的短暂故障扩大为整个 API 下线。
`/metrics` 使用 OpenMetrics，覆盖请求量、模板化路由延迟、错误、活跃请求、SSE、
上游请求、任务队列、进程 RSS、jemalloc、插件执行和平台/用户资源汇总，所有标签使用
固定词表或稳定 ID，禁止 URL、工作区名、插件 ID、错误文本和凭据进入指标。

release 构建保留 pprof 所需符号并编入 CPU 与 jemalloc heap profiling。诊断功能默认
关闭；启用后只复用内部 `8081` 监听器，仍要求内部 Bearer Token，并以有上限的内存
临时卷承载 heap dump。Helm 不为诊断增加 NodePort、hostPort、Service 或 Higress 公网
路由。Prometheus Operator 通过内部 ServiceMonitor 抓取，NetworkPolicy 只放行声明的
监控命名空间。

## 公网 SSH 与端口复用

公网 SSH 使用链路：`SSH 客户端 → Higress 固定 TCP 22 → OpenSSH 跳板机 → 工作区 ClusterIP Service:2222`。

Higress 只维护一条固定 TCPRoute：`ssh.example.com:22 → OpenSSH 跳板机 Service:22`，不为工作区分配公网端口。

跳板机采用标准 OpenSSH：

- 以无状态 Deployment 运行，可水平扩展。
- Higress 按 TCP 连接在跳板机副本间负载均衡。
- 不提供 Shell、PTY、SFTP 或任意目标访问，只允许 `direct-tcpip`。
- 使用 `restrict,port-forwarding,permitopen="目标:2222"` 限制可连接工作区。
- SSH 登录用户名携带工作区短 ID。
- `AuthorizedKeysCommand` 在认证时调用服务端内部鉴权接口，验证用户公钥和工作区权限。
- 内部鉴权接口只参与建立连接，不承载 SSH 数据。
- 工作区停止、删除或权限撤销后，新的连接立即被拒绝。

复制命令形态为 `ssh -J access+工作区短ID@ssh.example.com workspace@工作区内部别名`。UI 同时生成 SSH config，使用户可通过短别名连接。SFTP、SCP 和端口转发使用相同 ProxyJump。内网模式可直接使用 ClusterIP、NodePort 或 Internal LoadBalancer。

## Web Shell

Web Shell 采用现成的 ttyd：

- 工作区 Pod 添加 ttyd sidecar。
- ttyd 通过 Pod 内 SSH 连接主工作区容器。
- 链路为 `浏览器 → Higress HTTP/WebSocket → ttyd → localhost OpenSSH`。
- Higress 在 WebSocket 握手时调用 external-auth。
- 服务端签发限定用户、工作区和有效期的一次性 ticket。
- 终端流量不经过服务端 API。
- NetworkPolicy 只允许 Higress 访问 ttyd。
- 管理界面直接打开或嵌入 ttyd 页面，不实现自有 PTY/WebSocket 协议。

## Secret 与文件级联

注入项支持组织级、用户级、工作区实例级（含创建 API 内联提交），解析优先级为 `组织级 → 用户级 → 工作区实例级`。更具体的作用域覆盖上层；组织管理员可将条目标记为 `locked`，禁止覆盖。

每项支持多行 UTF-8 文本、Base64 二进制值、环境变量、敏感文件、普通配置文件或 SSH 公钥，以及目标路径、权限、属主、属组、模板/标签选择、敏感、锁定、版本和审计属性。

界面必须使用多行编辑器，保留空行、缩进和末尾换行，支持 JSON、YAML、PEM 和多行配置。Secret 保存后只写不可读但允许整体替换；展示最终值来自组织、用户还是工作区；创建前预览覆盖和锁定冲突。

Secret 使用信封加密存储。解析后物化为工作区 Namespace 内的 Kubernetes Secret/ConfigMap；明文不进入日志、审计或 Kubernetes 注解。

## API 与管理界面

使用 `/api/v1` REST JSON API，创建和动作接口支持 `Idempotency-Key`。

创建工作区支持组织、所有者、模板、镜像、CPU、内存、GPU、磁盘、内网或公网访问、组织/用户 Secret 引用、多行文本或 Base64 工作区级注入项、SSH 公钥，以及可选 `wait_until=ready&amp;timeout=...`。

Ready 后返回 ProxyJump SSH 命令、SSH config 片段、跳板机和工作区 host key 信息、Web Shell URL、当前资源、状态和注入来源；不返回私钥或 Secret 明文。

界面包含工作区创建/启动/停止/重启/删除、SSH 命令与配置一键复制、ttyd Web Shell、多行 Secret/文件管理与级联预览、当前 CPU/内存/磁盘/GPU/Pod 事件，以及用户、组织、角色、模板、镜像白名单、额度、审计和扩缩容状态。

## 测试与验收

- 单元测试覆盖权限、额度、Secret 级联、多行内容、状态机、幂等和任务租约。
- 验证 SQLite 单副本恢复和 PostgreSQL 多副本并行处理。
- 验证增加服务副本后任务得到分摊且不重复执行。
- 验证 SQLite 与 PostgreSQL 两套安装可在同一集群共存且不会协调或删除对方资源。
- 使用真实 OpenSSH 测试 ProxyJump、远程命令、SFTP/SCP 和端口转发。
- 验证一个公网 22 端口可同时连接多个工作区，且 `PermitOpen` 阻止跨工作区访问。
- 验证 ttyd、Higress WebSocket、external-auth 和一次性 ticket。
- 验证三级 Secret 覆盖、锁定和 API 内联注入。
- 验证删除后 Namespace、PVC、Secret、路由和跳板授权全部清理。
- 集成和部署测试仅运行在自有 K3s 实例，不建立 Kubernetes/K3s 多版本测试矩阵。

## 明确边界

- SQLite 模式不能横向扩展。
- PostgreSQL 模式可以横向扩展，但 PostgreSQL 和 Kubernetes API Server 仍是共享瓶颈。
- 公网工作区共享一个 Higress SSH 端口。
- 跳板机使用标准 OpenSSH，不实现自定义 SSH 协议。
- 不包含 VS Code、自研 Web Terminal、计费结算、自动休眠或 Kubernetes 多版本认证。
