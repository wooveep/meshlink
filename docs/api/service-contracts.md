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

输出：

1. 设备 ID
2. Overlay 地址
3. 当前可见 Peer 列表

约束：

1. 相同 `public_key` 必须返回稳定设备身份与稳定 Overlay IPv4。
2. 返回的 Peer 列表不包含调用者自己。
3. 当请求未携带 `direct_endpoint` 时，服务端必须允许注册，并保留该设备此前已登记的 endpoint。
4. 当请求携带 `direct_endpoint` 时，`host` 与 `port` 都必须完整。

### SyncConfig

输入：

1. 设备 ID

输出：

1. `FULL` 事件：包含调用者自己的 `Device` 视图和当前完整 Peer 集合。
2. `INCREMENTAL` 事件：在本阶段仍发送最新完整 Peer 集合，不使用 patch 语义。
3. 单调递增的 `revision` 字符串，客户端可按字典序比较新旧。
4. `Device.direct_endpoint` 与 `Peer.direct_endpoint` 在 Phase 03 可选出现，用于 Linux 静态直连；此阶段仍不传 NAT 候选。

语义约束：

1. 服务端必须在设备注册视图变化后向在线客户端推送更新。
2. `peers` 字段始终代表“当前可见 Peer 集合”，而不是局部差量。
3. Phase 02 客户端仍可只消费发现信息；Phase 03 Linux 客户端可基于完整视图执行本地 WireGuard 接口写入。

## SignalService

### OpenSignal

采用双向流，最小支持：

1. 候选地址交换
2. 打洞请求
3. 打洞结果
4. 心跳消息

## 演进规则

1. 新增字段必须保持向后兼容。
2. 不在未评审前重命名既有字段。
3. 对消息语义有破坏性影响的改动必须新增 ADR。
