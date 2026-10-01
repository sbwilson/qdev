---
title: 'Story 2.12: The Root Pulse'
type: 'feature'
created: '2026-09-18'
status: 'done'
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
- [x] `crates/qdev-core/src/pulse.rs` — Create: `PulsePayload` types, local git-probe helpers (branch, short SHA, porcelain count, left-right count; all degrading to null), `build_pulse` (cache fields, active-sprint blocks with the four counters + blocked computation, workspace-wide DW counts, gates summary, embedded `select_next`); pure and read-only.
- [x] `crates/qdev-core/src/lib.rs` — Export the pulse API; delete the `PulseStatus`/`get_pulse_status` stub.
- [x] `crates/qdev-core/src/schema.rs` + `crates/qdev-core/schemas/payload-pulse.json` — Add `PayloadKind::Pulse` and the payload schema (workspace flag, environment, sprints, gates, next reusing the next-selection definitions); bump `all()` 12 → 13.
- [x] `crates/qdev-cli/src/main.rs` — Replace the `None | Some(Commands::Status)` arm with `handle_pulse` (D-1 workspace check, query store, resolved identity, §5 text renderer, JSON envelope); update the `requires_workspace` doc comment; reuse the `render_next_text` shape for the Next section.
- [x] `crates/qdev-core/tests/pulse_tests.rs` — **new**: four-counter exclusivity and sum, stale-row exclusion, DW open/unacceptable counts, gates null-vs-present, no-active-sprints, non-git degradation, own-worktree lease filter, and a shuffled-fixture determinism test.
- [x] `crates/qdev-cli/tests/pulse_cli_tests.rs` — **new** (git-init + seeded fixture pattern from `next_cli_tests.rs`): text layout assertions per §5, `--json` envelope + `payload-pulse.json` round-trip, `next` equals `qdev next` selection, outside-workspace exit-0 hint with no cache file, non-git workspace, own-worktree lease line, gate-evidence case, and the performance-AC form (no network git args; wall-clock bound).
- [x] `crates/qdev-cli/tests/schema_payload_cli_tests.rs` — Add the `pulse` payload round-trip; keep the deferred-names list unchanged.
- [x] `docs/cli-reference.md` — Add `pulse` to the §2 payload list; §5 notes: Gates line only with evidence, outside-workspace behaviour, workspace-wide DW count, real-version header.

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
| 2026-09-19 | implementation / matrix audit | Boot `ensure_cache` ran for **every** command when `qdev.toml` exists, so the pulse's own boot either rebuilt the cache or ran `sweep_workspace` — and the sweep stamps `sync_meta (id, last_synced_at)` with `current_iso8601()`. That broke the "Any inputs → nothing changed" matrix row (`test_pulse_writes_nothing` went red whenever two runs straddled a second boundary) and, with the cache deleted, **repaired** the workspace and printed `Cache healthy (synced 632 ms ago, 0 entities, 0 findings)` instead of reporting it | The default command and its `status` alias no longer run the boot `ensure_cache`. `handle_pulse` resolves the cache from `storage.cache_dir` itself: no cache file → pulse with no store; an existing cache is opened only while `inspect_cache_schema` says it is readable; `NewerThanSupported` is refused with the same `schema_version_mismatch` (exit 5) every other command gives; anything else (mismatch, missing tables, unparseable stamp) → no store, cache reported degraded (`schema_status: "mismatch"`, null counts, `sprints` empty, `next: null`), exit 0, nothing created, no lock taken. `build_pulse` now handles `store: None` inside a workspace instead of erroring `cache_unavailable`, and the text renderer prints a `Next` section saying nothing can be recommended (suggesting `qdev sync`) with `"next": null` in JSON. The three `cache_cli_tests` boot pins (`test_cli_boot_with_empty_cache`, `test_cli_boot_with_schema_mismatch`, `test_cli_table_by_table_identical_cache_rebuild`) plus the hydration pins in `sweep`/`list`/`get`/`update`/`relate`/`create`/`init` are re-driven with `qdev doctor`, which still gets the boot `ensure_cache`; no assertion was weakened. `docs/cli-reference.md` §5 records the new contract. |

