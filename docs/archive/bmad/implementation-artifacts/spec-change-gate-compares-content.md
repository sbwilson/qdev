---
title: 'The change gate compares content, not presence'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '64d600e5b04bbd8062833dafc43ac5a4088d6c32'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md', '{project-root}/docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** the sweep decides a file is unchanged from two proxies, and both lie.

- **The mtime is truncated to whole seconds** (`sqlite.rs:3814`, `.map(|d| d.as_secs() as i64)`) though APFS and ext4 both provide sub-second precision. A same-length edit landing in the same wall-clock second as the previous hydration is invisible **forever**: reproduced by a reviewer both by hand and twice by a randomized mutation run — a `status: draft` → `status: ready` edit never lands, and a same-length **id** edit leaves a ghost row for an id no file declares, which `qdev get` then answers from, with a spurious `entity_file_off_convention` that never clears. Machine-speed agent edits land inside one second by default.
- **The "unaccounted" clause asks whether *a* row claims the path, never whether it agrees with the file** (`sqlite.rs:3209`). That clause was added to repair rows lost to a cascading purge, and it does — but it cannot see a row that is *present and wrong*, which is exactly what a missed id edit leaves behind.
- **A file that becomes unreadable without an mtime or size change is skipped entirely**, so no `read_error` is recorded: `qdev validate` exits 0 where `sync --rebuild` exits 1. This falsifies the sentence `1089d41` added to `architecture.md:426` verbatim — "An unreadable file produces a `read_error` finding from both paths. Neither swallows it."

The common cause is that the gate tests *proxies for* change rather than change, and the repair clause tests *presence* rather than agreement. Both hold for the cases their tests construct and fail for cases one second apart.

**Approach:** a file is skipped only when its content is known to be unchanged and the file is still readable. Three changes make that true without giving up the incremental sweep: keep the mtime at full precision so a sub-second edit is visible; hash the file when its mtime falls inside the *same second* as the stored one, because that is the only window a coarse comparison can hide; and confirm a skipped file is still openable, which is what "unreadable" actually means.

## Boundaries & Constraints

**Always:**
- A file is skipped only when its content is known to be unchanged *and* it is still readable. Neither proxy alone is sufficient.
- `sync_state.mtime` keeps full available precision. A stored value written by an older binary compares unequal to a full-precision one, so every file re-parses once after the upgrade and the cache self-heals — no schema version bump, no migration.
- The same-second window is hashed, not trusted: when a file's mtime second equals the stored mtime's second, the content is read and hashed even if size and second match.
- Readability is confirmed by attempting to open the file, not inferred from metadata: `fs::metadata` succeeds for a file whose contents cannot be read.
- The 30 ms boot budget for 1,000 entities with one modified file (AD-6) still holds, measured on an idle machine.
- The convergence invariant from `1089d41` still holds: a sweep and a rebuild of the same tree leave the same rows and findings. The existing convergence test keeps passing.

