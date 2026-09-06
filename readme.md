# qdev

> **Relational Workflow Engine, Gatekeeper, and Context Optimizer for High-Integrity Software Development**

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)
[![Architecture](https://img.shields.io/badge/architecture-BMAD%20%2B%20Shape%20Up-blue.svg)](docs/architecture.md)
[![Compliance](https://img.shields.io/badge/supports-IEC%2062304%20%2F%20ISO%2014971-orange.svg)](docs/compliance-and-safety.md)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

---

`qdev` is an offline-first, single-binary Rust development engine and gatekeeper for complex software projects and AI pair-programming workflows.

It combines **BMAD hierarchical decomposition** (PRDs → Requirements → Epics → Stories, with ADRs and Hazards) with **Shape Up boundaries** (appetite, target modules, rabbit holes, no-gos) to address the failure modes of AI-assisted engineering at scale: markdown context sprawl, syntax drift, line-anchor rot, and memoir comments in production code.

Specifications live as one Markdown file per entity in Git. A local, gitignored SQLite cache hydrates on every command in tens of milliseconds and serves as the relational index. Delete the cache at any time; it rebuilds from Git.

---

## At a glance

```
$ qdev
qdev 1.0 — Development Engine & Gatekeeper
Environment
  Working tree   clean (feature/E12S4-buffer @ 8f1b2c4)
  Integration    develop is up to date with origin/develop
  Cache          healthy (synced 18 ms ago, 184 entities, 0 findings)
  Lease          E12S4 held by simon since 09:41

Sprint 5 — The Rust Core Port  [release 0.1.0]
  Stories        42 done / 12 in progress / 3 blocked / 108 backlog
  Deferred work  8 open (0 unacceptable)
  Gates          14/14 passing, ratchets at baseline

Next
  E12S4 "CoreResponse Buffer Layout" is ready and unblocked.
  Run: /qdev-develop E12S4
```

---

## The problem

| Failure mode | What happens | What qdev does |
| --- | --- | --- |
| **Context sprawl** | Agent files balloon to thousands of words; agents burn 20,000+ tokens to find three acceptance criteria | `qdev context <id> --budget N` produces a bounded, deterministic projection |
| **Syntax drift** | Flat lists mix checkboxes, bullets, and prose; automation goes blind | One typed entity per file, validated on every hydration |
| **Line-anchor rot** | Specs cite `file.md:2340`; edits silently rot the anchors | Immutable IDs that never encode a sprint: `E12S4`, `AD-43`, `DW-7f3a` |
| **Memoir comments** | Agents write review essays into source | Compact citations `// [E12S4]`; a linter reports the rest |

---

## Core concepts

- **Positive structure** tells the agent where it is: PRD, requirements, epics, stories, ADRs.
- **Negative boundaries** tell it where it may not go: appetite, target modules, rabbit holes, no-gos, each with an ID so a rejection can cite it.
- **Leases** scope an agent or developer to one story; anything outside needs a logged override.
- **Gates** are your own scripts. qdev runs them with timeouts, classifies failures as logical or infrastructure, and writes committed evidence.
- **Sprints are assignments.** Stories carry over without renaming.

---

## Key features

- **Token-budgeted context.** `qdev context E12S4 --phase develop --budget 1200 --stats`.
- **Deterministic "what next".** `qdev next --json` for orchestrators; the root command for humans.
- **Attributed rejections.** Every refusal cites the constraint, gate, or policy ID and its text.
- **Zero-noise gate receipts.** One line on pass; the failing assertion on fail; "halt and alert" on infrastructure failure.
- **Evidence and traceability.** Requirement → story → gate run → evidence file, exportable as a matrix at sprint close.
- **Compliance support.** Deferred work carries ISO 14971 risk levels; SOUP audit and SBOM wrappers; residual anomaly report. Designed to support IEC 62304 processes, not to certify them.
- **Agent integration.** Generated skills for Claude Code, Cursor, and agent directories; `qdev mcp serve`.
- **Cross-platform, offline, zero telemetry.** macOS, Linux, Windows; no runtime; no network calls you did not configure.

---

## Quick start

```bash
cargo install --path tools/qdev --locked
qdev init --name MyProject --developer me --team core
qdev install skills --claude
qdev install hooks
qdev doctor
```

```bash
qdev create epic --title "Video ingest"
qdev create story E1 --title "Frame ring buffer" --appetite small --module engine
qdev constraint add E1S1 --kind no_go -- "Do not introduce an async runtime"
qdev claim story E1S1
qdev context E1S1 --phase develop --budget 1200
qdev gate run --for-transition review
qdev transition story E1S1 review
```

---

## Configuration

Committed `qdev.toml` holds project policy: teams, Git rules, module registry with paths, gates, hygiene rules, regulatory settings. Gitignored `.qdev.local.toml` holds identity and preferences. See the [CLI reference](docs/cli-reference.md#configuration).

---

## Documentation

| Guide | Contents |
| --- | --- |
| [Architecture & Data Model](docs/architecture.md) | Methodology, entity catalog, IDs, state machine, relations, layout, cache schema, projection |
| [CLI Reference & Configuration](docs/cli-reference.md) | Grammar, command catalog, JSON envelope, exit codes, config, install |
| [Quality, Gates & Compliance Support](docs/compliance-and-safety.md) | Hygiene, gate engine, ratchets, evidence, compliance mapping, preflight |
| [Governance & Releases](docs/governance-and-teams.md) | Ownership, leases, overrides, sprints, carry-over, releases |
| [Scope & Roadmap](docs/roadmap.md) | v1 scope, plan by epic, handover, edge cases |
| [Architecture Spine](docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md) | Authoritative architectural decisions |

---

## Roadmap

- [ ] **E1** Relational storage & CLI substrate
- [ ] **E2** Workflow & governance engine
- [ ] **E3** Gates, evidence & Git
- [ ] **E4** AI context & integration
- [ ] Self-hosting handover: finish building qdev with qdev

---

## License

Dual-licensed under Apache-2.0 or MIT at your option.
