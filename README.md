# MeshLink

MeshLink 是一个基于 WireGuard 的分布式 SD-WAN 组网项目，目标是提供控制面与数据面分离的安全 Overlay 网络，并尽可能在复杂 NAT 与 IPv6 环境下实现自动直连。

当前仓库已经从“可开工骨架”推进到“Phase 03 可验证”阶段，重点完成了以下内容：

1. Monorepo 目录结构
2. 基于 gRPC 的 Phase 01 最小控制链路
3. 基于 `SyncConfig` 的双客户端 peer discovery
4. Rust 客户端 peer cache 与结构化 peer 变更日志
5. Protobuf 协议与生成脚本
6. 架构、任务、AI 协作、路线图与 ADR 文档
7. 项目专用开发 Skill 与测试实验室 Skill
8. Phase 01/02 的本地 smoke 与最小 libvirt VM lab 骨架
9. 基于协议下发静态 endpoint 的 Linux WireGuard 直连与 VM lab overlay 验收

## 仓库结构

```text
.
├── docs/                  设计与协作文档
├── docs/tasks/            阶段任务与进度索引
├── proto/                 gRPC/Protobuf 契约
├── server/                Go 控制面与中继服务
├── client/                Rust 客户端工作区
├── deploy/                部署样例
├── scripts/               开发脚本
├── tests/                 集成与 e2e 测试目录
├── skills/                项目专用 Skill
└── WireGuard SD-WAN 项目规划.md
```

## 开发原则

1. 先交付可验证的最小闭环，再补复杂特性。
2. 控制面与数据面边界清晰，避免客户端和服务端职责耦合。
3. 所有跨模块协议先写契约，再落代码。
4. 代码变更需要同步更新设计文档和 AI 协作文档。

## 推荐启动顺序

1. 阅读 [WireGuard SD-WAN 项目规划.md](./WireGuard%20SD-WAN%20项目规划.md)
2. 阅读 `docs/architecture/` 和 `docs/ai/`
3. 补全 `proto/` 契约
4. 先实现 `server/internal/ipam`、`server/internal/device`、`server/internal/signal`
5. 再推进 `client/crates/wg-manager`、`stun` 和 `holepunch`

## 常用命令

```bash
make proto
make server
make client
make test
make smoke
make smoke-phase02
make vm-lab
make vm-lab-phase03
make fmt
make lint
```

## 当前已跑通的最小闭环

1. `managementd` 监听管理端口
2. `meshlinkd` 读取本地配置并注册
3. 服务端分配唯一 Overlay IPv4
4. 客户端建立 `SyncConfig` 长连接并接收配置事件
5. 两个客户端可通过 `SyncConfig` 发现彼此
6. 本地 smoke 可覆盖 Phase 01 与 Phase 02
7. VM lab 可覆盖 Phase 03 的静态 WireGuard 直连与 overlay ping

## VM Lab

最小 libvirt 实验室资产位于 `tests/nat-lab/`，当前覆盖：

1. `mgmt-1`
2. `client-a`
3. `client-b`

使用前先复制 `tests/nat-lab/libvirt.env.example` 为 `tests/nat-lab/libvirt.env`，填入本机 cloud image 路径，然后执行：

```bash
./tests/nat-lab/create-lab.sh
./tests/nat-lab/run-phase01-02.sh
./tests/nat-lab/run-phase03.sh
./tests/nat-lab/destroy-lab.sh
```

## 示例

服务端：

```bash
cd server
go run ./cmd/managementd -listen 127.0.0.1:33073
```

客户端：

```bash
cargo run --manifest-path client/Cargo.toml --bin meshlinkd -- \
  --config deploy/examples/client-config.toml
```
