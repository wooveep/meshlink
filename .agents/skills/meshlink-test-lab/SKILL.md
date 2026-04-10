---
name: meshlink-test-lab
description: Use when building or validating the MeshLink VM test lab, repeatable acceptance scenarios, smoke tests, libvirt topologies, or future NAT and relay experiments.
---

# MeshLink Test Lab

Use this skill when the task is about VM topology, reproducible validation, smoke tests, or networking experiments for MeshLink.

## First Read

1. `/home/cloudyi/code/meshlink/docs/tasks/phase-04-vm-lab-and-tests.md`
2. `/home/cloudyi/code/meshlink/docs/tasks/progress.md`
3. `references/vm-topology.md`
4. `references/acceptance-scenarios.md`

If the task touches runnable scripts, also read:

1. `references/command-recipes.md`
2. `references/vm-lab-usage.md`

## Working Rules

1. Prefer repeatable local validation over one-off manual experiments.
2. Keep acceptance scenarios phase-specific.
3. Use QEMU/libvirt as the default lab platform.
4. Do not mix NAT traversal experiments into phase 01-03 smoke tests.
5. When the task is to bring up the current repo VM lab, prefer the validated `noble-server-cloudimg` workflow in `references/vm-lab-usage.md` instead of improvising a new cloud-init path.

## Deliverables

A complete test-lab task should leave behind:

1. A documented topology or scenario
2. Repeatable commands or scripts
3. Clear pass/fail checks
