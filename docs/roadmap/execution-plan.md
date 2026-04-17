# Execution Plan

## Current Status

1. 已完成：协议骨架、仓库脚手架、任务文档与 AI 协作边界。
2. 已完成：设备注册、Overlay 地址分配、最小配置同步闭环。
3. 已完成：`SyncConfig` 双客户端发现与客户端 peer cache。
4. 已完成：Linux 静态 WireGuard 建链、双 NAT 打洞、Relay fallback 与 direct-path recovery。
5. 已完成：Debian/Ubuntu `amd64` 的服务端与客户端 deb 产物流，以及 VM lab 的 package-first 验证链路。
6. 已完成：控制面内置 Hook 链、客户端静态路由发布，以及 `run-phase08-routes.sh` 路由回归脚本。
7. 已完成：客户端运行时从外部 WireGuard 工具迁移到嵌入式 Linux UAPI 和 Windows embedded tunnel-service 路径，并完成 dual-NAT 真实联调验收。
8. 当前重点：推进 ACL/策略过滤，以及 embedded runtime 的观测与长期稳定性。

## Phase 0

1. 已完成：冻结协议骨架。
2. 已完成：完成仓库脚手架。
3. 已完成：明确首批模块负责人和验收标准。
4. 已完成：建立二进制产物和 deb 打包链路。

## Phase 1

1. 已完成：实现注册与地址分配。
2. 已完成：建立最小配置同步闭环。
3. 已完成：跑通 Linux 静态 WireGuard 建链。

## Phase 2

1. 已完成：引入 STUN 探测。
2. 已完成：完成候选地址交换与打洞竞速。
3. 已完成：验证 dual-NAT 下的动态直连路径。

## Phase 3

1. 已完成：增加 Relay fallback 与 direct-path recovery。
2. 已完成：增加静态路由发布和路由撤销回归。
3. 已完成：Windows 最小接入过渡路径和 package-first 验证基线。

## Phase 4

1. 已完成：完成 Linux embedded WireGuard 控制集成，移除 `wg(8)` 运行时依赖。
2. 已完成：完成 Windows embedded tunnel-service runtime 集成与版本钉住。
3. 已完成：完成 Linux↔Linux、Linux↔Windows 的 embedded-runtime 回归验收。
4. 已完成：把 `INCREMENTAL` 事件升级为真正的 peer patch 模型。
5. 待开始：完善 ACL、策略过滤和更细粒度的 route distribution。
6. 待开始：完善观测、升级和长期运行稳定性。
