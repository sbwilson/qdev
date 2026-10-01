---
id: AD-5
title: "Sub-process Gates with a Result Contract"
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
decision: "Gates are external executables configured in qdev.toml and typically stored in .qdev/gates/. The binary ships no project-specific verifiers."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Gate execution model and the gate result schema."
prevents:
  - "Hardcoding Rust, Swift, or medical-specific checks into a generic binary."
---

# AD-5: Sub-process Gates with a Result Contract

## Context
Authoritative architectural decision for qdev substrate.

## Decision
Gates are external executables configured in qdev.toml and typically stored in .qdev/gates/. The binary ships no project-specific verifiers.

* **Rule**: Gates are external executables configured in qdev.toml and typically stored in .qdev/gates/. The binary ships no project-specific verifiers.
* **Binds**: Gate execution model and the gate result schema.
* **Prevents**: Hardcoding Rust, Swift, or medical-specific checks into a generic binary.
