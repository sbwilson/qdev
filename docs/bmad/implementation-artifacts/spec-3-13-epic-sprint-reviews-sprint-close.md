---
title: 'Story 3.13: Epic & Sprint Reviews, Sprint Close'
type: 'feature'
created: '2026-09-27'
status: 'done'
route: 'dispatch'
baseline_commit: 'b2d997dbe20e12f9048f287460e2ffaa8a33cd07'
review_loop_iteration: 1
context:
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** qdev can record sprints, gate evidence, deferred work, ratchets, and SOUP findings, but release managers have no consistent review surface or safe way to finalize a sprint from that evidence.

**Approach:** Add epic and sprint review commands that render durable release reports, then extend sprint close to enforce its policy checks, capture an auditable baseline, support the requested terminal statuses, and carry unfinished work without changing story identity.

## Boundaries & Constraints

**Always:** Build reviews from the hydrated Markdown-backed store; run `sprint_close` gates through the shared gate runner; write release artifacts and state through the existing atomic write path; preserve normal JSON/text envelopes and established exit codes; validate every refusal before any sprint, release, assignment, or decision mutation.

**Never:** Reimplement gate execution or evidence writing; infer missing evidence, DW rationale, or release data; rename/mutate story IDs during carry-over; close a sprint after a policy refusal; create a baseline snapshot for `paused` or `abandoned` closure.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Epic review | `review epic E12` with completed stories and evidence | Report statuses, originating open DW by risk, evidence coverage, and unclassified debt | Exit 1 if a done story lacks required bound-gate evidence |
| Sprint review | Active sprint linked to a release | Run `sprint_close` gates and write deterministic `rtm.md` and `anomalies.md` beneath that release | Propagate gate failure/infra outcome; do not fabricate report data |
| Completed close | Eligible sprint and optional carry-over target | Snapshot counts, ratchets, commit SHA, and SOUP summary; mark completed and assign each unfinished story with `carried_from` | Validate all targets and policy checks before mutations |
| Blocked or alternate close | Unjustified unacceptable DW, live lease on in-progress story, or `paused`/`abandoned` status | Refuse safety violations with exit 3 and no state change; alternate status records a reasoned DEC and skips baseline | Require a non-empty reason for alternate terminal status |

</frozen-after-approval>

## Code Map

- `crates/qdev-cli/src/cli.rs`, `crates/qdev-cli/src/main.rs`, and `crates/qdev-cli/src/handlers/mod.rs` -- add and dispatch the new `review` command tree; extend sprint-close arguments with status/reason while retaining output conventions.
- `crates/qdev-cli/src/handlers/sprint.rs` and new `crates/qdev-cli/src/handlers/review.rs` -- resolve workspace, store, author, and sprint selection; map core results/errors to JSON and concise text.
- `crates/qdev-core/src/sprint.rs` -- extend the existing atomic close/carry-over lifecycle rather than duplicating it; perform refusal checks before writes and assemble the completed baseline snapshot.
- `crates/qdev-core/src/review.rs` and `crates/qdev-core/src/lib.rs` -- project epic evidence/DW/debt summaries and sprint RTM/anomaly artifacts from store records, shared gates, and SOUP summary seams.
- `crates/qdev-core/src/{gate/runner.rs,gate/mod.rs,lease.rs,dw.rs,soup.rs,store/mod.rs}` -- reuse transition-gate ordering/execution, immutable evidence, cross-worktree lease lookup, DW fields, SOUP records, and store queries; do not alter their domain contracts unnecessarily.
- `crates/qdev-core/schemas/{sprint.json,release.json}` and write helpers -- admit the requested close statuses/reason and persist a schema-valid release baseline snapshot.
- `crates/qdev-core/tests/{sprint_tests.rs,review_tests.rs}` and `crates/qdev-cli/tests/{sprint_cli_tests.rs,review_cli_tests.rs}` -- cover core policy ordering/artifacts and end-to-end command, envelope, and filesystem behavior.

## Tasks & Acceptance

**Execution:**
- [x] CLI review/sprint files -- add `qdev review epic|sprint`, status/reason parsing, dispatch, and stable JSON/text result rendering.
- [x] Core review module -- generate deterministic epic summaries plus release `rtm.md` and risk-grouped `anomalies.md`; run selected `sprint_close` gates via the existing resolver/runner and include evidence paths.
- [x] Core sprint lifecycle and schemas -- preflight unacceptable open DW and globally live leases for assigned in-progress stories, record completed baseline details, preserve carry-over semantics, and implement paused/abandoned DEC-backed close without a baseline.
- [x] Core and CLI tests -- exercise successful reports, missing bound evidence, gate failure, report data shape, refusal atomicity, baseline contents, carry-over, and alternate close status/reason behavior.

