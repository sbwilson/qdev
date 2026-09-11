---
title: 'An unchanged file is unchanged, not healthy'
type: 'bugfix'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '96b175b801e78b1ffbac7dfbfd84a04a1c91e75b'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/spec-change-gate-compares-content.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** the sweep treats *content unchanged since the last hydration* as *the file is healthy*.
Its hash-unchanged branch clears every finding for the path and clears the row's `stale` flag —
but the last hydration may have **failed**, in which case the stored hash is the hash of the
broken content. Once cleared, the row is "accounted for", so no later sweep re-parses the file and
the workspace is clean forever:

```
# E1S1.md holds <<<<<<< HEAD
qdev validate   → [error] merge_conflict, exit 1, entities.stale = 1
touch docs/specs/stories/E1S1.md
qdev sync ; qdev validate   → findings: [], exit 0, stale = 0, findings table empty
qdev sync --rebuild ; qdev validate  → [error] merge_conflict, exit 1
```

`qdev doctor` agrees with the false answer. The `schema_violation` variant is worse: the row is
un-staled and served as live, so `qdev get` returns pre-edit content with `"stale": false` for a
file that no longer has a title. This is the epic's load-bearing invariant — a sweep and a rebuild
of the same tree agree — failing in the story written to establish the change gate, and it is
reached by anything that rewrites identical bytes or touches metadata: `touch`, `chmod`,
`git checkout -- .`, `git stash pop`, an editor save-with-no-change, `cp -p`. A CI job that checks
out and runs `qdev validate` is exactly the shape that gets the false pass.

A second face of the same defect: when no `entities` row was retained, the `unaccounted` gate
restores the finding on the *next* sweep, so two identical consecutive `qdev validate` runs over an
unchanged tree disagree — exit 0, then exit 1.

**Approach:** an unchanged hash means the content has not changed; it says nothing about whether
the last attempt to hydrate it succeeded. Where the last attempt did not succeed, the sweep must
re-derive the answer rather than assume a healthy one.

## Boundaries & Constraints

**Always:**
- A sweep leaves the same findings and the same `stale` flags a full rebuild of the same tree
  leaves. That is the invariant; every other rule here serves it.
- `qdev validate` is idempotent on an unchanged tree: two consecutive runs give the same exit code
  and the same findings.
- A finding is cleared only when the condition that produced it has been re-checked — not because
  a stamp moved.
- The recovery this branch was written for keeps working: a file that was unreadable and is
  readable again, with content unchanged, ends the sweep with no `read_error` and no `stale` flag.
- The classification the fix rests on is **enumerable**: adding a seventh finding code must force a
  decision at the site, not inherit a default.

**Never:**
- No change to the change gate's *detection* — `file_change_stamp`, the `max(mtime, ctime)`
  nanosecond stamp and the whole-second "unresolved" rule are story 1-19's and stay as they are.
  This story is only about what the unchanged branch may conclude.
- No new finding codes, and no change to what any existing check reports.
- No full re-parse of an unchanged, healthy file. The warm-sweep budget (AD-6: 30 ms at N=1000)
  must hold, so whatever re-derivation this adds applies only to paths whose last hydration did
  not succeed.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Touch a conflicted file | `E1S1.md` holds conflict markers, then `touch` | `merge_conflict` survives, `stale` stays 1, exit 1 — equal to a rebuild | N/A |
