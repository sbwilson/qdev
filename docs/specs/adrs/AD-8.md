---
id: AD-8
title: "Sprints Are Assignments, Not Ownership"
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
decision: "Epics and stories belong to the functional tree only. Sprint membership is a many-to-one assignment recorded in the sprint file."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Sprint model, carry-over, release baselining."
prevents:
  - "Story renames and epic pinning when work spans sprints."
---

# AD-8: Sprints Are Assignments, Not Ownership

## Context
Authoritative architectural decision for qdev substrate.

## Decision
Epics and stories belong to the functional tree only. Sprint membership is a many-to-one assignment recorded in the sprint file.

* **Rule**: Epics and stories belong to the functional tree only. Sprint membership is a many-to-one assignment recorded in the sprint file.
* **Binds**: Sprint model, carry-over, release baselining.
* **Prevents**: Story renames and epic pinning when work spans sprints.