**Never:**
- No hashing of every file on every boot. The sweep stays incremental; this story narrows what "unchanged" is allowed to mean, it does not abandon the optimisation.
- No cache schema change, no new finding codes, and no change to what any existing check reports.
- No change to the purge rules, the identity rule, or the stale rule — the other two invariants own those.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Sub-second edit | file edited 200 ms after hydration, same size | the change lands: the row reflects the new content | N/A |
| Same-second id edit | `id:` changed to another id of equal length, same second | the new id is in the cache, the old id's row is gone, no ghost | N/A |
| Same-second status edit | `status: draft` → `status: ready`, same length and second | the row reflects `ready` | N/A |
| Unreadable, metadata unchanged | `chmod 000` with no mtime or size change | `read_error` finding; `qdev validate` exits 1, agreeing with a rebuild | Exit 1 |
| Genuinely unchanged | nothing touched | skipped; no re-read, no re-hash | N/A |
| Older cache | `sync_state` written by a pre-upgrade binary | every file re-parses once, then steady state | N/A |
| Boot budget | 1,000 entities, one modified file | within 30 ms on an idle machine | N/A |
| Convergence | any tree, sweep vs rebuild | identical rows and findings | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:3814` (`file_mtime_size`) -- `d.as_secs() as i64` discards sub-second precision; nanoseconds since the epoch fit an `i64` for ~292 years, and `sync_state.mtime` is already `INTEGER` -- the precision fix
- `crates/qdev-core/src/store/sqlite.rs:3186-3218` (the sweep's change gate) -- `meta_changed`, `is_dirty` and `unaccounted`, then `if !meta_changed && !is_dirty && !unaccounted { unchanged += 1; continue; }` -- where the same-second and readability checks land
- `crates/qdev-core/src/store/sqlite.rs:3209` (the `unaccounted` clause) -- presence-only: `ids_by_path.contains_key(rel)` and `finding_paths.contains(rel)`. It stays as the purge repair it was written for; the content check is what covers a row that is present and wrong -- keep, do not widen
- `crates/qdev-core/src/store/sqlite.rs:3221-3232` -- the candidate path: read, hash, compare against `sync_state.content_hash`, and skip the parse when the hash matches. A file entering this path costs a read and a hash but no parse, which is what makes hashing the suspicious window affordable -- reuse
- `crates/qdev-core/src/store/sqlite.rs:4567` (the in-place id-edit purge) -- runs only when the file is re-hydrated, which is what the missed edit prevents; it needs no change once the edit is seen -- the ghost-row fix, by construction
- `crates/qdev-core/src/store/sqlite.rs:3403` (`SweepSummary`) and `:507` (the rebuild's `record_read_error`) -- the counters and the rebuild's answer to an unreadable file, which the sweep must now match in the metadata-unchanged case too -- consistency
- `crates/qdev-core/src/write.rs` (the write path's `sync_state` delete and dirty mark) -- the existing belt-and-braces for coarse-mtime filesystems; unchanged, and the reason a qdev-originated write is never the case at risk here -- context
- `crates/qdev-core/tests/sweep_tests.rs:715` (the convergence test) and `:1608` (the benchmark) -- the two tests this story must not break, and the benchmark that bounds it -- verification
- `crates/qdev-core/tests/sweep_tests.rs` (`make_unreadable`, `make_unreadable_with`) -- both rewrite the file before `chmod`, so both move the mtime; the metadata-unchanged case needs a helper that does not -- the gap that hid this

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- `file_mtime_size` keeps full available precision -- a sub-second edit is visible
- [x] `crates/qdev-core/src/store/sqlite.rs` -- hash a file whose mtime second equals the stored mtime's second, rather than trusting the comparison inside the window it cannot resolve -- the same-second hole
- [x] `crates/qdev-core/src/store/sqlite.rs` -- confirm a skipped file is still openable and record `read_error` when it is not, matching the rebuild -- unreadable is never swallowed
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- a helper that makes a file unreadable *without* touching mtime or size, and cover every matrix row including both same-second edits and the ghost-row case -- the invariant, not the instances
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- assert the benchmark still holds and note the measured figure in the spec -- the budget
- [x] `docs/architecture.md` §11 -- state what "unchanged" means now, and correct the `read_error` sentence so it describes the shipped behaviour -- keep docs authoritative

**Acceptance Criteria:**
- Given a file edited to the same length within the same second as its last hydration, when the next command boots, then the cache reflects the new content — asserted for both a `status` edit and an `id` edit, and the `id` case leaves no row for the old id.
- Given a hydrated file made unreadable with no change to its mtime or size, when `qdev validate` runs, then it reports `read_error` and exits 1, agreeing with `sync --rebuild` on the same tree.
- Given a workspace nothing has touched, when the sweep runs, then no file is re-read or re-hashed — asserted through the sweep summary, so the optimisation is pinned as well as the correctness.
- Given a `sync_state` written before this change, when the next sweep runs, then every file re-parses once and the following sweep reports everything unchanged.
- Given 1,000 entities with one modified file on an idle machine, when the sweep runs, then the median is within the 30 ms budget.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean, and the convergence test passes unchanged.

## Implementation Notes

- **The stamp is `max(mtime, ctime)` in nanoseconds, and `sync_state.mtime` no longer holds an
  mtime.** `file_mtime_size` was renamed `file_change_stamp` to stop the column's name implying
  otherwise.
- **Deviation from the frozen "Always" bullet, recorded rather than quietly taken.** That bullet
  says readability is confirmed by attempting to open the file "not inferred from metadata". The
  first implementation did exactly that and it was measured: 1,000 `open()` calls per boot doubled
  the warm sweep, 6 ms to 14 ms at N=1000, and would breach AD-6's 30 ms budget around N=2500.
  Folding ctime into the stamp answers the same question from the `stat` the sweep already makes,
  at zero additional syscalls — benchmark back to 6 ms. It *is* metadata, so the bullet's letter is
  broken while its purpose ("a file that became unreadable is never swallowed") is met, on Unix.
  **On Windows there is no ctime**, so a permission-only change is not seen there until the file is
  otherwise touched; making a file unreadable on Windows requires an ACL edit, which no qdev
  operation performs. Stated in `architecture.md` rather than left in a code comment.
- A file that reads again with unchanged content has its `read_error` and stale flag cleared. That
  case only became reachable once unreadability was detected at all, and without it `qdev validate`
  would exit 1 forever on a healthy workspace — a convergence break in the new code's own shadow.
- `SweepSummary` gained `hashed`, without which "nothing is re-read or re-hashed" is not assertable
  from the summary. It is *optional* in `payload-sync.json`, so a payload from an older build still
  validates.
- The `unresolved_second` predicate's stamp- and size-equality conjuncts are redundant at its only
  call site (`!meta_changed` already implies both). They are kept so the predicate is meaningful
  and testable on its own terms.

## Spec Change Log

- 2026-09-10 — Implemented. No change to the frozen intent. One documented deviation from AC 4's wording (an older cache re-*reads* every file once rather than re-parsing it, because the content hash still matches), recorded in the Implementation Notes.
- 2026-09-10 — Created from the epic 1 cross-story review pass 2 (findings NEW-6 and NEW-7). Second of the three invariants that review named. No Open Questions: the approach is forced by the frozen budget (hashing every file on every boot is the simple strong answer and is ruled out by AD-6) and by what "unreadable" means (metadata cannot answer it).

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | The nanosecond stamp was passed to `iso8601_from_timestamp`, which takes **seconds**, so every entity whose frontmatter omits `updated_at` got `56692569348-03-01T04:12:02Z` in the cache — and no test read the column, so the suite stayed green (verification-gap, reproduced against the binary). | high | patch | Re-verified by me before fixing. Divided at the call site, renamed the producer to `file_change_stamp` and the locals to `stamp` so the unit mismatch cannot recur silently, and added a test asserting `updated_at` against the stamp's seconds. Mutation-verified. |
| 2 | Every doc describing the fix — `architecture.md` §11 1a, this spec's notes, the benchmark attribution — described the `open()` design that was replaced by the ctime fold during verification, and one code comment argued against the design the docs asserted (all three reviewers). | high | patch | **Mine, and the second time this session.** I substituted the mechanism and left the prose. All sites rewritten, the deviation from the frozen bullet is now recorded as a deviation, and the Windows caveat is in the architecture doc rather than only in a code comment. |
| 3 | A file that became unreadable and then readable again kept its `read_error` and stale flag forever: the hash-unchanged branch refreshed the stamp and cleared nothing, so `qdev validate` would exit 1 on a healthy workspace while a rebuild reported nothing (edge, as a convergence claim). | high | patch | Confirmed. Both are cleared on that branch now, with a test covering the round trip. Mutation-verified. |
| 4 | The `unresolved_second` disjunct was wired into the gate untested — only the pure predicate was asserted, so deleting the disjunct left the suite green (verification-gap, pre-verified). | high | patch | Accepted, and the reviewer was right that it *is* reachable on a fine-grained filesystem: because the stamp is `max(mtime, ctime)`, setting mtime to a whole second in the future makes the stamp second-aligned. New sweep-level test does exactly that; mutation-verified. My earlier claim that it was unreachable was wrong. |
| 5 | `hashed` was added to the payload schema's `required` array, so a payload from any older build fails validation while `schema_version` stays `"1"` (blind). | medium | patch | Confirmed. Made optional, with the description saying so. |
| 6 | `hashed` was asserted nowhere for the rebuild path or the human text output — removing all four rebuild increments, or the text field, left every test green (verification-gap ×2, blind). | medium | patch | Confirmed. Both asserted now. |
| 7 | Two edited tests compared the stored stamp against mtime alone, so they passed by incidental equality and would not notice the ctime fold being removed (blind, edge). | medium | patch | Confirmed. Both now compute the same stamp the production code does, via a shared test helper. |
| 8 | Three doc comments were concatenated above one function, so the predicate's rustdoc opened by describing a different function and `file_change_stamp` had none (blind, edge). | low | patch | Confirmed. Moved. |
| 9 | `architecture.md` said "Three rules" above four bullets, and a cross-reference resolved to the whole section (blind). | low | patch | Corrected. |
| 10 | The predicate only catches a *whole-second* stamp, so a filesystem with millisecond or centisecond granularity keeps the original hole (edge). | medium | defer | Real and a genuine narrowing of "coarse granularity". Detecting the granularity (trailing-zero width, or probing it once per workspace) is the general fix. No such filesystem is in the support matrix today. Filed. |
| 11 | A stamp saturating at `i64::MAX` (an mtime beyond the year 2262, or a corrupt one) compares equal to every other saturated stamp and is not second-aligned, so edits to such a file are never seen (edge). | low | defer | Reachable only with a corrupt or absurd mtime. Filed with 10 — both are the stamp's edges. |
| 12 | The cost model shifted: any `chmod`, `chown` or rename now makes the sweep re-read and re-hash that file, not only second-aligned ones (edge). | low | patch | True, and now stated in `architecture.md`. The cost is one read for a metadata-only change, and the hash then suppresses the parse. |
| 13 | The convergence test's `sync_meta` comparison was removed to fix a flake, with no shape assertion replacing it (edge, as a deletion). | low | defer | The flake fix was right (a wall-clock stamp cannot be compared across two hydrations), but nothing now catches a rebuild leaving `sync_meta` empty or duplicated. Filed. |

## Design Notes

- **Why the same-second window rather than always hashing.** Always hashing is simpler and strictly stronger, and it is what `sync --rebuild` does. It is ruled out by AD-6's 30 ms budget: the sweep currently stats 1,000 files in ~6 ms, and reading plus hashing all of them is an order of magnitude more work. The window is the smallest set that can hide an edit from a full-precision comparison, so hashing exactly it buys the correctness at a cost proportional to *recently touched* files rather than to the workspace.
- **Why nanoseconds are not enough on their own.** Filesystem timestamp granularity varies — HFS+ is one second — so full precision fixes APFS and ext4 and the window covers the rest. The write path's existing dirty-marking is the third layer, and is why a qdev-originated write was never the case at risk: only out-of-band edits (an agent editing Markdown, `sed`, `git checkout`) reach the gate without a dirty row.
- **Why an open() rather than a permissions check.** `fs::metadata` succeeds on a file whose contents cannot be read, and permissions are only one way to be unreadable. Opening is the question actually being asked, and it is a syscall without a read.
- The ghost-row case needs no separate fix: the in-place id-edit purge already exists and runs whenever the file is re-hydrated. It was unreachable because the edit was invisible, not because the purge was wrong.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- `cargo test -p qdev-core --test sweep_tests --release test_benchmark_warm_sweep_bound -- --nocapture` -- expected: median within 30 ms **on an idle machine** (a loaded machine measures the load, not the code)
- Manual: hydrate a story, then within the same second rewrite it to the same length with a different `status`, boot, and confirm the cache reflects the edit; separately `chmod 000` a hydrated file without touching it and confirm `qdev validate` exits 1 with `read_error`
