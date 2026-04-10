# WireGuard SD-WAN 项目规划

## 1. 项目描述

### 1.1 项目名称

MeshLink: 基于 WireGuard 的分布式 SD-WAN 组网系统

### 1.2 项目目标

构建一个面向多节点、多地域、跨 NAT 网络环境的轻量级 SD-WAN 系统。系统以 WireGuard 作为加密隧道基础，提供控制面与数据面分离的 Mesh 组网能力，重点解决以下问题：

1. 节点位于家庭宽带、企业出口、云主机、移动网络等复杂网络环境下的自动互联。
2. 在 IPv4 下通过 STUN、候选地址交换和 UDP 打洞尽可能实现直连。
3. 在 IPv6 下通过地址发现与双向主动发包实现防火墙 pinholing 直连。
4. 当直连失败时自动降级到中继模式，优先保障连通性。
5. 提供后续可扩展的 ACL、设备管理、审计、策略下发和可观测能力。

### 1.3 目标用户

1. 中小企业的分支互联与远程办公网络。
2. 自建跨云内网互通场景。
3. 实验室、开发团队、边缘节点的安全 Overlay 网络。
4. 需要自托管、可控、可二次开发的 WireGuard 组网平台。

### 1.4 核心价值

1. 基于 WireGuard，性能高、配置简单、加密现代化。
2. 默认走 P2P，减少中心转发带宽成本与时延。
3. 控制面集中治理，数据面尽量直连，兼顾易管理与高性能。
4. 跨 Linux/Windows 客户端，覆盖主流部署环境。

### 1.5 非目标

1. 第一阶段不做完整 SASE、零信任门户或浏览器代理。
2. 第一阶段不支持复杂流量工程和 BGP/OSPF 动态路由。
3. 第一阶段不追求 iOS/Android 原生客户端。
4. 第一阶段不以替代商用 SD-WAN 控制器为目标。

## 2. 产品范围与 MVP 定义

### 2.1 MVP 范围

MVP 只解决“节点可注册、可发现、可打洞、可直连、失败可回退”五件事。

MVP 必须具备：

1. 管理服务：设备注册、密钥登记、Overlay 地址分配、配置下发。
2. 信令服务：候选地址交换、连接协商消息转发。
3. 客户端守护进程：设备注册、地址收集、STUN 探测、状态机驱动。
4. WireGuard 配置热更新：动态新增或更新 Peer，不重启网卡。
5. 中继回退：双端直连失败时切换到 Relay。
6. 基础可观测：日志、节点在线状态、握手结果、连接路径类型。

### 2.2 首版成功标准

1. 两台 Linux 节点可在公网 IPv4 下建立 WireGuard 直连。
2. Linux 与 Windows 可跨平台建链并互通 Overlay 地址。
3. 同局域网、IPv4 NAT、IPv6 公网三类网络下均可自动选择最优路径。
4. 双边复杂 NAT 无法直连时，系统能在超时后自动切到中继链路。
5. 节点网络变化后，客户端能够自动重新探测并恢复可用链路。

## 3. 总体架构

### 3.1 架构原则

1. 控制面与数据面分离。
2. 优先直连，失败回退。
3. 客户端自治，服务端协调。
4. 配置热更新，不中断已建立连接。
5. 所有跨组件交互采用明确契约定义。

### 3.2 逻辑架构

```text
+------------------+        +------------------+
|  Management API  |        |   Signal API     |
|  注册/鉴权/配置  |        | 候选交换/协商转发 |
+---------+--------+        +---------+--------+
          |                           |
          +------------+--------------+
                       |
                +------+------+
                |   Metadata  |
                | DB / Cache   |
                +------+------+
                       |
        -----------------------------------------
        |                   |                   |
+-------+-------+   +-------+-------+   +-------+-------+
|  Client Node A |   |  Client Node B |   |  Relay Node   |
|  Linux/Windows |   |  Linux/Windows |   |  Optional      |
+-------+-------+   +-------+-------+   +-------+-------+
        |                   |
        +------ P2P WireGuard Tunnel ------+
```

### 3.3 组件职责

#### 管理服务 Management Service

