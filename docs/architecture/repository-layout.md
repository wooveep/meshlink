# Repository Layout

## 结构目标

Monorepo 用于统一管理协议、服务端、客户端、部署和文档，降低跨仓库协议演进的沟通成本。

## 目录说明

1. `proto/`
   所有跨组件协议的单一事实来源。
2. `server/`
   Go 控制面与中继服务。
3. `client/`
   Rust 客户端工作区与平台实现。
4. `docs/`
   设计、路线图、AI 协作文档。
5. `deploy/`
   本地开发与自托管部署模板。
6. `tests/`
   集成测试、NAT 实验和 e2e 场景。
7. `skills/`
   为 AI 辅助开发准备的项目专用技能。

## 约束

1. 协议变更先修改 `proto/` 和 `docs/api/`。
2. 跨服务共享逻辑优先放在 `server/internal` 的明确模块里，不做隐式耦合。
3. 客户端平台相关能力必须在独立 crate 中维护。
