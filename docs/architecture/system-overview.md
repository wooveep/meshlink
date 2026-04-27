# System Overview

## 目标

MeshLink 通过控制面协调与数据面直连的方式，为分布式节点提供自动发现、自动建链和回退中继能力。

## 核心组件

1. `managementd`
   负责注册、鉴权、地址分配、Peer 视图、静态路由发布校验，通过内置 Hook 链生成配置同步视图，并承载统一的 HTTP 管理台入口。
2. `signald`
   负责候选地址交换、连接协商、设备会话管理，并通过内部状态接口暴露在线会话与路由统计。
3. `relayd`
   负责无法直连场景下的回退流量转发，为每对 peer 分配按需创建的 UDP relay session，并通过内部状态接口暴露 relay 运行态。
4. `meshlinkd`
   负责本地 WireGuard 接口、STUN、打洞、Keepalive、relay reservation 和链路切换。
5. 在 Linux NAT punching 场景下，`meshlinkd` 现在可以在内核 WireGuard 前面启用一个
   userspace public UDP proxy：public `listen_port` 由 proxy 持有，内核 WG 只绑定
   loopback/internal port，proxy 负责同端口收发、observed-source 上报和向内核 WG
   转发加密 UDP。

## 数据流

1. 客户端向 `managementd` 注册并获取 Overlay 配置。
2. 客户端可在注册时发布静态 routed subnet；控制面校验并把它并入对端的 `AllowedIPs`。
3. 客户端通过 `signald` 与目标节点交换候选地址。
4. 双方客户端执行并发连接竞速并锁定最优路径。
5. Linux proxy 模式下，peer 的真实 remote endpoint 由 userspace proxy 维护，内核 WG
   peer endpoint 固定为本地 loopback ingress socket。
6. 无法直连时客户端向 `relayd` 申请共享 relay session，并把 WireGuard peer 的 real
   remote endpoint 切到 relay socket。
7. 故障恢复后客户端重新打洞成功，释放 relay reservation 并切回 direct path。
8. 管理员通过 `managementd` 的 `/admin/*` 与 `/api/admin/v1/*` 观察三服务状态并维护设备元数据。

## 系统边界

1. 服务端不应参与正常 P2P 数据转发。
2. Relay 只承载兜底路径，不承担核心流量。
3. 客户端负责最终链路选择，服务端只给出协调信息。
4. `managementd` 是唯一对浏览器暴露的管理入口；`signald` 与 `relayd` 的状态接口仅供内部聚合。
