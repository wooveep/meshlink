# Task Files

## Source of truth

1. `docs/tasks/index.yaml` is the machine-readable task and state source.
2. `docs/tasks/phase-*.md` files are the human-readable task descriptions.
3. `docs/tasks/progress.md` is the short current-status summary.

## Update rules

1. Add a task to the relevant phase document and to `index.yaml` in the same change.
2. When a task status changes, update `index.yaml` first.
3. When a phase goal changes, update the corresponding phase document and this file if workflow changed.
