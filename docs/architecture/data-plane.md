# Data Plane Design

## 目标

客户端在多种网络环境下尽量建立端到端 WireGuard 直连，并在失败时回退到中继。

## 数据面流程

1. 创建或检查 WireGuard 接口。
2. 收集本机 LAN/IPv6 地址。
3. 通过 STUN 获取公网 IPv4 映射。
4. 与目标节点交换候选地址。
5. 并发向各候选路径发起握手。
6. 锁定首个成功路径并热更新 Endpoint。
7. 开启 Keepalive 和质量监控。

## 平台实现策略

### Linux

1. 优先使用内核 WireGuard。
2. 通过 Netlink 和 WireGuard 控制接口进行热更新。
3. 避免依赖 `wg-quick` 重启网卡。

### Windows

1. 使用 Wintun 创建虚拟网卡。
2. 用户态管理 WireGuard 数据面。
3. 保持平台抽象层与 Linux 一致。

## 路径优先级

1. LAN
2. IPv6
3. 公网 IPv4
4. 打洞 IPv4
5. Relay

## 回退原则

1. 打洞超时后立刻回退，不长时间阻塞业务。
2. 中继模式下后台低频重试直连。
3. 一旦恢复直连，优先从中继切回。
