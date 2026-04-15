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

## 管理服务边界

`managementd` 需要提供以下稳定接口：

1. 注册设备
2. 拉取完整配置
3. 订阅配置变更
4. 查询设备元数据
5. 通过内置 Hook 链生成最终的 `Peer.allowed_ips`

当前内置 Hook：

1. `static_route_advertiser`
   负责把 peer overlay `/32` 与 peer 发布的静态 IPv4 CIDR 合并成最终 `AllowedIPs`。
2. 当前阶段不做 ACL 过滤，策略收敛留给后续 `policy` 模块。

## 信令服务边界

`signald` 需要提供以下能力：

1. 设备在线会话标识
2. 首帧 `SignalHello` 鉴权与设备身份校验
3. 候选地址转发
4. 打洞请求与结果回传
5. 心跳超时与会话清理
6. 最小 STUN binding request/response

## 首版存储建议

1. SQLite 作为本地开发和单机部署的默认持久层。
2. 将地址池、设备表、Peer 关系、策略版本单独建模。
3. 设计持久层接口时预留 PostgreSQL 替换能力。
