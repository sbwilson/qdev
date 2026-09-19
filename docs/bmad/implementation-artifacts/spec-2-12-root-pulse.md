---
title: 'Story 2.12: The Root Pulse'
type: 'feature'
created: '2026-09-18'
status: 'in-progress'
route: 'dispatch'
review_loop_iteration: 0
story_key: '2-12-root-pulse'
baseline_commit: 'dbacc88afa3fef0d3b3dc801e894744e757729cf'
spec_file: 'docs/bmad/implementation-artifacts/spec-2-12-root-pulse.md'
context:
  - 'docs/bmad/implementation-artifacts/epic-2-context.md'
  - 'docs/cli-reference.md'
  - 'docs/bmad/implementation-artifacts/spec-2-11-qdev-next.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev` with no arguments currently prints only the version line (the `get_pulse_status` stub); a developer or agent has no single command to orient — working-tree state, integration sync, cache health, active lease, sprint progress, deferred-work debt, and the next action are scattered across `doctor`, `sprint`, `dw`, and `next`. Story 2.11's deterministic selection exists but nothing composes it into one screen.

**Approach:** Implement the default command (`qdev`, with `qdev status` kept as an alias) as a read-only workspace pulse built in a new `qdev-core` module (`pulse.rs`). It composes cache-native store reads (health, sprints, assignments, deferred work, gate runs), a bounded set of local `git` subprocess probes, this-worktree lease records, and `select_next` (Story 2.11, reused as-is) into one payload, rendered as the human layout in `docs/cli-reference.md` §5 or a versioned `pulse` JSON payload.

## Boundaries & Constraints

