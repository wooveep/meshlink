# MeshLink

MeshLink 是一个基于 WireGuard 的分布式 SD-WAN 组网项目，目标是提供控制面与数据面分离的安全 Overlay 网络，并尽可能在复杂 NAT 与 IPv6 环境下实现自动直连。

当前仓库已经从“可开工骨架”推进到“Linux Phase 06 完整闭环 + 静态路由发布已落地 + Embedded WireGuard Runtime 进行中”阶段，重点完成了以下内容：

1. Monorepo 目录结构
2. 基于 gRPC 的 Phase 01 最小控制链路
3. 基于 `SyncConfig` 的双客户端 peer discovery
4. Rust 客户端 peer cache 与结构化 peer 变更日志
5. Protobuf 协议与生成脚本
6. 架构、任务、AI 协作、路线图与 ADR 文档
7. 项目专用开发 Skill 与测试实验室 Skill
8. Phase 01/02 的本地 smoke 与最小 libvirt VM lab 骨架
9. 基于协议下发静态 endpoint 的 Linux WireGuard 直连与 VM lab overlay 验收
10. 双 NAT 打洞、Relay fallback 与 direct-path recovery
11. 控制面内置 Hook 链与客户端静态路由发布
12. 面向 Debian/Ubuntu `amd64` 的服务端和客户端 deb 打包链路
13. 面向 Windows 客户端的 zip 打包脚本、嵌入式 tunnel-service 入口，以及基于 libvirt 的手工验证路径

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
make build-server
make build-client
make package-deb
make package-windows
make test
make smoke
make smoke-phase02
make vm-lab
make vm-lab-phase03
make vm-lab-phase03-deb
make vm-lab-phase08
make fmt
make lint
```

## 当前已跑通的主要闭环

1. `managementd` 监听管理端口
2. `meshlinkd` 读取本地配置并注册
3. 服务端分配唯一 Overlay IPv4
4. 客户端建立 `SyncConfig` 长连接并接收配置事件
5. 两个客户端可通过 `SyncConfig` 发现彼此
6. 本地 smoke 可覆盖 Phase 01 与 Phase 02
7. VM lab 可覆盖 Phase 03 的静态 WireGuard 直连与 overlay ping
8. Dual-NAT VM lab 可覆盖 Phase 05 的打洞直连与 Phase 06 的 Relay fallback/recovery
9. 控制面可分发 peer 发布的静态 routed subnet，并通过 `AllowedIPs` 下发到对端

## Debian Packages

当前仓库可输出以下 Debian 安装包到 `dist/deb/`：

1. `meshlink-managementd`
2. `meshlink-signald`
3. `meshlink-relayd`
4. `meshlink-client`

构建命令：

```bash
make build-server
make build-client
make package-deb
```

包内安装约定：

1. 服务端二进制安装到 `/usr/bin/`
2. 服务端默认环境文件安装到 `/etc/default/meshlink-*.env`
3. 客户端主配置安装到 `/etc/meshlink/client.toml`
4. 示例文件安装到 `/usr/share/doc/meshlink-*/examples/`
5. 仅 `meshlink-managementd` 和 `meshlink-client` 提供 systemd unit，且安装后默认不启用
6. `meshlink-client` 仍要求目标 Linux 节点具备内核 WireGuard 支持和 `iproute2`

`make package-deb` 会优先使用本机 `nfpm`，若未安装则回退到：

```bash
go run github.com/goreleaser/nfpm/v2/cmd/nfpm@v2.41.2
```

安装与检查示例：

```bash
sudo dpkg -i dist/deb/meshlink-managementd_0.1.0-1_amd64.deb
sudo dpkg -i dist/deb/meshlink-client_0.1.0-1_amd64.deb
dpkg-deb -I dist/deb/meshlink-managementd_0.1.0-1_amd64.deb
dpkg-deb -c dist/deb/meshlink-client_0.1.0-1_amd64.deb
systemctl status meshlink-managementd.service
systemctl status meshlink-client.service
```

## Windows Package

当前仓库可输出 Windows 客户端 zip 包到 `dist/windows/`，包内包含：

1. `meshlinkd.exe`
2. `client.toml`
3. `run-meshlinkd.ps1`
4. `README.txt`
5. `tunnel.dll`
6. `wireguard.dll`
7. `wintun.dll`

构建命令：

```bash
make package-windows
```

默认目标是 `x86_64-pc-windows-gnu`。如果当前 Linux 宿主机没有 Windows 交叉编译工具链，也可以先在其他环境生成 `meshlinkd.exe`，再复用仓库里的打包逻辑：

```bash
MESHLINK_WINDOWS_BINARY=/absolute/path/to/meshlinkd.exe make package-windows
```

Windows 包现在还要求预先把固定版本的 runtime DLL 放到：

```text
deploy/packages/windows/runtime/v0.3.17/amd64/
```

默认可以直接执行：

```bash
make windows-runtime
```

这会在 Linux 宿主机上：

1. 从 `wireguard-windows` `v0.3.17` 源码交叉构建 `tunnel.dll`
2. 从官方 `wireguard-nt` SDK 下载 `wireguard.dll`
3. 从官方 `wintun` release 下载 `wintun.dll`
4. 把它们和版本 manifest 一起 stage 到固定目录

如果你更希望在 Windows 构建机上产出 runtime，仍然可以使用
`scripts/build-wireguard-windows-runtime.ps1`。

## VM Lab

最小 libvirt 实验室资产位于 `tests/nat-lab/`，当前覆盖：

1. `mgmt-1`
2. `client-a`
3. `client-b`
4. `nat-a`
5. `nat-b`

使用前先复制 `tests/nat-lab/libvirt.env.example` 为 `tests/nat-lab/libvirt.env`，填入本机 cloud image 路径，然后执行：

```bash
./tests/nat-lab/create-lab.sh
./tests/nat-lab/run-phase01-02.sh
./tests/nat-lab/run-phase03.sh
./tests/nat-lab/run-phase03-deb.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase06.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh
./tests/nat-lab/destroy-lab.sh
```

其中：

1. `run-phase03.sh` 继续验证“宿主机构建二进制后拷入 VM”的基线场景
2. `run-phase03-deb.sh` 验证“宿主机构建 deb 后在 VM 内安装并通过 systemd 启动”的 package-first 场景
3. `run-phase05.sh` 验证 dual-NAT 打洞直连
4. `run-phase06.sh` 验证 relay fallback 与 direct-path recovery
5. `run-phase08-routes.sh` 验证静态路由发布、Relay 下的 routed traffic 连通性，以及撤销后的路由清理

## Windows VM Validation

Windows 的真实验证路径位于 `tests/windows-vm/`，推荐方式是：

1. 先起 `MESHLINK_LAB_TOPOLOGY=dual-nat` 的 Linux 实验室
2. `make package-windows`
3. 用 `./tests/windows-vm/create-vm.sh` 起一个额外挂到 `nat-a-lan` 或 `nat-b-lan` 的 Windows VM
4. 用 `./tests/windows-vm/prepare-dual-nat.sh map` 补 NAT 端口映射
5. 在 Windows 里运行包内的 `run-meshlinkd.ps1`
6. 如果安装了 `qemu-ga`，可以用 `./tests/windows-vm/qga.sh` 从宿主机执行 PowerShell 和读取配置文件

这条路径现在是 Embedded Runtime 阶段的标准 Windows 验收入口，覆盖
Windows 对 Linux 的直连、relay fallback、direct recovery、路由发布和
路由撤销验证；脚本也会在 dual-NAT 实验室里自动避开与同侧 Linux client
冲突的 Windows WireGuard 监听端口。

如果要覆盖当前 Linux 主交付链路，推荐执行：

```bash
make package-deb
make vm-lab-phase03-deb
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase06.sh
MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh
```

## 路由发布

客户端可在配置中声明：

```toml
advertised_routes = ["10.20.0.0/24"]
```

控制面会在注册时校验这些静态 IPv4 CIDR，并把它们并入该 peer 的 `AllowedIPs`。首版约束：

1. 仅支持静态 IPv4 CIDR
2. 不允许 `0.0.0.0/0`
3. 不允许与 overlay CIDR 重叠
4. 不允许与其他设备已发布路由重叠

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

安装包方式运行客户端：

```bash
/usr/bin/meshlinkd --config /etc/meshlink/client.toml
```