| Touch a schema-invalid file | `title:` removed, then `touch` | `schema_violation` survives, row stays stale, `get` still reports `stale: true` | N/A |
| Chmod-only change | mode changes, content identical | same as above — no finding cleared | N/A |
| Unreadable, then readable again | `chmod 000` then `chmod 644`, content unchanged | `read_error` cleared, `stale` cleared — the recovery this branch exists for | N/A |
| Unreadable over a broken file | schema-invalid file, `chmod 000`, sweep, `chmod 644`, sweep | the `schema_violation` is reported again — the `read_error` that replaced it must not become "clean" | N/A |
| Unchanged and healthy | ordinary file, `touch` | no re-parse, `parsed` unchanged, warm-sweep budget holds | N/A |
| No retained row | hand-authored file that never parsed, then `touch` | the same finding on every run; two consecutive `validate` runs agree | N/A |
| Fixed after being broken | conflict markers removed | finding cleared, `stale` cleared, converges with a rebuild | N/A |
| Sweep vs rebuild, every above state | same tree | identical rows and identical findings, table by table | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:3355-3370` (the sweep's hash-unchanged `else` arm) --
  `clear_findings_for_path` + `unstale_by_source_path` + `upsert_sync_state_row`; the comment above
  it states the read-error rationale that is correct for one case and wrong for the others -- the
  defect, and the prose that must move with it
- `crates/qdev-core/src/store/sqlite.rs:4024` (`record_read_error`) -- **clears the path's findings
  before recording `read_error`**, which is why classifying codes is not enough: a
  `schema_violation` that a `read_error` replaced is not in the table to be preserved. Read this
  before choosing a mechanism -- the trap
- `crates/qdev-core/src/store/sqlite.rs:3243-3247` (the `unaccounted` gate) -- why the false clean
  state is permanent when a row was retained, and why it is merely alternating when none was --
  the other face of the same defect
- `crates/qdev-core/src/store/sqlite.rs:4682` (`hydrate_markdown_file`) -- the re-derivation to
  reuse; it already clears the path's findings, re-parses and sets or clears `stale` correctly --
  reuse, do not reimplement
- `crates/qdev-core/src/store/sqlite.rs:4273` (`unstale_by_source_path`) / `:4291`
  (`flag_stale_by_source_path`) -- the two sides of the stale flag, both keyed on `source_path`
- `crates/qdev-core/src/store/sqlite.rs:3980` (`upsert_sync_state_row`) -- the stamp/size refresh
  that must still happen however the branch is restructured, or the file re-reads every sweep
- `crates/qdev-core/tests/sweep_tests.rs:250` (`test_touch_same_content_no_reparsed`) -- pins the
  healthy case and must keep passing; the missing fixture is the same test over a *broken* file
- `crates/qdev-core/tests/sweep_tests.rs:1624` (`test_benchmark_warm_sweep_bound`) -- the AD-6
  budget; run it on an idle machine, since this story adds work to the unchanged path
- `crates/qdev-core/tests/sweep_tests.rs:89` (`dump_tables`) and the `ALL_TABLE_NAMES` comparison
  loops -- the sweep-equals-rebuild assertion to extend to every matrix state

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- the hash-unchanged branch stops concluding "healthy":
  a path whose last hydration did not succeed is re-derived rather than cleared -- the fix
- [x] `crates/qdev-core/src/store/sqlite.rs` -- make the "did the last hydration succeed?" question
  answerable at one site, with no wildcard arm, so a seventh finding code forces a decision --
  the enumeration this story owes
- [x] `crates/qdev-core/src/store/sqlite.rs` -- rewrite the branch comment to say what the code now
  does and why the read-error recovery still works -- prose in step with mechanism
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- cover every matrix row, each mutation-verified;
  include the sweep-equals-rebuild comparison and the idempotence pair -- verification
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- confirm the warm-sweep benchmark still holds on an
  idle machine, and say in the notes what it measured -- AD-6
- [x] `docs/architecture.md` -- state that an unchanged file is not thereby a healthy one, next to
  the change-gate description -- the rule belongs where the gate is documented

**Acceptance Criteria:**
- Given a file carrying a `merge_conflict` or `schema_violation`, when its stamp changes but its
  content does not, then the finding and the stale flag survive, and `qdev validate` exits 1.
- Given a file that was unreadable and is readable again with unchanged content, when a sweep runs,
  then its `read_error` and stale flag are cleared.
- Given any state in the matrix, when a sweep and a full rebuild are compared table by table, then
  they are identical.
- Given an unchanged tree, when `qdev validate` runs twice, then both runs report the same findings
  and the same exit code.
- Given 1000 unchanged healthy files, when a warm sweep runs on an idle machine, then it stays
  within the AD-6 budget.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo fmt --check` are clean.

## Implementation Notes

- **The mechanism is one extra disjunct on the existing hydrate condition, not a new branch.** The
  sweep already loads `finding_paths` — every path carrying a code that explains why it has no row
  — for the `unaccounted` gate. That set *is* "the paths whose last hydration did not succeed", so
  `last_hydration_failed` is answered from state already in memory, with no extra query and no
  extra I/O, and the path simply joins `hash_changed || is_dirty || unaccounted` in taking the
  re-parse. `hydrate_markdown_file` then produces exactly what a rebuild produces, which is the
  property being restored. Nothing was reimplemented.
