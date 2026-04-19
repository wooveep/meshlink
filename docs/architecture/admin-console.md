# Admin Console

## 目标

首版 server Web 管理台为自托管单机部署提供统一浏览器入口，用于查看
`managementd`、`signald`、`relayd` 的状态，并对设备与拓扑元数据执行有限
管理操作。

## 承载方式

1. `managementd` 同时提供 gRPC 控制面接口和 HTTP 管理入口。
2. `/admin/*` 提供嵌入到 `managementd` 二进制内的 Vue + Vite SPA。
3. `/api/admin/v1/*` 提供浏览器调用的 JSON API。
4. `signald` 与 `relayd` 只暴露内部状态 HTTP 契约，不直接对浏览器提供 UI。

## 首版能力

1. Overview：三服务健康状态、设备总数、在线/离线数量、最近审计事件。
2. Devices：设备列表、筛选、详情、peer 视图查看。
3. Device Detail：编辑展示名、标签、`advertised_routes`，启用/禁用设备，删除陈旧设备。
4. Services：查看 `managementd`、`signald`、`relayd` 的运行态与只读配置摘要。
5. Audit：查看持久化的管理动作和关键系统事件。

## 持久化模型

1. `managementd` 使用 SQLite 作为唯一管理面状态源。
2. SQLite 持久化设备清单、标签、静态路由、禁用状态、最近 `last_seen`、配置 revision 和 audit event。
3. `managementd` 内存中的 registry 继续承担 `SyncConfig` 的订阅与 patch 推送，但所有修改都必须先落 SQLite。
4. 设计保留替换为 PostgreSQL 的空间，但首版不做 HA 或多实例协调。

## 安全与边界

1. 浏览器访问管理台时必须先通过静态 `admin token` 登录。
2. 登录成功后由 `managementd` 下发 HTTP-only session cookie。
3. `admin token` 只作为服务端比较值，不通过 API 回显。
4. 首版不在网页中修改服务级配置，也不引入 ACL/策略编辑。

## 服务协作

1. `managementd` 是唯一对浏览器暴露的管理入口。
2. `signald` 内部状态契约提供在线 session、最近心跳、路由命中/丢失、超时清理统计。
3. `relayd` 内部状态契约提供活跃 relay session、成员、端口、TTL 和清理统计。
4. 禁用设备后：
   1. `managementd` 不再把该设备包含在 peer 视图中。
   2. `signald` 拒绝该设备的新 `SignalHello`。
   3. `relayd` 拒绝该设备参与新的 reservation。

## 交付链路

1. 前端源码位于 `server/admin-ui/`。
2. `scripts/build-admin-ui.sh` 负责安装依赖并把构建产物输出到 `server/internal/adminui/dist/`。
3. `scripts/build-server.sh` 在编译 Go 二进制前先执行前端构建。
4. `server/Dockerfile` 通过 Node 构建 stage 生成前端资源，再把产物嵌入 `managementd`。
