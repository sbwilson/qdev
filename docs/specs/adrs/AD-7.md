---
id: AD-7
title: "Sprint-Free, Collision-Resistant Identifiers"
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
decision: "Planning entities use sequential human-readable IDs that never encode a sprint: epics En, stories EnSm, ADRs AD-n, requirements FR-n / NFR-n, hazards HAZ-n, PRDs PRD-n."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "ID grammar, file naming, citation format."
prevents:
  - "Renaming on sprint carry-over and silent ID collisions across branches and worktrees."
---

# AD-7: Sprint-Free, Collision-Resistant Identifiers

## Context
Authoritative architectural decision for qdev substrate.

## Decision
Planning entities use sequential human-readable IDs that never encode a sprint: epics En, stories EnSm, ADRs AD-n, requirements FR-n / NFR-n, hazards HAZ-n, PRDs PRD-n.

* **Rule**: Planning entities use sequential human-readable IDs that never encode a sprint: epics En, stories EnSm, ADRs AD-n, requirements FR-n / NFR-n, hazards HAZ-n, PRDs PRD-n.
* **Binds**: ID grammar, file naming, citation format.
* **Prevents**: Renaming on sprint carry-over and silent ID collisions across branches and worktrees.
