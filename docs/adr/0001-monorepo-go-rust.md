# ADR 0001: Monorepo + Go/Rust Split

## Status

Accepted

## Context

项目同时包含控制面、信令、中继、客户端、协议定义和 AI 协作文档。协议和实现高度耦合，如果分散在多个仓库，初期演进成本高且容易漂移。

## Decision

1. 使用 Monorepo 管理服务端、客户端、协议和部署文件。
2. 服务端使用 Go。
3. 客户端使用 Rust。
4. Protobuf 作为跨组件契约单一事实来源。

## Consequences

1. 优点是协议和代码可以同步演进，变更可追踪。
2. 缺点是工具链更多，需要在 CI 中分别处理 Go 和 Rust。
3. 后续需要严格维护目录边界，避免 Monorepo 内部耦合失控。