## Review Triage Log

| # | Source | Finding | Location | Verdict | Evidence / disposition |
|---|--------|---------|----------|---------|------------------------|
| 1 | blind-hunter | Default `--json` payload shape replaced while `schema_version` stays `"1"`; pinned `PulseStatus` assertions deleted without record | `cli_tests.rs`, `network_tests.rs`, spec | false | The stub's replacement is this story's own design (Code Map: delete it, only caller was `main.rs`); the Spec Change Log entry documents the change and the test re-drives, and `pulse` is a *new* payload kind with its own v1 schema — no existing payload kind's version was silently changed. |
| 2 | blind-hunter | I/O matrix omits cache-missing / cache-newer / multi-sprint / multi-lease / `status --json` cases | spec matrix | false | `test_json_envelope_and_next_parity_and_schema_round_trip` does run `["status", "--json"]` (L887) and `test_non_interactive_flag` pins outside-workspace `--json` with all-nulls; the cache-missing/newer behaviors are tested (`test_mismatched_cache…`, `test_newer_cache…` with exit 5) and documented in §5 + Design Notes. Adding matrix rows would be editing this build's spec — rejected per triage rules. |
| 3 | blind-hunter | Purity tests cannot fail — both warm up first, so first-open effects are never compared | `pulse_cli_tests.rs::test_pulse_writes_nothing`, `pulse_tests.rs::test_build_pulse_writes_nothing` | false | Verified by experiment: cold `qdev` in a fresh `init`-ed workspace and immediately after `qdev sync` creates and changes no file (tree + checksums identical before/after the first run). There is no write path to miss — the pulse never opens a store it would create. Grouped with ECH-8 (same root cause). |
| 4 | blind-hunter | "Never sweeps / no hydration" unverified against a valid cache; `sweep_cli_tests` switched to `doctor`, module doc allegedly still misleading | `sweep_cli_tests.rs` | false | The module doc *was* updated to record the switch ("the default command and its `status` alias are deliberately not such a command any more (Story 2.12)"); `handle_pulse`/`build_pulse` never call `ensure_cache`, sweep, or hydration, so the property is structural — untestable-in-principle cases don't need proofs of structurally absent code. |
| 5 | blind-hunter | `render_pulse_next_text` copies `render_next_text` but drops Blockers (on selection) and the `Leased by … finish it` line | `main.rs` pulse Next renderer | false | Decided D-5 defines exactly this layout: a selection renders the summary plus the `Run:` line; blockers render only in the `next: null` case; lease/continuation info remains in the Environment `Lease` line and in `reason.notes`, and `qdev next` itself still shows full blockers. Grouped with ECH-3 and ECH-9. |
| 6 | blind-hunter | `payload-pulse.json` under-constrains (`schema_version` type-only, `gates` allows `passing` > total, negative sprint ids, `status` `const` vs code fill, copied next defs) | `schemas/payload-pulse.json` | false | Every under-constrained shape is unreachable from the code (`passing` is counted from `latest` keys so `passing ≤ total` always; sprint blocks require `Some("active")`; ids are allocated positive; only ever `"1"` is emitted). Per-kind schemas owning their own definitions is the shipped pattern; nothing consumes these constraints. |
| 7 | blind-hunter | `lease_clock` shows `… since 00:00` for earlier-date leases; raw ms sync age; ungrammatical `synced never — run \`qdev sync\`, N entities` | `main.rs` renderers | false | The time-only `since HH:MM` format, raw-ms rendering, and the exact "synced never" wording are the documented §5 / Design-Notes contract ("`since 09:41`", "`synced 18 ms ago`"); the never-synced string is reachable only for a never-synced cache and directs to the working fix. A different display format would be a renegotiation of the layout the spec pins. |
| 8 | blind-hunter | Recovery guidance conflicts: Cache line says `sync --rebuild`, Next says `run qdev sync` | `main.rs` renderers, `docs/cli-reference.md` §5 | false | Verified by experiment: plain `qdev sync` recreates a missing cache and repairs a stale stamp, and §5 explicitly documents "Run `qdev sync` (or `qdev sync --rebuild`)" — both paths work; the stronger advice in the degraded line is also sufficient, and a newer-stamp cache exits 5 before any Next section renders. |
| 9 | blind-hunter | Perf/network AC asserted in name only (source-grep; prefix allow-list; 10 s bound; network tests only run outside a workspace) | `pulse_tests.rs`, `pulse_cli_tests.rs`, `network_tests.rs` | false | The spec itself defines the checkable form as "no network git args + a generous wall-clock bound (design target, not a tight CI gate)"; every probe in `pulse.rs` passes literal local-only argument vectors to `git` (read in full — no dynamic command construction), so the grep encodes the real contract. |
| 10 | blind-hunter | `docs/cli-reference.md` incomplete: §2 omits `transition`/`claim`/`release`/`chore`; §5 example shows counts no test produces | `docs/cli-reference.md` | false | §2's row lists exactly the kinds relevant to the changes made here; the four omissions predate this story (they shipped undocumented in stories 2.1–2.10), and the §5 block has been an illustrative example since it was written — this story corrected only what it changed (header version, Gates evidence rule). |
| 11 | blind-hunter | `_bmad/config.toml` gains `communication_language` — unrelated change, "no config keys" violation | `_bmad/config.toml` | false | Committed in `a0ad941` ("chore(BMAD): missing language setting") *before* story start (`779c9f7`); it's BMAD harness config, not a qdev `config key`. It appears in the diff only because `baseline_commit` predates it. |
| 12 | blind-hunter | Tracking metadata contradicts itself (`in-review` vs sprint-status `in-progress`; empty sections; no Decisions entry for the boot change; `epic-2-context.md` keeps the ≤ 100 ms promise) | spec frontmatter, `sprint-status.yaml` | false | The lifecycle is by design: `in-review` while the review runs, sprint-status advances to `review` at step-05; Decisions record checkpoint-1 resolutions while post-approval changes belong in the Spec Change Log, which has the entry; empty `Implementation Notes`/`Review Triage Log` are template placeholders. |
| 13 | blind-hunter | New tests re-declare `setup_workspace`/`write_story`/… and paste the gate-seeding block twice | `pulse_cli_tests.rs`, `cli_tests.rs` | false | Integration-test files in this repo are self-contained by convention (`next_cli_tests.rs` etc. already duplicate the same helpers); there is no shared test-support crate, and no runtime behavior is affected. |
| 14 | blind-hunter | Pulse never tested from a workspace subdirectory or with a relocated `storage.cache_dir` | tests | false | Verified by experiment: run from `sub/deep` and with `cache_dir = "custom/cache"` the pulse finds the workspace root and the configured cache (`Cache healthy (synced …, …)`); `find_workspace_root` walks up and `root.join(cache_dir)` handles relative and absolute values — correct behavior, merely untested, with no demonstrated defect. |
| 15 | edge-case-hunter | `inspect_cache_schema` `Err` shares the `Mismatch` arm — corrupt/permission-denied caches report "schema mismatch", real cause dropped | `main.rs:568-590` | false | Every failure kind renders "degraded — run `qdev sync --rebuild`", and both sync variants demonstrably recover a missing/stale cache; the spec prescribes exactly this: "anything else (mismatch, missing tables, unparseable stamp) → no store, cache reported degraded", exit 0. No reachable case needs the finer cause. |
| 16 | edge-case-hunter | Cache deleted between `inspect` and `open` → `SqliteStore::open` recreates it; read-only command would create cache | `main.rs:570-577` | false | Requires concurrent deletion inside a sub-second window no test or user path shows; and this check-then-open pattern is the same one `next` uses — unchanged by this story. Unshown-reachable situation; per triage rules this is correct behavior for that situation. |
| 17 | edge-case-hunter | Blockers render only in the `next: null` branch — a selection hides other blocked stories | `main.rs:645-680` | false | Same claim as row 5 (grouped with it) — D-5 deliberately specifies the pulse's Next layout as summary + `Run:`; blockers remain in the `next: null` case, in JSON, and in `qdev next`. |
| 18 | edge-case-hunter | Latest-run-per-gate chosen by string compare of raw `ran_at` — mixed offsets could pick the wrong run | `pulse.rs:533-548` | false | `gate_runs` rows are only written by test fixtures until Epic 3 exists (the runner that could write varied formats doesn't exist); with uniform `…Z` formats string order equals time order, and the worst case is a cosmetically stale `N/M passing`. |
| 19 | edge-case-hunter | `integration_status` keyed on the working-tree probe (`dirty_files.is_some()`) — a failed `git status` in a real repo skips the comparison | `pulse.rs:210-216` | false | A git repo where `git status --porcelain` fails while every other probe succeeds is not a situation this program can reach; if it somehow occurred, skipping to the "not available" marker is precisely the spec's "failed probe degrades that field" behavior. |
| 20 | edge-case-hunter | Store `None` inside a workspace gives `next: null` with no reason code — consumers can't tell unreadable cache from no candidates | `pulse.rs:437-444` | false | `environment.cache` reports `schema_status: "mismatch"` with null counts — that *is* the machine-readable signal — plus the rendered Next hint; the spec itself prescribes `next: null` for this case, so `"next": null` cannot also carry a selection-reason code without changing the contract. |
| 21 | edge-case-hunter | `git_probe` has no timeout — a hung git stalls the pulse | `pulse.rs:196-207` | false | Documented as intentional in `git_probe` and in the spec: the budget is held by the bounded local probe set, not by a deadline; all probes are local reads that cannot reach the network. Adding timeout machinery exceeds the spec's stated design. |
| 22 | edge-case-hunter | Purity tests warm up first — first-open effects (sidecars, dir creation) unverified | `test_*_writes_nothing` | false | Same claim as row 3 (grouped) — disproven by running cold in a seeded workspace: nothing is created or changed on a first run. |
| 23 | edge-case-hunter | `render_pulse_next_text` is a copy of `render_next_text`, not a call — pulse text can drift from `qdev next` text | `main.rs` | false | Same root cause as rows 5/17 (grouped): the layouts are *intended* to differ per D-5; only JSON parity is contractually pinned and tested, and it holds. |
| 24 | verification-gap | Integration states `behind`/`ahead`/`diverged`/missing-ref never produced by any test — direction mapping could ship inverted with a green suite | `pulse.rs:207-256` etc. | medium | Pre-verified gap, kept. The mapping is currently *correct* (verified live: 1-commit-ahead renders "is 1 ahead of origin/main", tracking renders "up to date"), but no test pins any non-`up_to_date` state. → patch: extend the git fixtures with ahead/diverged/missing-ref cases. |
| 25 | verification-gap | "Gates line renders once across multiple active sprints" (D-4 / §5) never exercised — no test has >1 active sprint, ever, with gate evidence | `main.rs` render + tests | low | Pre-verified gap, kept. The `i == 0` guard works; nothing pins it. → patch: seed two active sprints + `gate_runs` rows and assert exactly one `Gates` line. |
| 26 | verification-gap | `synced_ms_ago` never computed from a real `sync_meta` row — the seconds→ms conversion has no test | `pulse.rs` cache section | medium | Pre-verified gap, kept. The conversion is currently correct (verified live: "synced 668 ms ago" right after sync), but deleting the `* 1000` would pass everything. → patch: seed a fixed `sync_meta` row + fixed `now` and assert the exact value. |

**Grouping & routing:** rows 3+22 (one entry, `false` — rejected); rows 5+17+23 (one entry, `false` — rejected per D-5); rows 24, 25, 26 → three `patch` entries (no `intent_gap`/`bad_spec` found, so no loopback; `review_loop_iteration` stays 0); everything else rejected on refutation or as negligible `low`/`false`.

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