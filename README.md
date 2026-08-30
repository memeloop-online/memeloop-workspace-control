# memeloop-workspace-control

Rust 模块化单体 Kubernetes 工作区管理平台。权威范围与验收要求见 [PLAN.md](PLAN.md)。

实现覆盖安装隔离、RBAC、额度与审计、工作区状态机、Image Contract v1、三级注入级联、签名 Webhook、SQLite/PostgreSQL 任务租约、持久 SSE、Kubernetes 资源协调、Higress、标准 OpenSSH 跳板和 ttyd Web Shell。集群集成只在自有 K3S 环境验收，不以本地模拟集群替代。

代码是单个 Rust crate 和单个可执行文件。React/TypeScript 生产构建位于 `web/dist`，通过 `rust-embed` 内嵌到可执行文件。

## 本地运行

```bash
cargo run -- \
  --installation-id demo \
  --listen-address 127.0.0.1:8080 \
  --database-url 'sqlite://data/control-plane.sqlite?mode=rwc' \
  --instance-id local
```

需要加密注入、Webhook 或实际协调工作区时，设置：

```bash
export MWC_ENCRYPTION_KEY="$(openssl rand -base64 32)"
export MWC_INTERNAL_AUTH_TOKEN='replace-with-at-least-32-random-bytes'
export MWC_KUBERNETES_ENABLED=true
export MWC_TTYD_IMAGE='tsl0922/ttyd:1.7.7'
```

生产环境应从 Secret 注入这些值，不应写入命令历史。服务镜像由根目录 `Dockerfile` 构建；标准工作区与 OpenSSH 跳板镜像分别位于 `images/workspace-base` 和 `images/ssh-jump`。

`main` 分支通过 GitHub Actions 在全部质量门禁通过后发布并生成构建来源证明：

- `ghcr.io/memeloop-online/memeloop-workspace-control`
- `ghcr.io/memeloop-online/memeloop-workspace-control-workspace`
- `ghcr.io/memeloop-online/memeloop-workspace-control-ssh-jump`

提交 `d69f180f8f39e90674b72b633c6a3badda69509d` 经
[CI run 33334763871](https://github.com/memeloop-online/memeloop-workspace-control/actions/runs/33334763871)
验证并发布的不可变摘要为：

- 控制面：`sha256:d17e98bc7b37d3af51d856e034b34c9c6a6561a2be5c3c6107db3c6dd18c0965`
- 标准工作区：`sha256:66c2cb29d2d9c6c17c3d80113cea4ea024cc3e95f4f1ededb1e79645f45164cc`
- OpenSSH 跳板：`sha256:ccfa8d6155fab13dafab576bd37746ee9f927b1e43c8fefa7a7659516ac599b0`

探针与 OpenAPI：

- `GET /livez`（`/healthz` 保留为兼容别名）
- `GET /readyz`
- `GET /api/v1/system/info`
- `GET /api/v1/openapi.json`
- `GET /metrics`

Prometheus/Grafana 指标、Loki 标签和大规模部署的抓取成本约束见
[可观测性说明](docs/OBSERVABILITY.md)。

SQLite 模式会强制 `--replica-count 1` 并启用 WAL；PostgreSQL URL 可用于多副本模式。数据库首次迁移后会绑定不可变的安装 ID，其他安装 ID 无法复用该数据库。

工作区创建采用默认拒绝的 Image Contract v1 白名单；首次创建前，系统管理员必须先通过管理界面或 `PUT /api/v1/admin/images` 明确允许标准或已验收的第三方 OCI 镜像。

创建请求中的 `organization_injection_refs` 与 `user_injection_refs` 支持显式选择注入项：省略或传 `null` 表示使用全部经过模板/标签选择器匹配的项，传空数组表示不引用该级别，传键数组表示只引用指定项。组织管理员锁定的组织项始终强制注入，不能通过空数组或自选列表绕过。选择会随工作区原子持久化，并用于预览、后续协调、响应来源和 SQLite→PostgreSQL 快照迁移。

## 数据迁移

SQLite 快照包含权威业务状态和加密密文，不包含一次性 ticket、幂等缓存或活动租约。目标 PostgreSQL 必须为空且使用同一个 installation ID。

```bash
memeloop-workspace-control --installation-id internal-a database export --output snapshot.json
memeloop-workspace-control --installation-id internal-a \
  --database-url "$POSTGRES_URL" database import --input snapshot.json
```

也可以使用 `database migrate-to-postgres` 完成导出与导入。导入使用 PostgreSQL advisory lock，后台任务恢复为 pending 状态。

## Helm

Chart 位于 `deploy/helm/memeloop-workspace-control`：

- `mode=sqlite` 渲染单副本 StatefulSet 与独立 RWO PVC。
- `mode=postgresql` 渲染 Deployment 与 HPA。
- 公网 SSH 始终只有一条固定 TCPRoute，后端是标准 OpenSSH 跳板 Deployment。
- 每个工作区在自身 Namespace 创建 `networking.k8s.io/v1` Ingress，将
  `/shell/<short>/` 原样转发给使用相同 `--base-path` 的 ttyd；Higress external-auth
  消费一次性 ticket。Web Shell 不依赖 Gateway API CRD 或跨 Namespace ReferenceGrant。

公网 PostgreSQL 示例见 `values.example.yaml`，内网 SQLite 示例见
`values.internal.example.yaml`。安装前需提供数据库、信封加密、内部鉴权和跳板 host key Secret，并确认管理型 StorageClass 的 `reclaimPolicy` 为 `Delete`。
Chart 为控制面和标准 OpenSSH 跳板提供了默认 CPU/内存 requests 与 limits；PostgreSQL
启用 HPA 时若移除控制面 CPU 或内存 request，模板会直接拒绝渲染。

## 验收

本地门禁验证纯业务、密文、迁移、租约、资源模型、HTTP API、SSE、OpenSSH 授权输出和嵌入 UI。真实 ProxyJump/SFTP/SCP/端口转发、ttyd/Higress、PostgreSQL 多副本与安装共存必须按 [K3S 验收清单](docs/K3S-ACCEPTANCE.md) 在自有 K3S 实例执行。
清单配套的 `scripts/k3s/preflight.sh`、`verify-installation.sh` 和
`verify-workspace-cleanup.sh` 均为只读检查，并要求显式指定目标安装信息。

## 质量门禁

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cd web && npm ci && npm run check && npm run build
```
