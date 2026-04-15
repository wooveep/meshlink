# Module Boundaries

## 服务端

1. `ipam`
   只负责地址分配，不负责策略决策。
2. `device`
   只负责设备生命周期和元数据。
3. `peer`
   负责将设备、策略、地址组合为配置视图，并通过 Hook 链生成最终 `AllowedIPs`。
4. `signal`
   只负责转发协商消息，不负责路径决策。
5. `relay`
   只负责兜底转发，不解密业务数据。

## 客户端

1. `api-client`
   只负责与服务端通信。
2. `stun`
   只负责探测和解析外网映射。
3. `holepunch`
   只负责候选竞速和路径选择状态。
4. `wg-manager`
   只负责 WireGuard 接口与 Peer 写入。
5. 平台 crate 只暴露平台差异能力，不包含业务决策。
6. `wintun-windows`
   只负责 Windows embedded tunnel-service、服务生命周期和运行时资产检查，不处理路径选择或控制面协议。

## 修改原则

1. 不把业务状态机塞进平台抽象层。
2. 不把协议转换逻辑散落到多个 crate。
3. 不允许服务端模块互相绕过边界直接读写内部状态。