1. 设备注册与身份校验。
2. Overlay IPv4/IPv6 分配。
3. 设备拓扑、Peer 关系、ACL 与策略维护。
4. 客户端配置同步与增量更新。

#### 信令服务 Signal Service

1. 转发候选地址与探测协商消息。
2. 提供低延迟双向流通信。
3. 不承载业务流量，只处理控制消息。

#### 中继服务 Relay Service

1. 双边无法直连时承载加密后的中继流量。
2. 不解密 WireGuard 数据载荷。
3. 作为最终可达性保障路径。

#### 客户端 Agent

1. 管理本地 WireGuard 接口。
2. 收集本地候选地址和公网映射。
3. 执行连接竞速和路径切换状态机。
4. 维护 Keepalive、重连和探测任务。

## 4. 技术选型

### 4.1 服务端

1. 语言：Go 1.22+
2. RPC：gRPC + Protobuf
3. 存储：SQLite 起步，PostgreSQL 作为生产版目标
4. 缓存与在线状态：内存 + Redis 可选
5. 部署：Docker Compose 起步，Kubernetes 为后续阶段选项

选择 Go 的原因：

1. 并发模型适合控制面和长连接管理。
2. gRPC 生态成熟。
3. 适合快速落地服务端脚手架与后台治理能力。

### 4.2 客户端

1. 语言：Rust 1.75+
2. 异步运行时：Tokio
3. gRPC 客户端：Tonic
4. Linux：优先走内核态 WireGuard + Netlink 控制
5. Windows：Wintun + 用户态 WireGuard 实现

选择 Rust 的原因：

1. 更适合系统编程、网络并发和内存安全要求。
2. 便于统一 Linux/Windows 的底层能力抽象。
3. 有利于长期演进为高性能跨平台 Agent。

## 5. 建议项目框架

建议采用 Monorepo，统一管理协议、服务端、客户端和部署配置。

```text
meshlink/
├── docs/
│   ├── architecture/
│   ├── api/
│   ├── roadmap/
│   └── adr/
├── proto/
│   ├── management.proto
│   ├── signal.proto
│   └── common.proto
├── server/
│   ├── cmd/
│   │   ├── managementd/
│   │   ├── signald/
│   │   └── relayd/
│   ├── internal/
│   │   ├── app/
│   │   ├── auth/
│   │   ├── device/
│   │   ├── peer/
│   │   ├── ipam/
│   │   ├── policy/
│   │   ├── signal/
│   │   ├── relay/
│   │   ├── store/
│   │   └── observability/
│   ├── pkg/
│   └── migrations/
├── client/
│   ├── crates/
│   │   ├── agent-core/
│   │   ├── api-client/
│   │   ├── stun/
│   │   ├── holepunch/
│   │   ├── wg-manager/
│   │   ├── netlink-linux/
│   │   ├── wintun-windows/
│   │   ├── relay-client/
│   │   ├── telemetry/
│   │   └── cli/
│   ├── bin/
│   │   └── meshlinkd/
│   └── tests/
├── deploy/
│   ├── docker-compose/
│   ├── systemd/
│   ├── k8s/
│   └── examples/
├── scripts/
│   ├── gen-proto.sh
│   ├── lint.sh
│   └── test-e2e.sh
├── tests/
│   ├── integration/
│   ├── e2e/
│   └── nat-lab/
├── Makefile
├── README.md
└── WireGuard SD-WAN 项目规划.md
```

## 6. 模块拆分

### 6.1 服务端模块

1. `auth`
   负责设备身份校验、令牌签发、后续接入 OIDC 的扩展点。
2. `device`
   管理节点注册、在线状态、设备元数据。
3. `ipam`
   负责 Overlay 地址池管理与冲突检测。
4. `peer`
   计算节点可见性、Peer 列表与 AllowedIPs。
5. `signal`
   管理长连接和候选地址消息路由。
6. `relay`
   负责中继模式数据通道和限流控制。
7. `policy`
   提供 ACL、分组和策略版本控制。
8. `observability`
   日志、指标、链路诊断和事件审计。

### 6.2 客户端模块

1. `agent-core`
   负责主状态机、任务调度、配置缓存。
2. `api-client`
   对接管理服务和信令服务。
