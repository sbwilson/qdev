# qdev Documentation Index

Technical documentation for **qdev**, the relational workflow engine and gatekeeper for high-integrity software development.

## Authority

- The [Architecture Spine](bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md) is the authoritative list of architectural decisions (AD-1 … AD-14).
- The [PRD](bmad/planning-artifacts/prd-1/prd.md) is the authoritative requirements list.
- The [Epics & Stories](bmad/planning-artifacts/epics.md) are the implementation plan.
- The guides below elaborate on those; where they conflict, the spine and PRD win.

## Guides

- **[Planning Pipeline](planning-pipeline.md)** — entry modes (greenfield, document, brownfield, increment), the eight-stage pipeline with readiness checks, version-scoped planning and sprint cutting, elicitation engine, worked examples, feedback loops. *Draft; §11 lists the changes it implies for the other documents.*
- **[Architecture & Data Model](architecture.md)** — problem statement, methodology, entity catalog, ID grammar, state machine, relations, on-disk layout, cache schema, hydration, context projection.
- **[CLI Reference & Configuration](cli-reference.md)** — grammar, command catalog, JSON envelope and exit codes, non-interactive mode, pulse, `qdev.toml` and `.qdev.local.toml`, install and doctor.
- **[Quality, Gates & Compliance Support](compliance-and-safety.md)** — citations and hygiene linter, gate engine and result contract, ratchets, evidence bundles, IEC 62304 / ISO 14971 support mapping, preflight and hooks.
- **[Governance & Release Management](governance-and-teams.md)** — ownership, leases, overrides, sprints as assignments, carry-over, releases.
- **[v1 Scope & Roadmap](roadmap.md)** — scope, execution plan by epic, self-hosting handover, edge cases.

## Review history

- [Specification Review, 2026-09-06](bmad/planning-artifacts/spec-review.md) — findings that produced the current revision of every document above.

## Historical

- [Initial Planning HTML](initial_planning.html) — the original design document, kept unmodified for reference. It predates the spine and is superseded wherever they differ (IDs, states, paths, gate model, RBAC).