- **The enumeration is `FindingCode`** (`sqlite.rs`, beside `record_finding`): the six codes as an
  enum, with `means_hydration_failed` the single classification site — an exhaustive match, no
  wildcard arm, so a seventh code does not compile until its author picks a side. `as_str` and
  `next_variant` are exhaustive too, but the *enumeration* is not compiler-forced: an author can
  answer `next_variant` with `NewCode => None` and leave the code unreachable from `all()`, and
  hence out of both SQL lists. That half is pinned by `test_finding_code_enumeration`
  (`sqlite.rs`), which asserts `all()` yields all six, that the two `sql_in_list` partitions are
  disjoint and jointly cover them, and that every wire value is the one already on disk. Both the
  code doc comment and `architecture.md` say exactly this rather than claiming more. Both
  SQL code lists (the `finding_paths` scan and `validate_relations_graph`'s delete) are now
  generated from the one classification as complements of each other, so they cannot drift; the
  literals are `[a-z_]` compile-time constants, so splicing them into `IN (…)` injects nothing.
  `record_finding` takes a `FindingCode` rather than a `&str`, so a new code cannot be recorded
  without being declared.
- **A latent convergence bug found on the way.** Scratch, evidence and config files record no
  findings of their own, so the only finding such a path can carry is a `read_error` — and nothing
  cleared it when the file was read again after a *content* change (the old clear lived only on the
  hash-unchanged branch). A rebuild starts from an empty `findings` table, so the two diverged.
  Those three role arms now clear the path's findings before re-hydrating, and
  `test_unreadable_scratch_and_evidence_files_record_read_error_on_both_paths` now carries the
  recovery through to the end: both files readable again with new content, no findings left for
  either path, and `assert_sweep_equals_rebuild`.
- **The healthy path is untouched.** Re-derivation applies only to paths carrying a
  hydration-failure finding, and a healthy workspace has none. Measured: warm sweep at N=1000,
  median 7 ms, max 8 ms over 25 runs (budget 30 ms) — within noise of the figure recorded before
  this story.
- **Mutation-verified.** Deleting the `|| last_hydration_failed` disjunct fails five of the new
  tests; moving `SchemaViolation` to the "hydration succeeded" side of the classification fails
  five tests, two of them pre-existing ones. Dropping the three non-Markdown clears fails the
  scratch/evidence recovery test, and unlinking a variant from `next_variant` fails
  `test_finding_code_enumeration`.
- The unreadable-then-readable-again recovery now reaches its result by re-parsing instead of by
  assuming the file is fine: the outcome is identical (finding and stale flag cleared) and the
  existing round-trip test passes unchanged.

## Spec Change Log

- 2026-09-11 — Post-review pass over the story's own implementation (adversarial, edge-case and
  verification-gap lenses). Six findings acted on, none requiring a change to the frozen intent:
  the Config role's clear and `apply_relation_change`'s path spelling gained the tests they
  lacked; separator normalization moved into `workspace_rel_path`, which walks components rather
  than replacing backslashes (a backslash is legal in a Unix file name); `SweepSummary` gained
  `retained`, so a file read and tried that produced no row is counted rather than falling out of
  the totals; `FindingCode`'s doc no longer claims a closed set `Store::upsert_finding` can
  bypass, and its test asserts the classification as data instead of as rendered SQL; `found_at`'s
  meaning is now stated where it is defined. One divergence found and deliberately not fixed —
  `entities.updated_at` after a touch — is in `deferred-work.md` with the reasoning.
- 2026-09-11 — Implemented. No change to the frozen intent, and no deviation from it. One
  in-scope repair beyond the stated tasks: the non-Markdown role arms now clear a stale
  `read_error` when they re-hydrate, without which the fix would have regressed that recovery.
- 2026-09-11 — Created from the pass-3 acceptance gate (P3-1), second of the seven blocking stories.

## Review Triage Log