3. `stun`
   实现公网地址探测与 NAT 类型推断。
4. `holepunch`
   并发发起 IPv4/IPv6/LAN 候选连接竞速。
5. `wg-manager`
   统一 WireGuard 接口创建、Peer 热更新、Keepalive 管理。
6. `netlink-linux`
   封装 Linux 下的链路、路由、地址和 WireGuard 控制。
7. `wintun-windows`
   封装 Windows 虚拟网卡与用户态数据通道。
8. `relay-client`
   负责回退中继连接与恢复直连重试。
9. `telemetry`
   上报客户端日志、事件、握手质量数据。

## 7. 核心流程

### 7.1 节点上线流程

1. 客户端启动并加载本地密钥。
2. 连接管理服务完成注册和鉴权。
3. 获取 Overlay 地址、Peer 视图和策略。
4. 收集本地候选地址并执行 STUN 探测。
5. 通过信令服务与目标节点交换候选信息。
6. 执行连接竞速，锁定最优链路。
7. 动态写入 WireGuard Peer 配置并进入保活状态。

### 7.2 路径选择优先级

1. 同机房或同 LAN 直连
2. IPv6 直连
3. IPv4 公网直连
4. IPv4 UDP 打洞直连
5. 中继回退

### 7.3 客户端状态机

```text
INIT
  -> REGISTERED
  -> CONFIGURED
  -> DISCOVERING
  -> SIGNALING
  -> PUNCHING
  -> VERIFYING
  -> ESTABLISHED
  -> DEGRADED_RELAY
  -> RECONNECTING
```

## 8. Todo 清单

### 8.1 P0: 必做

1. 初始化 Monorepo 目录结构。
2. 定义 `management.proto`、`signal.proto`、`common.proto`。
3. 实现管理服务最小能力：注册、设备查询、地址分配。
4. 实现信令服务最小能力：双向流、候选消息转发。
5. 实现 Rust 客户端最小骨架与配置持久化。
6. 实现 Linux 下 WireGuard 接口创建与 Peer 热更新。
7. 实现 STUN 探测与候选地址收集。
8. 实现客户端打洞状态机。
9. 实现首次直连成功后的 Endpoint 锁定。
10. 实现基础日志和错误码体系。
11. 搭建 Docker Compose 本地联调环境。
12. 输出最小安装和使用文档。

### 8.2 P1: 高优先级

1. Windows Wintun 集成。
2. 中继回退通道。
3. ACL 和节点分组能力。
4. 客户端网络变化检测与自动重连。
5. 管理端简单 Web API 或 CLI。
6. 连接质量指标采集：时延、握手耗时、丢包估算。
7. 端到端集成测试与 NAT 实验环境。

### 8.3 P2: 后续增强

1. OIDC/SSO 登录。
2. 多租户支持。
3. 高可用控制面部署。
4. 节点标签、策略模板、审计日志。
5. 智能路径评分与链路切换策略。
6. Web 控制台。
7. 移动端接入。

## 9. 路线图

以下路线图按 16 周首版计划估算，可根据团队规模压缩或扩展。

### Phase 0: 方案冻结与仓库搭建（第 1 周）

目标：

1. 冻结 MVP 范围。
2. 初始化仓库、代码规范、CI、Proto 流水线。

交付物：

1. Monorepo 初始目录。
2. README、开发约定、Makefile。
3. 第一版协议草案。

### Phase 1: 控制面最小闭环（第 2-4 周）

目标：

1. 实现设备注册、地址分配、配置下发。
2. 打通客户端与服务端的基本通信。

交付物：

1. `managementd` 可运行。
2. SQLite 设备表与地址池。
3. 客户端可注册并获取基础配置。

验收标准：

1. 新节点启动后可得到唯一 Overlay 地址。
2. 服务端可查询在线节点及公钥。

### Phase 2: Linux 数据面落地（第 5-7 周）

目标：

1. 在 Linux 上完成 WireGuard 接口创建和静态建链。
2. 支持 Peer 动态增删改。

交付物：

1. `wg-manager` Linux 实现。
2. 节点间静态隧道联通。
3. 热更新 Peer 验证脚本。

