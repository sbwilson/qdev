---
id: AD-3
title: "Markdown Source of Truth, SQLite as Rebuildable Cache"
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
decision: "The source of truth is one Markdown file with YAML frontmatter per entity, committed to Git. SQLite is exclusively a local, gitignored, rebuildable index."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Storage architecture, Git strategy, and every read and write path."
prevents:
  - "Merge-conflict-prone monolithic ledgers, binary diffs, and lock-in."
---

# AD-3: Markdown Source of Truth, SQLite as Rebuildable Cache

## Context
Authoritative architectural decision for qdev substrate.

## Decision
The source of truth is one Markdown file with YAML frontmatter per entity, committed to Git. SQLite is exclusively a local, gitignored, rebuildable index.

* **Rule**: The source of truth is one Markdown file with YAML frontmatter per entity, committed to Git. SQLite is exclusively a local, gitignored, rebuildable index.
* **Binds**: Storage architecture, Git strategy, and every read and write path.
* **Prevents**: Merge-conflict-prone monolithic ledgers, binary diffs, and lock-in.
