---
id: AD-9
title: "Story Leases Instead of Access Control"
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
decision: "Scope is enforced by leases. qdev claim story <id> records a lease and issues a session token via QDEV_SESSION."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Governance, multi-agent concurrency, worktree orchestration."
prevents:
  - "Two agents claiming one story, silent cross-team edits, and a spoofable RBAC that provides false assurance."
---

# AD-9: Story Leases Instead of Access Control

## Context
Authoritative architectural decision for qdev substrate.

## Decision
Scope is enforced by leases. qdev claim story <id> records a lease and issues a session token via QDEV_SESSION.

* **Rule**: Scope is enforced by leases. qdev claim story <id> records a lease and issues a session token via QDEV_SESSION.
* **Binds**: Governance, multi-agent concurrency, worktree orchestration.
* **Prevents**: Two agents claiming one story, silent cross-team edits, and a spoofable RBAC that provides false assurance.
