---
id: AD-2
title: "Synchronous SQLite via rusqlite"
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
decision: "Database interaction uses synchronous rusqlite with bundled SQLite. No async runtimes (tokio) or ORMs in core."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Database access layer."
prevents:
  - "Binary bloat and complexity from async overhead for short-lived CLI commands."
---

# AD-2: Synchronous SQLite via rusqlite

## Context
Authoritative architectural decision for qdev substrate.

## Decision
Database interaction uses synchronous rusqlite with bundled SQLite. No async runtimes (tokio) or ORMs in core.

* **Rule**: Database interaction uses synchronous rusqlite with bundled SQLite. No async runtimes (tokio) or ORMs in core.
* **Binds**: Database access layer.
* **Prevents**: Binary bloat and complexity from async overhead for short-lived CLI commands.