**Always:**
- Read-only: no cache row, entity file, lease file, or decision record is written; no advisory lock is taken (open the query store exactly as `next` does). Never mutates story or sprint state.
- Outside a workspace (no `qdev.toml`): exit 0 with a one-line hint ("not a qdev workspace — run `qdev init` first") in text, and a JSON payload with `workspace: false` and every other field `null`. The `requires_workspace` exemption for the default command/`Status` is preserved, including its pinned tests.
- Git probes are local and bounded — no network calls (`git fetch`/`pull`/`ls-remote` are forbidden): branch (`rev-parse --abbrev-ref HEAD`), short SHA (`rev-parse --short HEAD`), dirty count (`status --porcelain` line count), ahead/behind (`rev-list --left-right --count <integration>...<remote>/<integration>`). A non-git directory or failed probe degrades that field to `null` with a `unavailable`-style marker — a git failure never fails the command.
- Environment section: working tree (`clean`/`dirty (N files)` + `branch @ shortsha`), integration state relative to `config.git.remote` + `config.git.integration_branch` (`up_to_date` / `N behind` / `N ahead` / diverged; missing local refs → state that names them, no fetch), cache health from cache-native reads only — `schema_status` (ok/mismatch), entity count, `finding_count` (the `findings` table, as doctor's cache section reads it), sync age — and the current lease: leases held by **this worktree** only (canonical path comparison, the `next.rs` pattern), `lease: null` when none.
- Sprint section: one block per sprint with `status: active` and a live entity row (the live-only rule `next.rs` applies), sorted by sprint id. Story counters are the four fixed, mutually exclusive buckets (decided D-2): `done`; `in-progress` (status `in-progress`, not computed-blocked); `blocked` (not `done`, computed-blocked by the same `depends_on` live-`done` semantics as `next.rs`); `backlog` = everything else assigned. Buckets sum to the assigned-story count; stale rows are excluded everywhere (live-only reads).
- Deferred-work line: workspace-wide count of `status: open` DW plus how many of those carry `safety_risk: unacceptable`, rendered in each active sprint block (decided D-3); per-sprint detail stays `qdev dw list`.
- Gates line: rendered only when `gate_runs` rows exist — `N/M passing` from the most recent run per gate; otherwise omitted (text) / `null` (JSON). No ratchet clause while Epic 3 ratchets do not exist (decided D-4).
- Next section: `select_next` with default scope (no `--sprint`/`--owner`) and the resolved current identity, embedded verbatim. Text shows the reason summary and, for a selection, `Run: /qdev-develop <id>   (or: qdev context <id> --phase <phase>)` with phase mapping `draft→specify`, `ready`/`in-progress→develop` (decided D-5); `next: null` renders the summary plus the blocker lines, reusing the `render_next_text` shape.
- Header is `qdev <CARGO_PKG_VERSION> — Development Engine & Gatekeeper` (the real version, not the docs example's 1.0).
- Deterministic: output independent of cache-row or fixture order (sorted buckets, BTreeMap, sorted sprint/lease order). `--json` emits the versioned `pulse` payload via `payload-pulse.json` and `PayloadKind::Pulse`.
- Completes without stdin in non-interactive environments; the 100 ms + Git-status budget means cache reads plus bounded git probes only — no hydration, no file-tree scans.

**Never:**
- Never write or lock: no lease claim/release, no entity or cache mutation, no decision record, no file created outside test fixtures.
- Never invoke network git, `run_validation`, or a sweep — the pulse reads what the cache and local git refs already say; an unsynced workspace is reported as stale cache, not repaired.
- Don't build Epic 3 gates/ratchets, Epic 4 context projection, or Story 2.13 module registry; don't add entity kinds, globs, config keys, or store migrations.
- Don't change `qdev next` behaviour, the `requires_workspace` classification of any other command, or the pinned boot/guard tests beyond the documented additions.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|-------------|
| Full fixture | Git repo on a branch; active sprint 5 with done/in-progress/blocked/backlog stories; one lease held by this worktree; open DW incl. one unacceptable; no gate runs | Text matches §5 layout: Environment (tree, integration, cache, lease), sprint block with four counters + DW line, Next with both invocations; no Gates line | N/A |
| `--json` | Same fixture | Envelope with `workspace: true`, `environment`, `sprints`, `gates: null`, `next`; `next` byte-identical to `qdev next --json`'s selection; payload round-trips `payload-pulse.json` | N/A |
| No active sprints | All sprints closed/absent | Sprints section reports none active (text) / empty array (JSON); `next: null` with `no_active_sprints`; exit 0 | N/A |
| Outside workspace | No `qdev.toml` | Exit 0; one-line init hint (text) / `workspace: false`, all else null (JSON); no cache file created | N/A |
| Non-git workspace | Initialized workspace in a plain directory | Git fields degrade to null/unavailable markers; cache/sprint/next sections render normally; exit 0 | Git probe failure → degraded field, never an error exit |
| Stale assigned story | Story row retained `stale`, assigned to the active sprint | Excluded from all four counters (live-only); never counted twice | N/A |
| Own-worktree lease | Lease file for a candidate story in this worktree | Environment Lease line names story/holder/`(this worktree)`; Next returns that story with the 2.11 continuation note | N/A |
| Gate evidence | `gate_runs` rows present (fixture) | Gates line `N/M passing` between DW line and Next | No rows → line omitted / `null` |

## Decisions

_Resolved at checkpoint 1 (2026-09-18); user approved with these decided._

- **D-1 (was OQ-1) — outside a workspace:** exit 0 with a one-line hint; preserve the `requires_workspace` exemption and the pinned tests (chosen over requiring a workspace, exit 2).
- **D-2 (was OQ-2) — sprint story counters:** four fixed mutually exclusive buckets `done` / `in-progress` (unblocked) / `blocked` (computed, non-done) / `backlog` (the rest); they sum to the assigned count and match the §5 layout literally (chosen over per-status counters with an overlapping `blocked` count).
- **D-3 (was OQ-3) — deferred-work scope:** workspace-wide open/unacceptable counts rendered in each active sprint block, documented as global debt (chosen over sprint-scoping via `origin_story_id`).
- **D-4 — gates:** line rendered only from existing `gate_runs` evidence; the "ratchets at baseline" clause is invented data and waits for Epic 3.
- **D-5 — Next line:** both invocations per the AC; phase mapping `draft→specify`, `ready`/`in-progress→develop`; header uses the real `CARGO_PKG_VERSION`.
- **Performance AC:** "100 ms + Git status time" is asserted in CI as its checkable form — no network git args issued and a generous wall-clock bound on the test fixture (design target, not a tight CI gate).

</frozen-after-approval>

## Code Map

- `crates/qdev-cli/src/main.rs:265` — the `None | Some(Commands::Status)` dispatch arm calling the `get_pulse_status` stub; **replace the body** with a `handle_pulse` shared by both arms. `render_next_text` (L504) is the shape to reuse for the Next section's blocker/notes rendering.
- `crates/qdev-cli/src/main.rs:595` — `requires_workspace`: `None`/`Status` stay exempt; update its doc comment (it currently describes `status` as a reporter) to name the pulse.
- `crates/qdev-cli/src/cli.rs:37` — `Status` variant already exists, argument-less; no CLI surface change needed (global `--json` covers output mode).
- `crates/qdev-core/src/lib.rs:153-168` — `PulseStatus`/`get_pulse_status`: the stub this story replaces; delete and export the new pulse API in its place (only caller is `main.rs:266`).
- `crates/qdev-core/src/pulse.rs` — **new**: `PulsePayload` (workspace flag; environment {working_tree, integration, cache, lease}; sprints[] {id, title, release, status, stories{done, in_progress, blocked, backlog}, deferred_work{open, unacceptable}}; gates {passing, total} | null; next: `NextSelection`), `build_pulse` (pure over store/config/git/leases), and the local git-probe helpers (each degrading to null on failure).
- `crates/qdev-core/src/next.rs:277` — `select_next` + `NextSelection`/`NextReason`/`NextBlocker` (L94-158): reuse verbatim, embedded in the payload; its blocked/lease semantics are the pulse's (don't reimplement).
- `crates/qdev-core/src/lease.rs:18,260` — `StoryLease`, `list_leases(root)`; this-worktree filter via canonical path comparison as in `next.rs:460-469`.
- `crates/qdev-core/src/store/mod.rs:409,439,457,496,503-507` — `list_sprints`, `list_deferred_work`, `list_gate_runs`, `get_last_synced_at`, `cache_schema_version`/`cache_missing_tables`; `get_live_entity_for_derivation` (L348) is the live-only rule for every count.
- `crates/qdev-core/src/doctor.rs:99-166` — the cache-field read pattern to mirror (schema_status ok/mismatch, entity_count, cache-native finding_count); do **not** run `run_validation` (doctor's validation section is too heavy for the 100 ms budget).
- `crates/qdev-core/src/config/types.rs:44-61` — `GitConfig { remote, integration_branch }` for the integration line; defaults `origin`/`develop`.
- `crates/qdev-core/src/dw.rs:255,449` — DW `status` values `open`/`done`/`wont_fix`; risk literal `unacceptable`.
- `crates/qdev-core/src/schema.rs:151,215` — `PayloadKind`: add `Pulse` variant, `include_str!("../schemas/payload-pulse.json")`, `as_str`/`from_str_loose` arms, bump `all()` 12 → 13.
- `crates/qdev-core/schemas/payload-pulse.json` — **new** payload schema.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs:87` — pins `status` succeeding outside a workspace (D-1 keeps it; add the no-cache-file assertion if not already covered by the loop).
- `crates/qdev-cli/tests/config_cli_tests.rs:261` — pins boot failure on a config schema violation for the pulse (boot order unchanged; must keep passing).
- `crates/qdev-cli/tests/next_cli_tests.rs` — the real-`git init` + seeded-cache fixture pattern (from spec-2-11) to copy into the new `pulse_cli_tests.rs`.
- `crates/qdev-cli/tests/schema_payload_cli_tests.rs:230-233` — deferred-names list stays `["context", "gate_run"]`; add the `pulse` round-trip beside the `next` one.
- `docs/cli-reference.md:50,297-318` — §2 payload list gains `pulse`; §5 is the target layout; add notes for the no-evidence Gates line, the outside-workspace hint, and the workspace-wide DW count.

## Tasks & Acceptance

**Execution:**
- [ ] `crates/qdev-core/src/pulse.rs` — Create: `PulsePayload` types, local git-probe helpers (branch, short SHA, porcelain count, left-right count; all degrading to null), `build_pulse` (cache fields, active-sprint blocks with the four counters + blocked computation, workspace-wide DW counts, gates summary, embedded `select_next`); pure and read-only.
- [ ] `crates/qdev-core/src/lib.rs` — Export the pulse API; delete the `PulseStatus`/`get_pulse_status` stub.
- [ ] `crates/qdev-core/src/schema.rs` + `crates/qdev-core/schemas/payload-pulse.json` — Add `PayloadKind::Pulse` and the payload schema (workspace flag, environment, sprints, gates, next reusing the next-selection definitions); bump `all()` 12 → 13.
- [ ] `crates/qdev-cli/src/main.rs` — Replace the `None | Some(Commands::Status)` arm with `handle_pulse` (D-1 workspace check, query store, resolved identity, §5 text renderer, JSON envelope); update the `requires_workspace` doc comment; reuse the `render_next_text` shape for the Next section.
- [ ] `crates/qdev-core/tests/pulse_tests.rs` — **new**: four-counter exclusivity and sum, stale-row exclusion, DW open/unacceptable counts, gates null-vs-present, no-active-sprints, non-git degradation, own-worktree lease filter, and a shuffled-fixture determinism test.
- [ ] `crates/qdev-cli/tests/pulse_cli_tests.rs` — **new** (git-init + seeded fixture pattern from `next_cli_tests.rs`): text layout assertions per §5, `--json` envelope + `payload-pulse.json` round-trip, `next` equals `qdev next` selection, outside-workspace exit-0 hint with no cache file, non-git workspace, own-worktree lease line, gate-evidence case, and the performance-AC form (no network git args; wall-clock bound).
- [ ] `crates/qdev-cli/tests/schema_payload_cli_tests.rs` — Add the `pulse` payload round-trip; keep the deferred-names list unchanged.
- [ ] `docs/cli-reference.md` — Add `pulse` to the §2 payload list; §5 notes: Gates line only with evidence, outside-workspace behaviour, workspace-wide DW count, real-version header.

**Acceptance Criteria:**
- Given an initialized workspace with an active sprint, when running `qdev`, then the output matches the §5 layout: Environment (working tree, integration, cache, lease), per-sprint block with the four counters and the DW line, and a Next section carrying both the skill and CLI invocation.
- Given the same state, when running `qdev --json`, then the versioned `pulse` payload is emitted, round-trips `payload-pulse.json`, and its `next` equals `qdev next --json`'s selection for the same input.
- Given no active sprints, when running `qdev`, then the sprints section reports none and `next` is null with reason `no_active_sprints`; exit 0.
- Given a directory without `qdev.toml`, when running `qdev`, then exit 0 with the one-line init hint and no cache file created.
- Given an initialized workspace in a non-git directory, when running `qdev`, then the git fields degrade to null markers while cache/sprint/next sections render; exit 0.
- Given `gate_runs` rows, when running `qdev`, then the Gates line shows `N/M passing`; with no rows the line is omitted (text) / `null` (JSON).
- Given any inputs, then no file, cache row, lease, or story state changed (asserted in tests via directory listing / cache checksum).
- Given the reference test fixture, when running `qdev`, then it completes within 100 ms plus Git status time — asserted as no network git args plus the wall-clock bound.

## Implementation Notes

## Spec Change Log

| Date | Source | What was flagged | What changed |
|------|--------|------------------|--------------|

## Review Triage Log

## Design Notes

Payload shape (JSON):

```json
{ "schema_version": "1", "workspace": true,
  "environment": {
    "working_tree": { "git_repository": true, "clean": true, "dirty_files": 0, "branch": "feature/x", "head": "8f1b2c4" },
    "integration": { "remote": "origin", "branch": "develop", "state": "up_to_date", "ahead": 0, "behind": 0 },
    "cache": { "schema_status": "ok", "entity_count": 184, "finding_count": 0, "synced_ms_ago": 18 },
    "lease": { "story_id": "E12S4", "holder": "simon", "started_at": "…", "branch": "feature/x" } },
  "sprints": [ { "id": 5, "title": "…", "release": "0.1.0", "status": "active",
                 "stories": { "done": 42, "in_progress": 12, "blocked": 3, "backlog": 108 },
                 "deferred_work": { "open": 8, "unacceptable": 0 } } ],
  "gates": null,
  "next": { "…": "NextSelection verbatim" } }
```

Counter order for the D-2 buckets, per assigned story: `done` first; then `in-progress` iff status `in-progress` and not computed-blocked; then `blocked` iff not done and computed-blocked (same `depends_on` live-`done` semantics as `next.rs`); else `backlog`. Multiple this-worktree leases render one line each, sorted by story id. `workspace: false` nulls `environment`, `sprints` (empty), `gates`, and `next`.

## Verification

**Commands:**
- `cargo test --test pulse_tests` — expected: counters, filters, degradation, and determinism tests green.
- `cargo test --test pulse_cli_tests` — expected: end-to-end pulse against seeded fixtures, incl. outside-workspace exit 0 and `next` parity with `qdev next`.
- `cargo test --test schema_payload_cli_tests` — expected: `payload-pulse.json` round-trips; `all()` invariant covers `pulse`; deferred list unchanged.
- `cargo test --workspace` — expected: full suite green (incl. the pinned guard/boot tests).
- `cargo clippy --workspace --all-targets -- -D warnings` — expected: clean (CI gate).
- `cargo fmt --all -- --check` — expected: no diff in touched files.