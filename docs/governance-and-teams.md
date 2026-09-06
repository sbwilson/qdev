# qdev Team Governance & Release Management

> Ownership, leases, cross-team overrides, sprints as assignments, and release baselines.

## Table of Contents

- [Ownership Model](#ownership)
- [Story Leases](#leases)
- [Cross-Team Overrides](#overrides)
- [Sprints, Carry-Over & Releases](#sprints)
- [External References](#external)

---

<a id="ownership"></a>

## 1. Ownership Model

Owners of an epic, story, or sprint are persons, teams, or both:

```yaml
owners:
  - "simon"
  - "team:core-platform"
```

Teams are declared in `qdev.toml`. The current user's identity and teams come from `.qdev.local.toml`, falling back to `git config user.email`. Identity is **advisory and audit-oriented**: it attributes actions, it does not authenticate them. qdev deliberately has no role-based access control (see **AD-9**).

---

<a id="leases"></a>

## 2. Story Leases

A lease is the unit of scope. Before mutating a story, its scratchpad, or code within its modules, a developer or agent claims it:

```
$ qdev claim story E12S4
✔ Lease E12S4 → simon (worktree /work/qubric, branch feature/E12S4-buffer)
export QDEV_SESSION=qs_E12S4_9f2c
```

- Leases live in `.qdev/leases/` (gitignored, per worktree) and are also visible to other worktrees of the same repository through the shared `.git` directory, so `qdev next` never offers a story that is leased elsewhere.
- Commands that mutate entity state check the lease: the target must be the leased story, a child of it (constraints, scratchpad, deferred work it originates), or a sprint-agnostic record like a decision.
- Mutating a parent epic, another story, an ADR, or a requirement from within a story lease requires `--override --justification "..."`, logged as a `lease_override` decision.
- `qdev release` ends the lease; `transition ... done` releases automatically. Stale leases are reported by `qdev doctor` and can be broken with `qdev release E12S4 --force --justification`.
- Leases carry the holder's `author_type`, so agent and human mutations are distinguishable in the audit trail.

This gives the intended protection ("an agent working a story cannot quietly rewrite the epic's requirements") without pretending to be an access-control system that a gitignored file could defeat.

---

<a id="overrides"></a>

## 3. Cross-Team Overrides

When the current user is not among an entity's owners and none of their teams are either, a mutation is a cross-team edit.

Interactive:

```
⚠ Cross-team edit
  Entity  E12 "The Crux Shell"   owners: simon, team:core-platform
  You     sally (team:ui-shell)
  [1] Override with justification (logged as DEC cross_team_override)
  [2] Add team:ui-shell to owners (requires an existing owner's lease or override)
  [3] Abort
```

Non-interactive (**AD-12**): the command exits 3 with `needs_confirmation` naming `--override --justification`. There is no silent path.

All overrides create a `DEC-` record with `decision_type = cross_team_override`, subject, author, and justification. `qdev review sprint` lists them.

---

<a id="sprints"></a>

## 4. Sprints, Carry-Over & Releases

Per **AD-7** and **AD-8**, the functional tree (PRD → Requirements → Epics → Stories, with ADRs and Hazards) is independent of time. Sprints schedule work; they do not own it.

### Sprint file

```yaml
# docs/state/sprints/sprint-5.md
---
id: 5
title: The Rust Core Port
status: active
release: 0.1.0
owners: ["team:core-platform"]
assignments:
  - story: E12S4
    assigned_at: 2026-08-30
  - story: E11S9
    assigned_at: 2026-08-30
    carried_from: 4
---
```

### Multiple active sprints

Any number of sprints may be `active`, for example a feature sprint and a patch sprint. Commands take `--sprint N`; the fallback is `default_sprint` in `qdev.toml`, then the single active sprint if there is exactly one.

### Closing a sprint

```
$ qdev sprint close 5 --status completed --carry-over 6
✔ Ran sprint gates (14/14)
✔ Residual anomaly report: 8 open DW (0 unacceptable) → docs/state/releases/0.1.0/anomalies.md
✔ Traceability matrix → docs/state/releases/0.1.0/rtm.md
✔ Baseline snapshot: 42 done, ratchet values recorded
✔ Carried 15 open stories to sprint 6 (no IDs changed)
```

`--status paused | abandoned` skip the baseline and record the reason as a decision. Closing is refused (exit 3) while any `unacceptable` deferred work is open without a rationale, or while any story in the sprint is `in-progress` with a live lease.

### Releases

Releases are semver records with a `base_version` for patch tracks:

- PRD-1 → sprints 1–27 → release 0.1.0
- PRD-2 → sprints 28–53 → release 0.2.0
- Patch track 0.1.1 derived from 0.1.0, executed in sprint 54 in parallel with 0.2.0 work

A release snapshot bundles the traceability matrix, anomaly report, SOUP inventory, baseline metrics, and the commit SHA.

---

<a id="external"></a>

## 5. External References

```yaml
external_uris:
  jira: "https://example.atlassian.net/browse/QB-1042"
  regulatory_dossier: "https://vault.internal/records/DHF-0042"
```

External URIs are stored and displayed, never fetched. Bidirectional sync with issue trackers is out of scope for v1.
