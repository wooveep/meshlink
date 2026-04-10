---
name: meshlink-sdwan-dev
description: Use when working on the MeshLink WireGuard SD-WAN project to implement features, evolve protocols, update architecture docs, or coordinate AI-assisted development tasks across the server, client, proto, and design documents.
---

# MeshLink SD-WAN Dev

Use this skill when the task touches MeshLink code, architecture, protocols, roadmap, or AI collaboration artifacts.

## First Read

Before editing, read only the files needed for the task:

1. Project scope and priorities: `/home/cloudyi/code/meshlink/WireGuard SD-WAN 项目规划.md`
2. Doc map and task routing: `references/doc-map.md`
3. Delivery rules: `references/delivery-rules.md`
4. Phase map: `references/phase-map.md`

If the task changes protocols, also read:

1. `/home/cloudyi/code/meshlink/proto/*.proto`
2. `/home/cloudyi/code/meshlink/docs/api/service-contracts.md`
3. `references/protocol-checklist.md`

If the task changes architecture or module ownership, also read:

1. `/home/cloudyi/code/meshlink/docs/architecture/system-overview.md`
2. `/home/cloudyi/code/meshlink/docs/ai/module-boundaries.md`

If the task changes project status, sequencing, or backlog, also read:

1. `/home/cloudyi/code/meshlink/docs/tasks/index.yaml`
2. `/home/cloudyi/code/meshlink/docs/tasks/progress.md`
3. `references/task-files.md`

## Working Rules

1. Keep control plane and data plane boundaries explicit.
2. Prefer Linux-first implementations unless the task is Windows-specific.
3. Update docs when code changes affect contracts, workflow, or ownership.
4. Do not make Relay the default path; Relay is fallback only.
5. Keep protocol changes backward compatible unless the task explicitly allows breaking changes.

## Task Routing

For protocol or API tasks:

1. Update `proto/`
2. Update `docs/api/service-contracts.md`
3. Update any affected architecture docs

For server tasks:

1. Keep logic inside the correct `server/internal/*` module
2. Avoid crossing module boundaries through hidden shared state
3. Add or update validation paths

For client tasks:

1. Put platform-neutral logic in `agent-core`, `holepunch`, `stun`, `wg-manager`
2. Keep platform code isolated in `netlink-linux` and `wintun-windows`
3. Reflect any path-selection changes in docs

## Deliverable Standard

A complete task should leave behind:

1. Code or document changes
2. Updated design context when boundaries changed
3. A clear verification note

Do not create extra narrative docs unless they materially help future implementation.
