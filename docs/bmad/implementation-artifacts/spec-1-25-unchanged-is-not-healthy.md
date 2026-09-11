---
title: 'An unchanged file is unchanged, not healthy'
type: 'bugfix'
created: '2026-09-11'
status: 'draft'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '32c94bb391c01e299bdf8fcca14f355de7b8b67a'
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
- [ ] `crates/qdev-core/src/store/sqlite.rs` -- the hash-unchanged branch stops concluding "healthy":
  a path whose last hydration did not succeed is re-derived rather than cleared -- the fix
- [ ] `crates/qdev-core/src/store/sqlite.rs` -- make the "did the last hydration succeed?" question
  answerable at one site, with no wildcard arm, so a seventh finding code forces a decision --
  the enumeration this story owes
- [ ] `crates/qdev-core/src/store/sqlite.rs` -- rewrite the branch comment to say what the code now
  does and why the read-error recovery still works -- prose in step with mechanism
- [ ] `crates/qdev-core/tests/sweep_tests.rs` -- cover every matrix row, each mutation-verified;
  include the sweep-equals-rebuild comparison and the idempotence pair -- verification
- [ ] `crates/qdev-core/tests/sweep_tests.rs` -- confirm the warm-sweep benchmark still holds on an
  idle machine, and say in the notes what it measured -- AD-6
- [ ] `docs/architecture.md` -- state that an unchanged file is not thereby a healthy one, next to
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

## Spec Change Log

- 2026-09-11 — Created from the pass-3 acceptance gate (P3-1), second of the seven blocking stories.

## Review Triage Log

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
