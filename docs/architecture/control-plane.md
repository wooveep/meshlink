# Control Plane Design

## 目标

控制面负责设备身份、拓扑状态和配置同步，不承载业务数据。

## 最小职责

1. 设备注册和引导令牌校验。
2. Overlay 地址池分配。
3. Peer 列表和 AllowedIPs 生成。
4. 配置版本与增量更新。
5. 信令长连接维护。
6. 静态路由发布校验与分发。
7. 管理台 HTTP API、设备元数据维护、审计记录与统一观测入口。

## 管理服务边界

`managementd` 需要提供以下稳定接口：

1. 注册设备
2. 拉取完整配置
3. 订阅配置变更
4. 查询设备元数据
5. 通过内置 Hook 链生成最终的 `Peer.allowed_ips`
6. 提供 `/api/admin/v1/*` 浏览器 API 和 `/admin/*` SPA 入口
7. 聚合 `signald` 与 `relayd` 的内部运行态契约

当前内置 Hook：

1. `static_route_advertiser`
   负责把 peer overlay `/32` 与 peer 发布的静态 IPv4 CIDR 合并成最终 `AllowedIPs`。
2. 当前阶段不做 ACL 过滤，策略收敛留给后续 `policy` 模块。
3. 当设备被标记为 `disabled` 时，hook 输入仍保留在管理面状态中，但不会进入任何 peer 可见视图。

## 信令服务边界

`signald` 需要提供以下能力：

1. 设备在线会话标识
2. 首帧 `SignalHello` 鉴权与设备身份校验
3. 候选地址转发
4. 打洞请求与结果回传
5. 心跳超时与会话清理
6. 最小 STUN binding request/response
7. 只对内部管理面暴露运行态 JSON 契约，不直接暴露浏览器 UI

## 首版存储建议

1. SQLite 作为本地开发和单机部署的默认持久层。
2. 管理面首版持久化设备表、标签、静态路由、禁用状态、revision 和 audit event。
3. `SyncConfig` 仍由内存 registry 提供订阅语义，但每次管理面变更必须先持久化到 SQLite。
4. 设计持久层接口时预留 PostgreSQL 替换能力。