**Acceptance Criteria:**
- Given an epic, when `qdev review epic <id>` runs, then it reports statuses, originating open DW with risk, gate-evidence coverage, and unclassified debt, exiting 1 when a done story lacks its bound-gate evidence.
- Given an active release-linked sprint, when `qdev review sprint <id>` runs, then it executes `sprint_close` gates and writes its RTM and residual-anomaly reports under that release.
- Given an eligible active sprint, when it is completed with a carry-over target, then it records the required baseline and carries every non-done assignment with its original ID and `carried_from` value.
- Given an unacceptable unrationalized DW or a live lease on an in-progress assigned story, when close is requested, then qdev exits 3 with no durable close/carry-over mutation.
- Given a paused or abandoned close with a reason, when it succeeds, then it writes the status and an attributed decision but no completed baseline snapshot.

### Review Findings

_Code review 2026-09-27 (layers: blind-hunter, edge-case-hunter, verification-gap, acceptance-auditor). 4 decision-needed (all resolved 2026-09-27), 19 patch, 0 defer, 0 rejected._

- [x] [Review][Decision] Sprint status schema drops `planning`, contradicting the cache layer — **Decided (a): re-admit `planning`** — the file schema enum accepts the same five statuses the cache CHECK does; the release `baseline_snapshot` keeps accepting `string` (no legacy invalidation).
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Re-admit `planning` to the sprint file schema enum (matching the unmodified cache CHECK) and restore `baseline_snapshot` to `["string","object"]` in the release schema [crates/qdev-core/schemas/sprint.json:21, crates/qdev-core/schemas/release.json:83]
- [x] [Review][Decision] `review sprint` gate runs are attributed to the wrong story and invisible in its own RTM — **Decided (a): explicit workspace-level attribution** — the gate runner gains a workspace-level option that bypasses the lease fallback so sprint-review runs land under `_workspace`.
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Add a workspace-level attribution option to the gate runner and use it from `review_sprint`, so its gate runs are attributed to `_workspace` (never to an unrelated lease) and the RTM/payload reflect them [crates/qdev-core/src/gate/runner.rs:210, crates/qdev-core/src/review.rs:168]
- [x] [Review][Decision] RTM contains no requirement mapping — **Decided (a): add a requirements column derived from `traces_to`**, so the matrix actually traces requirements to stories/gate runs/evidence per AD-8.
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Add a requirements column to the RTM built from each assigned story's `traces_to` relations [crates/qdev-core/src/review.rs:213]
- [x] [Review][Decision] `sprint_close` gate binding never fires at close time — **Decided (a): keep the binding, document it** — `on_transition = ["sprint_close"]` runs via `qdev review sprint`; the doc note clarifies that actual `qdev sprint close` does not re-execute gates.
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Document that `sprint_close`-bound gates fire via `qdev review sprint` (not at `qdev sprint close`), e.g. in the `review sprint` help text and the `close_sprint` doc [crates/qdev-cli/src/cli.rs:1052, crates/qdev-core/src/sprint.rs:548]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite DW "open" predicate tests against a status that does not exist — `!= "closed"` is true for every DW because the domain vocabulary is `open`/`done`/`wont_fix` (`crates/qdev-core/src/dw.rs:441-449`, `crates/qdev-core/src/store/sqlite.rs:137`): resolved deferred work appears as "open" in the epic report and `anomalies.md`, and a resolved `unacceptable` DW with an empty `rationale` makes every completed sprint close refuse permanently [crates/qdev-core/src/review.rs:121, crates/qdev-core/src/sprint.rs:633]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Safety preflight runs only for `--status completed` — the unacceptable-DW and live-lease refusals are nested in `if status == "completed"`, so a `paused`/`abandoned` close mutates the sprint despite both safety violations, violating AC 4 ("when close is requested") and the frozen matrix [crates/qdev-core/src/sprint.rs:631]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Workspace regression: Story 3.4 test fails — the flat → nested `baseline_snapshot` restructure leaves `fm["baseline_snapshot"]["warning-count"]` as `Null`; `cargo test --workspace` is red and the spec's Verification claim ("the complete regression suite passes") is false [crates/qdev-cli/tests/gate_baseline_cli_tests.rs:522]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Release baseline snapshot runs after the sprint is durably mutated, and preflight/mutation disagree on release linkage — preflight checks only existence from the cache row (`release_version`), while the snapshot block reads the file's `release`/`release_version` precedence and fails mid-mutation (read error, `missing_frontmatter`, `schema_violation`) leaving a closed sprint with no baseline; add a pre-lock parse+validate of the release file with one source of truth, plus the missing-release close test [crates/qdev-core/src/sprint.rs:614, crates/qdev-core/src/sprint.rs:927]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite No guard against re-closing an already-terminal sprint — re-closing re-bumps version, rewrites `completed_at`, re-snapshots the release baseline, and a repeated `paused`/`abandoned` close appends a fresh DEC record each time [crates/qdev-core/src/sprint.rs:549]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Decision record written after the protected window with no compensation — if the DEC write fails after the sprint file and cache are already durably `paused`/`abandoned`, the audit record the close promises is missing with no recovery hint; return a distinct error naming the partial state and how to record the DEC [crates/qdev-core/src/sprint.rs:1068]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite `review sprint` has no active-sprint guard — it re-executes every configured `sprint_close` gate (arbitrary user subprocesses) and overwrites the release reports for a completed/paused/abandoned sprint; the frozen matrix requires an "Active sprint"; report and evidence writes also run outside the advisory write lock, and the module doc still claims "read-only" [crates/qdev-core/src/review.rs:146]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Epic review silently drops stale stories — `.filter(|s| !s.stale)` removes a `done` story whose file became unreadable/unparseable (the tamper/drift case the audit exists to catch) instead of reporting it; the story vanishes from the exit-1 missing-evidence check and the "N stories" count; the RTM surfaces the same condition as `"missing"` [crates/qdev-core/src/review.rs:66]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Completed baseline snapshot has data-integrity gaps — the SOUP summary aggregates records across *all* releases (no `evaluated_for_release` filter, `crates/qdev-core/src/store/mod.rs:246`), a ratchet baseline that fails to parse is silently omitted from the snapshot, and an unresolvable commit writes a literal `"unknown"` into the auditable baseline [crates/qdev-core/src/sprint.rs:1008]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite New-surface test coverage gaps — no positive `unclassified_debt` test, no CLI `review epic` success test with a `review`-bound gate (the config→required-gates filter is never exercised), RTM/`anomalies.md` content is never asserted (file existence only; core `review_sprint` has no test at all), no `carry_over_not_allowed` refusal test, the alternate-close test pins only the JSON id (no linked release, so the no-baseline guard is unobservable; `close_reason` and DEC content unasserted), and no invalid-`--status` / re-close / text-mode tests [crates/qdev-core/tests/review_tests.rs, crates/qdev-cli/tests/review_cli_tests.rs]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Formatter churn contradicts the spec's claim — the spec says drift is "pre-existing … in unrelated files", but the HEAD versions of the diff's own files were fmt-clean and the diff introduces drift in three of them (`crates/qdev-cli/src/handlers/review.rs`, `crates/qdev-core/src/lib.rs`, `crates/qdev-core/src/sprint.rs`) while also reformatting unrelated arms in `crates/qdev-cli/src/main.rs`; `cargo fmt --check` is dirtier in the diff's own footprint than it was at HEAD [crates/qdev-cli/src/handlers/review.rs:20]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite `anomalies.md` is risk-sorted, not risk-grouped, and the JSON payload carries no summary — the spec task says "risk-grouped anomalies.md" but a single flat table is emitted, and `SprintReview` exposes no counts or anomaly list, forcing consumers to parse Markdown [crates/qdev-core/src/review.rs:228]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Small close/review surface nits — invalid `--status` has no machine-readable code (bare usage error, unlike sibling refusals); `--status completed --reason` silently persists `close_reason` with no DEC record (help text says the reason is for paused/abandoned); the `close_sprint` doc comment still describes only the old completed-only behavior; the alternate-close DEC hardcodes `decision_type: "human_ruling"` even for agent-authored closures; and the `review sprint` text line omits the release id [crates/qdev-core/src/sprint.rs:556, crates/qdev-core/src/sprint.rs:1069, crates/qdev-cli/src/handlers/review.rs:60]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Tracking surfaces disagree within the same change — the spec frontmatter says `status: 'in-review'` while the diff sets `3-13-…: in-progress` in sprint-status.yaml [docs/bmad/implementation-artifacts/sprint-status.yaml:94]
- [x] [Review][Patch] — applied 2026-09-27, verified by full workspace suite Evidence-recency and coverage semantics in epic review — a historical passing gate run from an old commit counts as current evidence for a `done` story (no freshness check), `evidence_paths` includes fail/skipped runs while coverage counts passes only, the missing-evidence error can print an empty list, and the governing `&&`/`||` condition needs explicit parentheses to be obviously correct [crates/qdev-core/src/review.rs:103]

