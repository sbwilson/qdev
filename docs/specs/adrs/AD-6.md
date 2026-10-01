---
id: AD-6
title: "Directory Metadata Sweep Hydration"
status: accepted
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
owners:
  - "simon"
  - "team:core"
decision: "On every CLI boot, qdev sweeps the mtime and size of all entity files and re-parses only those whose metadata changed since last sync."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Boot sequence and file system layer."
prevents:
  - "Slow boots, stale reads after git checkout, and mtime illusion bugs on coarse filesystems."
---

# AD-6: Directory Metadata Sweep Hydration

## Context
Authoritative architectural decision for qdev substrate.

## Decision
On every CLI boot, qdev sweeps the mtime and size of all entity files and re-parses only those whose metadata changed since last sync.

* **Rule**: On every CLI boot, qdev sweeps the mtime and size of all entity files and re-parses only those whose metadata changed since last sync.
* **Binds**: Boot sequence and file system layer.
* **Prevents**: Slow boots, stale reads after git checkout, and mtime illusion bugs on coarse filesystems.