- **medium / patch** — `FindingCode::all()` is generated from `next_variant`, whose exhaustive match forces an *arm* for a seventh variant but not a *pointer to* it: `NewCode => None` compiles while `DependencyCycle => None` stays, so `all()` omits the new code from both SQL lists and the hash-unchanged branch stops seeing it as a hydration failure. Verified by reading the three arms; the doc comment, `architecture.md` and the Implementation Notes all assert a guarantee the construction does not give. Patched: the claim corrected to what holds, plus a unit test pinning the enumeration and the partition.
- **medium / patch** — the `clear_findings_for_path` added to the Scratch/Evidence/Config arms is unpinned. Verified: `test_unreadable_scratch_and_evidence_files_record_read_error_on_both_paths` (sweep_tests.rs:2123) stops before recovery, and all seven new tests use `docs/specs/stories/*.md`, so deleting the three clears fails nothing. Filed pre-verified by the verification-gap layer. Patched with a recovery assertion for the non-Markdown roles.
- **low / rejected** — no direct test over `FindingCode` itself (`all()` count, partition, wire strings). Real but subsumed by the first entry's patch, which adds exactly that test.
- **low / rejected** — the complement side of the classification (a file carrying only `dangling_relation` etc., touched, not re-parsed) has no dedicated test. `validate_relations_graph` deletes and re-derives those three codes on every sweep unconditionally (`sqlite.rs:4196`), and that re-derivation is pinned at sweep_tests.rs:873; the behaviour cannot silently regress.
- **low / rejected** — `clear_findings_for_path` now runs unconditionally on every changed scratch/evidence/config file rather than only where a finding exists. A single indexed `DELETE` per changed non-entity file; gating it on `finding_paths` adds a branch to save nothing measurable.
- **low / rejected** — `sql_in_list` rebuilds its `IN (…)` string on each call. Twice per sweep, six short literals; a `LazyLock` is more machinery than the allocation costs.
- **low / rejected** — `FindingCode` has no `from_str`, so the read path (`list_findings`, `FindingRecord.code`) is still `String`. A real design asymmetry with no named harm in this change: nothing in the diff reads a code back and branches on it.
- **false** — "the central fix is Unix-only". `test_touch_over_merge_conflict_keeps_the_finding_and_the_stale_flag` and `test_touch_over_schema_violation_keeps_the_finding_and_the_row_stale` carry no `#[cfg(unix)]`; only the chmod-driven rows do, and those need `chmod`.
- **low / rejected** — the two `make_unreadable_in_place` bailouts return without asserting when run as root. Pre-existing helper and pre-existing convention in this suite (the same bailout guards the older read-error tests); changing how skips signal is a suite-wide decision, not this story's.
- **low / rejected** — a file re-parsed through the new disjunct that stays broken is counted in neither `parsed` nor `unchanged`. Verified at `sqlite.rs:3306-3346`: that is the pre-existing accounting for every broken file that hydrates, unchanged by this story — only the population reaching it grew.
- **false** — `sql_in_list` could emit `IN ()` if every code fell on one side. Both sides are non-empty by construction today, and the first entry's new test pins that; SQLite accepts an empty `IN ()` as false rather than failing to prepare.
- **false** — "a path previously hydrated as Markdown, now classified a non-entity role, keeps a stale row". `SweepFileRole` is decided by the path's directory, so a path cannot change role without becoming a different path.
- **false** — the `read_error`-over-`schema_violation` test compares only `list_findings` rather than every table. That is deliberate and commented: a row retained stale over a file that previously parsed is the epic's documented divergence from a rebuild, asserted on its own in `test_unreadable_previously_parsed_file_retains_its_rows_stale`.
- **low / patched** — `sprint-status.yaml` lost the file's trailing-comment column alignment on the 1-25 line. Direct correction; realigned. (The status difference between the spec and the sprint file is the workflow's own sequencing, not a defect.)

## Design Notes

- **Why classifying finding codes is not enough.** The obvious fix — clear only `read_error`, keep
  the parse-failure codes — fails a real sequence: a schema-invalid file that then becomes
  unreadable has its `schema_violation` *replaced* by `read_error` (`record_read_error` clears
  first), so when permissions are restored with the content unchanged there is no parse-failure
  finding left to preserve, and the workspace goes clean over a file that is still broken. That
  matrix row exists to catch this.
- **The mechanism that follows from it** is to re-derive rather than to reason: when the last
  hydration of a path did not succeed, re-parse it — `hydrate_markdown_file` already produces
  exactly what a rebuild would, which is the property being restored. The set of such paths is
  small by construction (a healthy workspace has none), so the warm-sweep budget is unaffected,
  and "did the last hydration succeed?" is answerable from state the sweep already loads.
- **Enumeration.** The rule is only as good as the next finding code's author remembering it. An
  exhaustive `match` with no wildcard arm — over a finding-code enum, or a single classification
  function the compiler forces to cover every variant — turns that into a compile error. Prefer
  the form the compiler checks over a runtime list.
- The healthy path must stay free: an unchanged, successfully hydrated file is still just a stamp
  and size refresh, with no read and no parse.

## Verification

**Measured:** `cargo test --workspace` clean (57 in `sweep_tests`, 7 new); `cargo clippy
--workspace --all-targets -- -D warnings` silent; `cargo fmt --check` silent. Warm-sweep
benchmark, release, N=1000, 25 runs: **median 7 ms, max 8 ms** against a 30 ms budget and a 500 ms
ceiling. All three manual checks run against the built binary and behaved as specified: the
touched conflicted file keeps `merge_conflict` and exit 1 and agrees with `sync --rebuild`; the
`chmod 000` → `chmod 644` round trip over a title-less file reports `schema_violation` again rather
than going clean, with `qdev get story E1S1` still saying `stale: true`; two consecutive
`qdev validate` runs over the unchanged broken tree are identical.

**Commands:**
- `cargo test --workspace` -- expected: clean
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: no output
- `cargo fmt --check` -- expected: no output

**Manual checks:**
- Plant conflict markers, sweep, `touch`, sweep: `qdev validate` still exits 1 with
  `merge_conflict`, and `sync --rebuild` then `validate` agrees.
- Remove a `title:`, sweep, `chmod 000`, sweep, `chmod 644`, sweep: `schema_violation` is reported
  again rather than the workspace going clean.
- `qdev validate` twice over an unchanged broken tree: identical output both times.
