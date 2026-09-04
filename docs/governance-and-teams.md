# qdev Team Governance & Release Management

> Multi-owner entity models, cross-team boundary governance interlocks, and orthogonal SemVer release baselining.

## Table of Contents

- [Multi-Owner Teams & Cross-Team Governance](#ownership-governance)
- [Orthogonal Sprints & SemVer Releases](#orthogonal-releases)

---

<a id="ownership-governance"></a>

## 9. Multi-Owner Teams & Cross-Team Governance

In complex systems, multiple specialized teams or roles work concurrently. `qdev` introduces an **Ownership and Team Governance Model** supporting multiple owners (persons or teams) to prevent accidental cross-boundary interference.

### Multi-Owner Representation

Owners of a Sprint, Epic, or Story can be defined as one or more persons, a team, or a combination:

```yaml
# Inside Story or Epic Frontmatter
owners:
  - "simon"
  - "amelia"
  - "team:core-platform"
```

### The Cross-Team Governance Guard

If a developer or AI agent operating under one team attempts to claim or modify an entity owned exclusively by another team:

> [!WARNING]
> **Cross-Team Governance Alert (Prompt Interlock)**
>
> ```text
> ⚠️  CROSS-TEAM GOVERNANCE INTERLOCK:
>   Target Entity: Epic S5E2 ("The Crux Shell")
>   Entity Owners: ["simon", "team:core-platform"]
>   Current User:  sally (Team: ui-shell)
>
> Modifying this architectural core entity crosses defined team boundaries.
> Select action:
>   [1] Proceed with Cross-Team Override (Logs justification in Decision Ledger)
>   [2] Reassign Entity Ownership to include current user/team
>   [3] Abandon modification
> ```
>
> If option 1 is selected, `qdev` requires an explicit justification and permanently logs a `cross_team_override` record in the `decisions` table for auditability.

---

<a id="orthogonal-releases"></a>

## 10. Orthogonal Sprints & SemVer Releases

In `qdev`, **Functional Specifications** and **Temporal Execution Sprints** are orthogonal.

### The Functional Space (System Specification)

The continuous, evolving tree of product capabilities: PRDs, Epics, Stories, and ADRs. Stories represent architectural features that persist across time.

### The Temporal Space (Sprints & Baselines)

Execution intervals (e.g., Sprint 1, Sprint 2, ..., Sprint 5). A sprint schedules, implements, and verifies stories. Completed sprints are frozen as immutable snapshots.

### Mapping SemVer Releases to PRDs and Sprints

- **PRD v0.1.0 (Initial Platform):** Covers Sprints 1 through 27 → **Release v0.1.0 Baseline**.
- **PRD v0.2.0 (SfS Metrology Program):** Covers Sprints 28 through 53 → **Release v0.2.0 Baseline**.
- **PRD v0.1.1 (Emergency Patch Track):** Derived from v0.1.0; executed in **Sprint 54** to patch a critical issue without pulling in in-flight v0.2.0 work.

### External URI Referencing

```yaml
# Inside PRD or Story Frontmatter
external_uris:
  jira: "https://hospital-health.atlassian.net/browse/QB-1042"
  regulatory_dossier: "https://vault.internal/records/DHF-0042"
  prior_art_rfc: "https://github.com/rust-lang/rfcs/pull/1234"
```

---