**Rejected:** none — all 49 raw findings verified as real (17 blind-hunter, 12 edge-case-hunter, 7 verification-gap, 13 acceptance-auditor, with 15 cross-layer duplicates merged into shared root causes).

## Implementation Notes

Planning facts: this is one cross-layer release-governance goal; it has no schema migration or external side effect, but successful commands intentionally mutate workspace release/sprint/assignment/decision artifacts. The expected footprint is the CLI surface, review projection, sprint lifecycle/schema, and focused core/CLI tests.

Implemented `qdev review epic|sprint`, deterministic release artifacts, close preflight policy checks, completed baseline snapshots, and paused/abandoned decision records. Focused core and CLI matrix tests pass; the repository-wide formatter check still reports pre-existing drift in unrelated files.

## Spec Change Log

## Review Triage Log

## Design Notes

Keep review projections read-only except for the requested release reports. For close, stage every resolvability, safety, lease, and target check before the existing locked write sequence, because the current lifecycle begins durable mutation before all Story 3.13 policy prerequisites have been evaluated.

## Verification

**Commands:**
- `cargo test -p qdev-core review sprint` -- expected: review projections, refusal ordering, lifecycle, and report persistence pass.
- `cargo test -p qdev-cli --test review_cli_tests --test sprint_cli_tests` -- expected: CLI commands, envelopes, reports, and close policies pass.
- `cargo test --workspace` -- expected: the complete regression suite passes.
