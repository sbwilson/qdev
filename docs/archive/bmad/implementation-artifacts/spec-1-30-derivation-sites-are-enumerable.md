---
title: 'Derivation sites are enumerable'
type: 'bugfix'
created: '2026-09-12'
status: 'done'
route: 'dispatch'
review_loop_iteration: 2
baseline_commit: 'b4eed6ead305d7b667e2211ef2e777f1442100f5'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/spec-stale-means-absent.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev get` computes a story's `blocked` state from a raw cached dependency row. When the dependency is retained stale with status `done`, `get` reports the dependent unblocked while relation validation correctly treats the same dependency as dangling. The earlier stale-means-absent story centralised the rule but did not make new derivation sites mechanically visible.

**Approach:** Apply the existing derivation-presence rule before using a dependency's status, then configure an enforcement mechanism so raw stale-inclusive entity reads require an explicit, documented exception rather than silently becoming a derivation site.

## Boundaries & Constraints

**Always:**
- A stale dependency is absent for computing `blocked`, exactly as it is for relation validation and a full rebuild; a stale dependency whose retained status is `done` therefore blocks its dependent story.
- `get_entity` and `list_entities` remain stale-inclusive read APIs: `qdev get` and `qdev list` continue returning retained rows with `stale: true`.
- Every intentional raw stale-inclusive `Store::get_entity` use is explicitly marked as a read/reporting exception, while the linter rejects unmarked future uses.
- The existing `entity_presence_for_derivation` / `entity_exists_for_derivation` rule remains the sole answer to existence for derived state; live and genuinely missing dependency behaviour remains unchanged.

**Never:**
- Do not change retention, cache hydration, relation graph SQL filtering, finding codes, payload schemas, or the meaning of relation writes.
- Do not filter stale rows from ID allocation or reporting-path lookups; those are documented exceptions with different semantics.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Live done dependency | Story depends on a live target with `status: done` | `qdev get` reports `blocked: false` | N/A |
| Live unfinished dependency | Story depends on a live target not `done` | `qdev get` reports `blocked: true` | N/A |
| Missing dependency | Story depends on no cached target | `qdev get` reports `blocked: true` | N/A |
| Stale done dependency | Target is retained stale with `status: done` | `qdev get` reports `blocked: true`, agreeing with validation's dangling edge | N/A |
| Direct raw derivation read | A new unapproved `Store::get_entity` call is added | Clippy fails and directs the author to choose an explicit exception or derivation helper | Lint failure |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/query.rs` -- `compute_blocked` is the P3-11 defect: it reads a dependency through stale-inclusive `Store::get_entity` and must use the existing derivation presence rule before accepting its status.
- `crates/qdev-core/src/store/mod.rs` -- documents the raw read contract and provides the derivation helpers; add one coherent live-entity retrieval helper so a presence decision and status come from the same row snapshot.
- `crates/qdev-core/src/store/sqlite.rs` -- owns the optimized derivation-presence implementation and whole-graph SQL live-row filters; do not replace those graph queries with per-row reads.
- `crates/qdev-core/src/validate.rs` -- examples of correct helper use and the deliberate `deferred_work_path` reporting-path raw-read exception.
- `crates/qdev-core/tests/query_tests.rs` -- existing live, unfinished, dangling, and non-story `blocked` coverage; add the stale-done regression beside them.
- `clippy.toml`, `crates/qdev-core/src/doctor.rs`, and `crates/qdev-cli/src/main.rs` -- new lint configuration for both stale-inclusive Store reads, with narrow documented allows at reporting, ownership, graph, and write-gate sites.
- `docs/architecture.md` -- update the stale-derivation invariant and exception inventory to name the enforcement mechanism.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/mod.rs` and `crates/qdev-core/src/query.rs` -- provide and use one coherent live-entity-for-derivation read for `blocked`, so status and staleness are from the same cached row snapshot -- remove the concurrent stale-transition window.
- [x] `clippy.toml` and intentional raw-read call sites under `crates/qdev-core/src/` and `crates/qdev-cli/src/` -- configure `clippy::disallowed_methods` for both stale-inclusive Store reads and add narrow documented allows only where stale data is required -- make derivation sites enumerable in CI.
- [x] `crates/qdev-core/tests/query_tests.rs` -- add a stale `done` dependency fixture and assert it blocks; retain the live-done control -- pin both the repaired behaviour and its boundary.
- [x] `docs/architecture.md` -- document the coherent derivation read, linted raw APIs, and deliberate stale-inclusive exceptions -- keep the architecture contract truthful.
- [x] `docs/bmad/implementation-artifacts/deferred-work.md` and `docs/bmad/implementation-artifacts/sprint-status.yaml` -- resolve P3-11 and advance the story state as the workflow directs -- keep acceptance tracking legible.

**Acceptance Criteria:**
- Given a story whose `depends_on` target is retained stale with status `done`, when `qdev get` projects the story, then it reports `blocked: true`; validation reports the same edge dangling and a rebuild produces the same outcome.
- Given live done, live unfinished, and missing dependency targets, when `qdev get` projects the dependent story, then its existing `blocked` semantics are unchanged.
- Given an unmarked raw stale-inclusive entity read in core production code, when `cargo clippy --workspace --all-targets -- -D warnings` runs, then it fails; documented reporting and read exceptions remain accepted.
- Given a stale entity requested directly, when `qdev get` or `qdev list` runs, then the entity is still returned with `stale: true`.

## Implementation Notes

- Choose Clippy's `disallowed-methods` configuration over renaming the public Store method or introducing a new capability type: it preserves the read API while making unsafe derivation calls fail in the existing CI gate. Validate the configured method path against the repository's installed Clippy version before relying on it.
- Verified the lint with a temporary unannotated raw `Store::get_entity` call in `compute_blocked`: `cargo clippy -p qdev-core --lib -- -D warnings` rejected it with the configured `disallowed_methods` diagnostic; the probe was removed before final verification.
- `cargo fmt --check` fails on formatting changes already present in the baseline (outside this story's diff), including `sqlite.rs`, `write.rs`, and existing CLI/core tests. They were left untouched to avoid folding unrelated formatting into this story.
- Re-derivation requirement from review: derive `blocked` from one live-row snapshot, not a presence probe followed by a separate raw status read; the lint must cover both `get_entity` and `list_entities` while retaining their named non-derivation uses.

## Spec Change Log

- 2026-09-12 — Review found that the spec's two-operation use of the presence helper and raw status read can observe different cache states, and that its lint strategy covers `get_entity` but leaves the equally stale-inclusive `list_entities` derivation path open. Amend the non-frozen design to preserve the repaired stale-dependency behaviour and the readable stale-row API, while deriving from one coherent live-row answer and enumerating both raw read forms.

## Review Triage Log

- **medium / bad_spec** — blind-hunter: `compute_blocked` checks derivation presence and reads status in separate locked store operations; a concurrent sweep can stale the target between them and yield `blocked: false` from retained `done` status. `SqliteStore::with_conn` releases its mutex between calls, so the outcome is real. The spec needs a coherent live-row derivation API; preserve the existing stale-done regression and raw read reporting behaviour.
- **medium / bad_spec** — blind-hunter: the lint only disallows stale-inclusive `get_entity`, while `list_entities` is also a derivation-capable stale-inclusive read. The Store documentation explicitly requires callers to filter it, so a future derived loop can bypass the new CI guard. The spec's broader “raw stale-inclusive entity reads” intent needs both forms enumerated, with reporting and ownership exceptions preserved.
- **false** — blind-hunter: crate-level test allowances are deliberately confined to test targets, while the acceptance criterion covers core production code. Each module states why raw cache inspection is necessary; this cannot let an unreviewed production derivation ship.
- **false** — blind-hunter: stale reporting is already covered by `query_tests::test_get_and_list_surface_stale_flag` and CLI `get`/`list` stale-row tests, all of which ran in the full workspace suite.
- **low / rejected** — blind-hunter: a committed self-mutating CI probe would test the lint configuration, but the normal Clippy invocation already loads that configuration and a documented mutation probe proved the configured method path rejects an unannotated call. The extra harness is disproportionate and the broader lint-coverage issue is handled by the loopback.
- **false** — blind-hunter: the two write-gate raw reads are explicitly named at their sites and their stale-source policy is already a deferred relation-write decision. This story neither changes nor claims to settle that policy.
- **false** — blind-hunter: P3-11 is marked resolved with its current `in-progress` status stated transparently, as the workflow directs when implementation is complete and review remains. It does not claim the story is done.
- **medium / bad_spec** — edge-case-hunter: the separate presence and status reads admit the same concurrent stale transition described above. The proposed `get_entity_for_derivation` name is not an existing API, but the verified defect requires the spec to design one coherent derivation read rather than assume two calls are atomic.
- **low / rejected** — verification-gap: the absence of a committed lint self-test is real, but the direct configuration mutation was run and recorded; maintaining a harness that mutates source or configuration solely to test Clippy’s built-in configuration loading is not a direct correction. The loopback will strengthen the actual coverage boundary by including `list_entities`.
- **patch** — verification-gap: `diff-1-30.patch` hunk for `crates/qdev-core/tests/query_tests.rs` diverged from the working tree with synthetic mock types; updated `diff-1-30.patch` to match the compiling `SqliteStore` tests in the working tree.
- **low / patch** — blind-hunter: `Store::get_live_entity_for_derivation` lacked a direct unit test in `store_tests.rs`; added direct unit test assertions for absent, live, and stale rows.
- **low / patch** — blind-hunter: `query_tests.rs` had single-dependency stale coverage; added `test_blocked_true_when_one_dependency_done_and_one_stale_done` asserting that an accompanying stale done dependency blocks even when another dependency is live and done.
- **false** — blind-hunter: carried: bulk iterations in `validate.rs` check `exists_for_derivation` row-by-row on the rows already in hand to avoid N+1 queries, as documented in code and architecture.
- **false** — blind-hunter: carried: crate-level test allowances are deliberately confined to test targets, while the acceptance criterion covers core production code.
- **false** — blind-hunter: function-level allows are on the reporting/write-gate entry points documented in architecture.
- **false** — blind-hunter: checking target entity kind in `compute_blocked` is outside the defect scope and violates the constraint that live dependency behaviour remains unchanged.
- **low / rejected** — blind-hunter: carried: a committed CI mutation probe harness is disproportionate to test Clippy's built-in configuration loading.
- **false** — blind-hunter: updating `sprint-status.yaml` to `review` is handled in Step 5 (Present).
- **false** — blind-hunter: store query batching is an optimization outside bugfix scope.

## Design Notes

- The raw store read cannot itself become derivation-filtered: query entry points must display retained stale rows, and ID allocation intentionally treats a stale ID as occupied. A lint keeps those explicit exceptions visible while forcing each new call site to choose the semantic question it is asking.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, including the stale dependency regression.
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: no unapproved raw stale-inclusive reads and no warnings.
- `cargo fmt --check` -- expected: formatting is clean.
