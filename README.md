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

## 用户设置与 API 密钥

用户资料通过 `/api/v1/me/profile` 更新。头像使用本地上传内容保存为受校验的 PNG、JPEG 或 WebP 数据，单个头像最大 512 KiB；远程图片地址不作为头像来源。未上传头像时，界面使用用户 ID 生成的默认图案。

API 密钥通过 `/api/v1/me/api-keys` 管理。新建密钥必须指定至少一个细粒度 scope，并以 Unix 秒设置未来 365 天以内的过期时间；密钥明文只在创建响应中出现一次。可用 scope 为 `create_workspace`、`read_workspace`、`connect_workspace`、`change_workspace_state`、`delete_workspace`、`manage_organization`、`manage_members`、`manage_locked_injections`、`manage_system` 和 `manage_api_keys`。迁移前创建的通配权限密钥保留兼容性，但新密钥不能再申请通配权限。

## 分页接口

面向规模化部署的列表接口使用游标分页。请求可以携带 `limit`、`cursor` 和 `search`，响应统一返回 `items` 与可选的 `next_cursor`；服务端限制单页大小，客户端使用返回的游标继续加载：

- `GET /api/v1/workspaces?organization_id=<id>`：按组织检索工作区。
- `GET /api/v1/organizations`：检索当前用户可见的组织。
- `GET /api/v1/admin/users`：系统管理员检索用户。
- `GET /api/v1/organizations/<id>/members`：检索组织成员。

游标由服务端生成，客户端按原值回传，不依赖数据库 offset。`search` 用于服务端筛选，适合管理界面中的搜索框和逐页加载。

## 工作区端口映射

工作区就绪后，可以为应用端口创建 HTTP 映射：

- `POST /api/v1/workspaces/<workspace_id>/port-mappings` 创建映射，提交 `internal_port` 和可选的 `display_name`。
- `GET /api/v1/workspaces/<workspace_id>/port-mappings` 返回映射状态和稳定的 `https_url`。
- `POST /api/v1/workspaces/<workspace_id>/port-mappings/<mapping_id>/open` 签发一次性浏览器启动地址。
- `DELETE /api/v1/workspaces/<workspace_id>/port-mappings/<mapping_id>` 删除映射并立即使已有票据和会话失效。

映射使用 `p-<mapping-id>.<portMappingDomain>` 独立主机名，经 Higress 的精确 `p-*` 主机匹配和 external-auth 鉴权后转发到工作区内的 ClusterIP Service。启动地址只用于一次浏览器跳转；交换成功后设置 `__Host-mwc-port-session` 的 `HttpOnly`、`Secure`、`SameSite=Lax` cookie。部署时需为 `*.<portMappingDomain>` 准备 DNS 与 TLS，端口映射不使用 NodePort 或 hostPort。

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
