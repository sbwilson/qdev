---
id: AD-1
title: "Strict Two-Crate Workspace"
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
decision: "The workspace is split into qdev-cli and qdev-core. qdev-core never writes to stdout or reads stdin."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Project layout and dependency tree."
prevents:
  - "CLI formatting and I/O side effects from leaking into testable, headless core routines."
---

# AD-1: Strict Two-Crate Workspace

## Context
Authoritative architectural decision for qdev substrate.

## Decision
The workspace is split into qdev-cli and qdev-core. qdev-core never writes to stdout or reads stdin.

* **Rule**: The workspace is split into qdev-cli and qdev-core. qdev-core never writes to stdout or reads stdin.
* **Binds**: Project layout and dependency tree.
* **Prevents**: CLI formatting and I/O side effects from leaking into testable, headless core routines.
