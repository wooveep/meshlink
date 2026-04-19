# Service Contracts

## Protocol First

任何跨进程、跨语言、跨模块的接口，都必须先更新 `proto/` 与本文件。

## ManagementService

### RegisterDevice

输入：

1. 设备名称
2. WireGuard 公钥
3. 引导令牌
4. 操作系统和版本信息
5. 可选的 `direct_endpoint`，用于 Linux 静态直连阶段发布可达地址与监听端口
6. 可选的 `advertised_routes`，用于发布该节点后方的静态 IPv4 CIDR

输出：

1. 设备 ID
2. Overlay 地址
3. 当前可见 Peer 列表
4. 当前设备视图中的 `advertised_routes`

约束：

1. 相同 `public_key` 必须返回稳定设备身份与稳定 Overlay IPv4。
2. 返回的 Peer 列表不包含调用者自己。
3. 当请求未携带 `direct_endpoint` 时，服务端必须允许注册，并保留该设备此前已登记的 endpoint。
4. 当请求携带 `direct_endpoint` 时，`host` 与 `port` 都必须完整。
5. `advertised_routes` 在首版只支持静态 IPv4 CIDR，必须先规范化、去重、排序后再入库。
6. `advertised_routes` 不允许包含 `0.0.0.0/0`，不允许与 overlay CIDR 重叠，也不允许与其他设备已发布路由重叠。
7. 当请求携带空的 `advertised_routes` 列表时，服务端必须清空该设备此前发布的所有路由。

### SyncConfig

输入：

1. 设备 ID

输出：

1. `FULL` 事件：包含调用者自己的 `Device` 视图和当前完整 Peer 集合，使用 `peers` 字段承载完整快照。
2. `INCREMENTAL` 事件：使用 `peer_upserts` 与 `removed_peer_ids` 承载 peer patch；`peers` 不再表示完整视图。
3. 单调递增的 `revision` 字符串，客户端可按字典序比较新旧。
4. `Device.direct_endpoint` 与 `Peer.direct_endpoint` 在 Phase 03 可选出现，用于 Linux 静态直连；此阶段仍不传 NAT 候选。
5. `Device.advertised_routes` 反映当前设备已经发布并通过校验的静态路由集合。
6. `Peer.allowed_ips` 现在由控制面 Hook 链生成，语义为“对端 overlay /32 + 对端已发布静态路由”。
7. `Device.disabled=true` 表示该设备仍保留在管理面状态中，但不会再被分发到其他 peer 视图。
8. `Device.last_seen_unix` 反映控制面最近一次持久化到该设备状态的时间戳。

语义约束：

1. 服务端必须在设备注册视图变化后向在线客户端推送更新。
2. `FULL.peers` 始终代表“当前完整可见 Peer 集合”；`INCREMENTAL.peer_upserts` 只包含新增或变更的 peer，`INCREMENTAL.removed_peer_ids` 只包含被移除的 peer。
3. 客户端必须把 `INCREMENTAL` 应用到本地 peer cache 上，再基于 cache 的收敛视图执行本地 WireGuard 接口写入。
4. 首版 `managementd` 内置 `static_route_advertiser` Hook，不做 ACL 过滤，所有可见 peer 都接收这些静态路由。
5. 当设备被禁用时，服务端对该设备自身仍可返回 `Self` 视图，但 `Peers` / `peer_upserts` 中不再包含它，也不再向其他设备广播它。

## SignalService

### OpenSignal

采用双向流。

首帧约束：

1. 第一帧必须是 `SignalHello`。
2. `SignalHello.device_id`、`SignalHello.public_key`、`SignalHello.bootstrap_token` 都不能为空。
3. `signald` 必须校验 bootstrap token，并通过 `ManagementService.GetDevice` 校验设备存在且公钥一致。
4. 同一 `device_id` 只允许一个活跃 session；新连接必须顶掉旧连接。

运行时消息：

1. 候选地址交换
2. 打洞请求
3. 打洞结果
4. 心跳消息

消息约束：

1. `SignalEnvelope.session_id` 用于串联同一 peer 协商过程。
2. `CandidateAnnouncement` 与 `PunchRequest` 在 Phase 05 只承载 `LAN` 与 `PUBLIC_IPV4` candidates。
3. `PunchResult.success=false` 只表示本次直连协商失败；Phase 06 客户端可在本地据此触发 Relay fallback。
4. 服务端只转发消息与管理会话，不负责路径选择，也不持久化 NAT candidates。
5. 当 `ManagementService.GetDevice` 返回 `Device.disabled=true` 时，`signald` 必须拒绝新的 `SignalHello`。

## RelayService

### ReservePeerRelay

输入：

1. 发起设备的 `device_id`
2. 发起设备的 `public_key`
3. 引导令牌
4. 目标 `peer_id`

输出：

1. `relay_host`
2. 动态分配的 `udp_port`
3. `ttl_seconds`
4. `session_id`

约束：

1. `relayd` 必须校验 bootstrap token，并通过 `ManagementService.GetDevice` 校验调用方设备存在且公钥一致。
2. `peer_id` 必须存在于管理面。
3. 同一对设备的重复 reserve 必须复用同一个 relay session 与 UDP 端口，并刷新 TTL。

### ReleasePeerRelay

输入：

1. 发起设备的 `device_id`
2. 发起设备的 `public_key`
3. 引导令牌
4. 目标 `peer_id`
5. `session_id`
6. 释放原因 `reason`

语义约束：

1. `session_id` 必须匹配当前设备对的活跃 relay session。
2. 客户端在 peer 移除、signal loop 重连、或 direct path 恢复后必须主动释放 reservation；服务端 TTL 只作为兜底清理。
3. Relay state 不回写 `ManagementService` 的 peer 视图，路径选择仍由客户端本地维护。
4. 当 `peer_id` 或调用方设备处于 `disabled` 状态时，`relayd` 必须拒绝新的 reservation。

## Admin HTTP API

由 `managementd` 提供：

1. `/admin/*`
   承载嵌入式 Vue SPA。
2. `/api/admin/v1/session/login`
   使用静态 `admin token` 建立会话 cookie。
3. `/api/admin/v1/overview`
   返回三服务状态摘要、设备总量、在线/离线统计和最近审计事件。
4. `/api/admin/v1/devices`
   返回设备列表；`PATCH /devices/:id` 支持修改展示名、标签和 `advertised_routes`；`POST /devices/:id/disable|enable` 控制设备启停；`DELETE /devices/:id` 删除陈旧设备。
5. `/api/admin/v1/services`
   返回 `managementd`、`signald`、`relayd` 的聚合运行态。
6. `/api/admin/v1/config`
   返回只读配置摘要，不回显敏感 token 值。
7. `/api/admin/v1/audit`
   返回持久化的管理动作和关键系统事件。

## Internal Status Contracts

1. `signald` 提供 `/internal/status/v1/service`
   返回在线 session、最近心跳、route hit/miss 和过期清理统计。
2. `relayd` 提供 `/internal/status/v1/service`
   返回活跃 relay session、成员、端口、TTL 和清理统计。
3. 这些状态契约只供 `managementd` 聚合，不直接向浏览器暴露。

## 演进规则

1. 新增字段必须保持向后兼容。
2. 不在未评审前重命名既有字段。
3. 对消息语义有破坏性影响的改动必须新增 ADR。