验收标准：

1. 不重启网卡即可更新 Endpoint 和 AllowedIPs。
2. 两台 Linux 可完成加密互通。

### Phase 3: 动态发现与打洞（第 8-10 周）

目标：

1. 完成候选地址收集、STUN 探测和信令交换。
2. 建立 IPv4/IPv6/LAN 多路径竞速机制。

交付物：

1. `stun` 与 `holepunch` 模块。
2. `signald` 可转发候选消息。
3. 打洞成功后的链路锁定逻辑。

验收标准：

1. 在常见锥型 NAT 环境下可自动建立直连。
2. 在 IPv6 可达环境下优先命中 IPv6 路径。

### Phase 4: Windows 与中继回退（第 11-13 周）

目标：

1. 完成 Windows 客户端最小接入。
2. 为复杂 NAT 场景增加中继兜底。

交付物：

1. `wintun-windows` 模块。
2. `relayd` 最小中继服务。
3. 客户端回退策略与恢复逻辑。

验收标准：

1. Linux 与 Windows 可互联。
2. 无法打洞时能自动切到中继模式。

### Phase 5: 稳定性、测试与发布（第 14-16 周）

目标：

1. 完善测试、部署、日志和基础可观测。
2. 输出首版可安装交付物。

交付物：

1. Docker Compose 部署方案。
2. e2e 测试集。
3. 首版发布包与运维文档。

验收标准：

1. 支持最小生产环境部署。
2. 核心链路具备问题定位能力。

## 10. 里程碑定义

### M1: 服务端可注册

节点启动后，可完成注册、地址分配和配置拉取。

### M2: Linux 静态建链

两台 Linux 节点在人工指定 Endpoint 下稳定互通。

### M3: 自动打洞成功

两台位于 NAT 后的节点在无需人工干预的情况下自动建立直连。

### M4: 跨平台互联

Linux 与 Windows 节点通过同一控制面互联。

### M5: 回退与恢复

直连失败时可自动中继，网络变化后可恢复直连。

## 11. 验收指标

### 11.1 功能指标

1. 节点注册成功率 >= 99%。
2. 常见 NAT 环境直连成功率达到可接受水平。
3. 中继回退成功率 >= 99.9%。

### 11.2 性能指标

1. 同地域直连建链时间目标 < 3 秒。
2. 回退中继切换时间目标 < 15 秒。
3. 控制面支持至少数千在线节点的长连接维护。

### 11.3 稳定性指标

1. 客户端异常退出后可自动恢复。
2. 网络抖动、IP 变化、休眠恢复后可重新探测建链。
3. 服务端重启后客户端可自动重连并恢复状态。

## 12. 风险与决策点

### 12.1 主要风险

1. 双边对称型 NAT 下直连成功率天然受限。
2. Windows 用户态实现的性能与稳定性需要单独验证。
3. 不同家庭路由器、运营商网络的行为差异较大。
4. IPv6 可用不等于 IPv6 可入站，仍需 pinholing 逻辑。

### 12.2 关键决策

1. 是否在 MVP 就引入 Relay，还是在第二阶段后补齐。
2. Windows 首版是否支持完整数据面，还是先提供实验性版本。
3. 地址池是否固定使用 `100.64.0.0/10`，以及 IPv6 Overlay 规划方式。
4. 控制面鉴权是先用预共享令牌，还是一开始就接入 OIDC。

## 13. 建议的首批开发顺序

如果按最小可交付优先，推荐按下面顺序开工：

1. `proto/` 协议定义。
2. `server/internal/ipam` 与 `server/internal/device`。
3. `server/internal/signal`。
4. `client/crates/api-client`。
5. `client/crates/wg-manager` Linux 版。
6. `client/crates/stun`。
7. `client/crates/holepunch`。
8. `tests/e2e` 和 `deploy/docker-compose`。

## 14. 文档输出结论

本项目适合按“控制面先行、Linux 数据面优先、打洞能力逐步增强、Windows 与中继后补”的策略推进。这样可以先交付一个能运行、能互通、能验证路径选择的 MVP，再逐步补上复杂网络适配与生产能力，减少初期架构过重和实现失控的风险。
