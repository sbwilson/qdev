---
id: AD-4
title: "WAL Concurrency & Atomic Transactions"
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
decision: "SQLite connections use WAL mode with busy_timeout = 5000ms. All cache mutations are short atomic transactions."
context: "Authoritative architectural decision for qdev substrate."
binds:
  - "Connection initialisation, transaction lifecycle, file write path."
prevents:
  - "Corruption when an agent and a human run qdev simultaneously."
---

# AD-4: WAL Concurrency & Atomic Transactions

## Context
Authoritative architectural decision for qdev substrate.

## Decision
SQLite connections use WAL mode with busy_timeout = 5000ms. All cache mutations are short atomic transactions.

* **Rule**: SQLite connections use WAL mode with busy_timeout = 5000ms. All cache mutations are short atomic transactions.
* **Binds**: Connection initialisation, transaction lifecycle, file write path.
* **Prevents**: Corruption when an agent and a human run qdev simultaneously.
