# Execution Plan

## Current Status

1. 已完成：协议骨架、仓库脚手架、任务文档与 AI 协作边界。
2. 已完成：设备注册、Overlay 地址分配、最小配置同步闭环。
3. 已完成：`SyncConfig` 双客户端发现与客户端 peer cache。
4. 已完成：Phase 01/02 的本地 smoke 与最小 libvirt VM 实验室骨架。
5. 下一开发重点：Linux 静态 WireGuard 建链与 VM lab 的 Phase 03 扩展。

## Phase 0

1. 已完成：冻结协议骨架。
2. 已完成：完成仓库脚手架。
3. 已完成：明确首批模块负责人和验收标准。

## Phase 1

1. 已完成：实现注册与地址分配。
2. 已完成：建立最小配置同步闭环。
3. 进行中：跑通 Linux 静态 WireGuard 建链。

## Phase 2

1. 待开始：引入 STUN 探测。
2. 待开始：完成候选地址交换与打洞竞速。
3. 待开始：验证 IPv4/IPv6/LAN 三类路径选择。

## Phase 3

1. 待开始：完成 Windows 客户端最小接入。
2. 待开始：增加 Relay 回退。
3. 已启动基础骨架：建立 e2e 与 NAT 实验环境。

## Phase 4

1. 待开始：完善 ACL、观测、部署和升级流程。
2. 待开始：优化异常恢复与路径切换稳定性。
