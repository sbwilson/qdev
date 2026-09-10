---
title: 'The hex allocators ask the one id-in-use rule, and both guarded invariants get a test'
type: 'refactor'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 1
baseline_commit: 'c9fefb9375fa4ad096ee7d51f61b5d04c90001ac'
context: [ '{project-root}/docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md', '{project-root}/docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Epic 1's two remaining acceptance blockers, both filed with `Review by: before epic 1 acceptance`.

- **A second allocator still keeps its own answer to "is this id taken".** `spec-one-id-in-use-rule.md` made `validate::ids_in_use` the single authority — declared ∪ carried ∪ cached, recursive over both trees, extension matched case-insensitively — and pointed story allocation at it. The hex allocators (`DW-`, `DEC-`) were left probing one flat directory with `std::fs::read_dir` and case-sensitive prefix matching (`id::hex_id_collides`). The invariant is not established while a second allocator disagrees: the same divergence that had `create story` minting ids other files already carried is still open for these two, and every hole that story closed — a nested file, a `.MD` extension, a file whose frontmatter will not parse, an id living only in the cache — is a hole here.
- **The two invariants epic 1 acceptance now rests on are each protected by a comment rather than a test.** `ids_in_use` uses the *unfiltered* `list_entities` deliberately, because a stale row's id is still taken; nothing pins it, and the architecture text ("every derivation site asks the helper") makes adopting the derivation helper here look like a consistency fix. And nothing asserts that repeated sweeps over a duplicate-id pair settle on the same winner a full rebuild picks — the property that makes the convergence invariant hold across passes was verified by hand, three sweeps at a terminal, never by a test.

**Approach:** delete the private collision scan and give the hex allocators the same id space every other allocator uses, then pin both invariants with tests that fail when the mechanism is reverted.

## Boundaries & Constraints

**Always:**
- One answer to "which ids does this workspace own", asked through `ids_in_use`. After this story no allocator keeps a scan of its own.
- The hex allocators keep their *strategy*: random hex, 4 characters growing to 6 then 8 on collision, then bounded retries. Only the collision oracle changes. `DW-`/`DEC-` ids are not sequential and must not become sequential.
- An id is owned workspace-wide, not per-directory. A `DW-` id declared by a file outside `<state_dir>/dw/` is taken.
- Case-insensitive comparison, because the union's members disagree on case: `DW-`/`DEC-` hashes are canonically lowercase, and a declared id is whatever the frontmatter says verbatim.
- Each new test fails when the specific mechanism it pins is reverted, demonstrated by running that reversion — not asserted.

**Never:**
- No change to what `ids_in_use` itself answers. The stale-row test pins today's behaviour; it does not adjust it.
- No new `qdev create dw` / `qdev create decision` command. These allocators have no production caller yet, which is why their signatures are free to change — that is scope for epic 2 story 2.8.
- No change to the sweep's duplicate-id resolution. The convergence tests pin the winner rule that exists (`sorted-last path owns the row`); they do not propose a different one.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Hex id declared by a nested file | `DW-7f3a` declared in `<state_dir>/dw/archive/old.md` | that hash is taken; allocation returns a different one | N/A |
| Hex id carried by a `.MD` name | `<state_dir>/dw/DW-7f3a.MD` | taken (hydration reads it, so allocation must too) | N/A |
| Hex id in an unparseable file's name | `DW-7f3a-notes.md` whose frontmatter will not parse | taken via the carried half | N/A |
| Hex id only in the cache | store holds `DEC-1234`, no file on disk | taken; `store: None` falls back to the filesystem halves | N/A |
| Hex id declared mis-cased | frontmatter `id: DW-7F3A` | taken; comparison is case-insensitive | N/A |
| Hex id outside its own directory | `DW-7f3a` declared under `<specs_dir>/stories/` | taken — ownership is workspace-wide | N/A |
| Length growth still works | every 4-char hash the rng yields is taken | grows to 6, then 8, then bounded retries | returns the last candidate rather than looping forever |
| Stale row's id | cache holds a stale `E1S1`, no readable declaration on disk | `ids_in_use` contains `E1S1`; allocation returns `E1S2` | N/A |
| Standing duplicate pair, repeated sweeps | two files declaring one id, swept three times with no edits | same winner every pass, and the winner a rebuild picks | N/A |
| Three files declaring one id | `X.md`, `X-b.md`, `X-c.md` | sorted-last owns the row, stable across sweeps and equal to a rebuild | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/id.rs:569` (`hex_id_collides`) -- the private scan: one `read_dir`, `starts_with` on `{prefix}-{hash}` with `-`/`_`/`.` separators, case-sensitive -- what goes
- `crates/qdev-core/src/id.rs:591` (`allocate_hex_id_with_rng`) -- the 4→6→8 growth plus 100 bounded retries; keep the strategy, replace its oracle. Takes `workspace_root` + `rel_dir` today; it should take the id set and stop touching the filesystem -- the change
- `crates/qdev-core/src/id.rs:632-690` (`allocate_deferred_work_id{,_with_rng,_in_with_rng}`, `allocate_decision_id{,_with_rng,_in_with_rng}`) -- six public entry points, all six re-exported from `lib.rs:45-47`; no production caller (only `crates/qdev-core/tests/id_tests.rs`), so signatures may take a store and return `Result` -- the API
- `crates/qdev-core/src/id.rs:475` (`allocate_next_story_id_in`) -- the template to follow: `ids_in_use(workspace_root, storage, store)?` first, then a strategy over the set -- reuse, do not re-derive
- `crates/qdev-core/src/validate.rs:165` (`ids_in_use`) / `:177` (`ids_in_use_from_scan`) -- the authority, and the deliberate unfiltered `list_entities` at `:186-195` whose comment is all that protects the stale exception -- what the new test pins
- `crates/qdev-core/tests/validate_tests.rs:1046` (`test_ids_in_use_includes_cache_only_ids`) -- the in-memory-store pattern to copy, including the local `entity()` builder at `:15` (its `stale` field is persisted by `upsert_entity`) -- the fixture shape
- `crates/qdev-core/src/store/sqlite.rs:3243` (the sweep's `unaccounted` gate) and `:3298-3325` (the in-memory `path_of_id`/`ids_by_path` mirror) -- why the sorted-last winner is stable: a displaced file has no row and no explanatory finding, so it is re-parsed every pass and takes the row back. No persisted marker does this -- the mechanism the convergence test pins
- `crates/qdev-core/tests/sweep_tests.rs:692` (`test_rebuild_and_sweep_findings_equal`), `:925` (`test_deleting_the_duplicate_that_owned_the_row_reparses_the_survivor`) -- existing duplicate coverage: each does exactly **one** sweep, and every fixture is a *pair*. Helpers to reuse: `dump_tables` (`:89`, drops `findings.found_at`), `write_at` (`:171`), `story_md` (`:41`), `make_workspace` (`:71`), `count_rows` (`:177`); `sync_meta` must be skipped in any table-by-table comparison (`:774`) -- the gap and its tools

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/id.rs` -- delete `hex_id_collides`; `allocate_hex_id_with_rng` takes the in-use id set and tests membership of `{prefix}-{hash}` case-insensitively -- one oracle, and a pure function
- [x] `crates/qdev-core/src/id.rs` -- the four `DW-`/`DEC-` entry points resolve the id space through `ids_in_use(workspace_root, storage, store)` and return `Result<Identifier, QdevError>`; the `_in_with_rng` pair takes `Option<&dyn Store>` like `allocate_next_story_id_in` -- the same question, asked once
- [x] `crates/qdev-core/src/lib.rs` -- re-exports follow the signatures -- keep the public surface honest *(no edit needed: the re-export names are unchanged and only the signatures moved. Left ticked with this note rather than unticked, because the task was checked and found already satisfied — review flagged it as a ticked no-op, which it is)*
- [x] `crates/qdev-core/tests/id_tests.rs` -- cover the hex rows of the matrix, including the length growth with a rigged rng and the workspace-wide ownership case -- the holes the flat scan left
- [x] `crates/qdev-core/tests/validate_tests.rs` -- a stale row's id is in `ids_in_use` and is skipped by allocation, on a fixture where the declared and carried halves see nothing -- make the deliberate exception enforceable
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- repeated sweeps over a standing duplicate pair keep one winner and equal a rebuild; a three-file variant of the same -- pin the property that was verified by hand
- [x] `docs/architecture.md` -- state the rule as "every allocator asks `ids_in_use`" and note the stale exception where the derivation rule is described -- the text currently invites the mistake
- [x] `docs/bmad/implementation-artifacts/deferred-work.md` -- mark the three entries this story closes -- the ledger is the acceptance checklist

**Acceptance Criteria:**
- Given a workspace where a `DW-` hash is owned in any of the ways `ids_in_use` recognises, when a hex id is allocated, then that hash is not returned — no allocator has an answer of its own.
- Given a cache holding a stale row, when any id is allocated, then that row's id is not handed out, and reverting `ids_in_use` to the derivation-filtered listing fails a named test.
- Given a workspace with a duplicate-id pair (and, separately, a triple), when it is swept repeatedly with no edits, then the same path owns the row every pass and the cache equals a full rebuild of the same tree table by table.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

- **`allocate_hex_id_with_rng(used: &HashSet<String>, prefix, rng) -> String`** is now pure, and
  `hex_id_collides` is gone with its `read_dir`. The set is lowercased once per call inside the
  allocator and `hex_id_taken` compares `{prefix}-{hash}` lowercased against it, which is what
  makes `id: DW-7F3A` take `dw-7f3a`'s space.
- **All six public entry points return `Result<Identifier, QdevError>`** (the `?` on `ids_in_use`
  has to go somewhere), and the `_in_with_rng` pair gained `store: Option<&dyn Store>` as the last
  parameter, matching `allocate_next_story_id_in`. The two-argument `_with_rng` forms and the
  one-argument plain forms delegate with `StorageConfig::default()` and `None`, and their rustdoc
  now carries the same warning `allocate_next_story_id` carries: against a configured `[storage]`
  they ask about the wrong tree, and against an initialised workspace they cannot see cache-only
  ids. `lib.rs` needed no edit — the names are unchanged and only the signatures moved. Still no
  production caller; `qdev create dw` / `qdev create decision` is epic 2 story 2.8.
- **Two of the eight new hex tests do not fail under the flat-scan reversion**, and that is
  honest rather than a weak test: the deleted scan's `{prefix}-{hash}.` and `{prefix}-{hash}-`
  prefix branches did happen to catch a `.MD` name and a slugged unparseable file in its own
  directory. Those two rows pin the *carried* half of the union instead, and they fail when it is
  reverted (see Verification). Every other new test fails under the flat-scan reversion.
- **The invariant test was rewritten during verification** because it passed under the reversion:
  with a wandering rng, a random 4-character hash almost never lands on one of five fixture ids,
  so the assertion held by coincidence — the same failure mode `spec-one-id-in-use-rule.md`'s
  review found in *its* invariant test, one story later. It now rigs the rng to demand each owned
  hash in turn.
- **The convergence tests needed a settling sweep** in the three-file variant: after removing the
  canonical name, the first pass legitimately purges the removed path's row, so `purged == 0` can
  only be asserted from the pass after it. The removal is what makes the fixture able to tell
  "the sorted-last path wins" from "the canonical name wins" — every `-suffix` name sorts before
  `E1S1.md`, so with the canonical file present the two rules give the same answer.
- **One pre-existing doc defect fixed in passing:** `architecture.md`'s in-use section had a
  dangling half-sentence ("Both read the same in-use set, and / refuses rather than colliding when
  it cannot"), left by an earlier edit. Repaired rather than left in a paragraph this story
  rewrites.

## Spec Change Log

- 2026-09-11 — Implemented. The one signature surprise: all six hex entry points return `Result` rather than the four the task list named, because the plain and `_with_rng` forms delegate to the `_in_with_rng` pair and cannot swallow its error.
- 2026-09-11 — Created from the two `deferred-work.md` entries whose review-by is "before epic 1 acceptance", after `spec-init-cache-migration.md` closed the last high-severity defect. One story rather than two: the stale-row test guards the very rule the allocator change makes a second production caller of, and both are gates on the same acceptance.

## Review Triage Log

### 2026-09-11 — Review pass (blind-hunter 12 findings, edge-case-hunter 8, verification-gap 3 + 1)

Three reviewers, 24 findings, grouped into 11 entries by root cause. No `intent_gap` and no
`bad_spec`, so no loopback. Every mutation cited below I re-ran myself; where a reviewer's
demonstration is the evidence, it says so.

**Patched:**

- `[med]` `[patch]` **The `storage` parameter was not observed by any test** (verification-gap main finding, blind #6) — the reviewer replaced `storage` with `StorageConfig::default()` in both allocator bodies and the whole suite stayed green, because every hex fixture is built under the default layout. That is the *original* defect's shape: an allocator asking about the wrong tree. New `test_hex_allocation_resolves_the_configured_storage_layout` owns a hash under a configured `state_dir` and a decoy under the default one, so the mutation now fails (re-run, confirmed).
- `[med]` `[patch]` **The retry branch's oracle was unverified** (verification-gap) — with the all-zero rng, the step-3 candidate and every retry candidate are the same string, so deleting the membership test on the retry path (`return retry_hash` unconditionally) left the suite green. New `test_hex_allocation_retry_path_tests_each_fresh_candidate` makes each candidate distinct and the second one free; the mutation fails it (re-run, confirmed).
- `[med]` `[patch]` **Occupancy silently narrowed for names the identity rule does not resolve** (verification-gap, edge #4) — the deleted scan refused any name in `<state_dir>/dw/` merely *starting* with `DW-7f3a.`, so `DW-7f3a.notes.md` and a `DW-abcd.txt` sidecar used to block the hash and now do not. The narrowing is the right answer — occupancy that disagreed with resolution is the divergence this invariant exists to remove, and `create dw` writes `DW-7f3a.md`, which collides with neither — but the change's prose claimed the new oracle was strictly wider, which was wrong. `test_hex_allocation_ignores_names_the_identity_rule_does_not_resolve` puts the decision on the record and `architecture.md` states it.
- `[med]` `[patch]` **The `Result` arm the signature change was made for was never executed** (blind #5) — every test and call site ended in `.unwrap()`. `test_hex_allocation_propagates_a_store_failure` drops the `entities` table under the store and asserts `Err`. (Filesystem-only allocation cannot fail today: the scan swallows read errors — filed, see below.)
- `[med]` `[patch]` **Prefix isolation untested** (blind #7) — membership is tested on `{prefix}-{hash}`, and nothing asserted the complement: an oracle comparing bare hashes would have passed every other test while halving both id spaces. `test_hex_allocation_does_not_confuse_the_two_prefixes` pins it.
- `[med]` `[patch]` **A ledger claim outran its tests** (edge #8) — the `resolved:` line for the convergence entry said the `unchanged`-count half was now asserted, and the helper asserted `purged` and `parsed` only. Fixed on the side that was wrong: the helper now asserts `unchanged` too (it is the half a `parsed`-only assertion cannot pin — a regression re-parsing the whole tree keeps `parsed` right and moves this), and the ledger line says what is true, including that triage caught the gap. The same test's comment asserted an error-severity `duplicate_planning_id` that nothing checked; it now calls `find_duplicate_planning_ids` and asserts both paths and the severity.
- `[low]` `[patch]` **`architecture.md` over-claimed case-insensitivity as a property of the whole rule** (blind #2) — verified: `Identifier::from_str` is case-sensitive, so `id: e1s7` sits in the in-use set without affecting `E1` numbering. Scoped the claim to the hex comparison and said what the story allocator does instead (such a file is a schema violation in its own right, not an id to work around).
- `[low]` `[patch]` **The lowercasing rationale was inaccurate** (blind #3) — it framed `DW-7F3A` as a non-canonical spelling; `validate_hex_hash` admits only `0-9a-f`, so it is not a legal id at all. Reworded in the rustdoc and the architecture text.
- `[low]` `[patch]` **The repaired dangling sentence attributed the refusal to the wrong command** (blind #4) — I checked the pre-truncation text (`git show 64d600e:docs/architecture.md`, where it was already broken): the refusal in that paragraph is `--fix-ids` meeting a duplicate outside its kind's directory, which the following paragraph then describes. Re-attributed, and the hex exhaustion exception named there rather than left as a blanket guarantee.
- `[low]` `[patch]` **`ids_in_use`'s own rustdoc still described the pre-story world** (verification-gap, other findings) — it named two allocators at exactly the site a reader checks before adding a third private scan. It now names the hex pair, says none of them keeps a scan, and records the case-sensitivity asymmetry between its two consumers.
- `[low]` `[patch]` **Rustdoc narrated the diff's history** (blind #11) — ~20 lines of retrospective in front of a 25-line private function. Trimmed to what the code does plus one line of why, with the history left where it belongs (this spec). The exhaustion behaviour is now stated there instead.

**Filed, not fixed** (`deferred-work.md`, each with a `Review by:`): exhaustion returning a hash it knows is taken (blind #1, edge #1/#6 — pre-existing, and the frozen matrix row describes the current fallback, so the behaviour change goes to whoever gives these allocators a caller; the rustdoc and architecture text now say plainly that it returns a taken id); allocation answering ownership rather than reservation (blind #8, edge #2); no `…_from_set`/`…_from_scan` entry point, so N ids cost N walks (blind #9); and an id declared only inside an unreadable or unparseable file whose name carries nothing being invisible with `store: None` (edge #3 — same root as the existing duplicate-scan read-failure entry).

**Rejected:**

- `[low]` `[reject]` `hex_id_taken`'s lowercased-set precondition is documentation-only, and the `"DW"`/`"DEC"` prefix literals are repeated at two call sites (blind #10). Real but negligible: one private caller, which derives the set immediately above the loop. The proposed newtype adds a type to enforce a two-line invariant, and lowercasing inside the membership test would re-lowercase the whole set per candidate.
- `[false]` The reviewers' claim that reversion coverage was overstated because two of the hex tests do not fail under the flat-scan reversion (edge #5). Refutation: the Implementation Notes disclosed exactly that before review, and the ledger line says "six of them" rather than all seven. Nothing overstated.
- `[low]` `[reject]` The exhaustion fallback returns the step-3 candidate rather than the last retry, while the frozen matrix row says "the last candidate" (edge #7). The row is imprecise, not wrong about the outcome — both candidates are taken, both are 8 characters, and no consumer can tell them apart. Fixing it means editing the frozen block, so it is recorded here for renegotiation instead; the rustdoc and architecture text say precisely which candidate comes back.
- `[low]` `[reject]` Spec-internal counting inconsistencies — "seven"/"6 of the 7" against eight new `#[test]` functions, and a ticked `lib.rs` task that needed no edit (blind #12). A finding whose fix is to edit this build's spec is out of scope for triage; the counts and the task note were corrected as record-keeping, not as a triage patch. The `resolved:` ledger stamps landing before the review that records them is the one part that mattered, and it is patched above.


## Design Notes

- **Why the allocator becomes pure.** `allocate_hex_id_with_rng(used: &HashSet<String>, prefix, rng)` resolves the id space at the entry points and passes it down, so collision testing is set membership with no filesystem behind it. That is what makes "one answer" structural rather than conventional: there is no path left in the file for a second scan to appear on.
- **What this costs.** Each allocation now walks `specs_dir` and `state_dir` and parses frontmatter, where it previously read one directory. Story allocation already pays exactly this, and correctness at allocation time is worth a walk; there is no production caller to regress today.
- **Case handling.** Compare `{prefix}-{hash}` lowercased on both sides. The union holds canonical ids from the carried and cached halves (`Identifier::to_string()`) but *verbatim* frontmatter strings from the declared half, so `id: DW-7F3A` in a file must still take `dw-7f3a`'s space.
- **Why the sweep's winner is stable** (needed to write the test, and easy to get wrong): nothing persists "this path lost". The displaced file has no `entities` row and no `schema_violation`/`merge_conflict`/`read_error` finding, so the sweep's `unaccounted` gate re-reads and re-hydrates it every pass; walking in sorted order, the earlier path takes the row first and the sorted-last one takes it back. The observable consequence a test should also assert is the cost: a standing duplicate pair never reaches the `unchanged` fast path, so `parsed` stays non-zero on an unedited workspace.

## Verification

**Result (2026-09-11):** `cargo test --workspace` green (four full runs; see the note below), `cargo clippy --workspace --all-targets
-- -D warnings` clean, `cargo fmt --check` clean, and `git grep -n "read_dir"
crates/qdev-core/src/id.rs` reports no matches.

**Reversions run, not asserted.** Each mutation was applied, the suite run, and the file
restored:

| Mutation | Tests that fail |
|---|---|
| A — restore the flat, case-sensitive, own-directory, filenames-only oracle | 6 of the 8 new hex tests, including the invariant test |
| B — drop the lowercasing from `hex_id_taken` | the mis-cased-declaration test and the invariant test |
| E — `write::id_carried_by_filename` requires a literal `.md` | the uppercase-extension test and the invariant test |
| F — drop `carried_ids` from `DuplicateIdScan.all_ids` | the uppercase-extension and unparseable-file tests, the invariant test, and both pre-existing slugged-collision tests |
| C — `.filter(\|e\| e.exists_for_derivation())` on the listing in `ids_in_use_from_scan` | `test_ids_in_use_includes_a_stale_rows_id_and_allocation_skips_it` |
| D1 — the sweep's accounted-for gate set to `false` | both convergence tests, on the `parsed` assertion |
| D2 — the post-hydration row-move mirroring removed (maps snapshotted) | both convergence tests, on the winner assertion — the winner moves to `E1S1-copy.md` / `E1S1-c.md` |
| G — the sweep's sorted walk reversed (`entity_files.reverse()`) | both convergence tests, on the winner assertion — added in triage, run by me |
| H — `storage` replaced by `StorageConfig::default()` in both allocator bodies | `test_hex_allocation_resolves_the_configured_storage_layout` (before triage: nothing) |
| I — the retry path returns its first candidate unchecked | `test_hex_allocation_retry_path_tests_each_fresh_candidate` (before triage: nothing) |

**One unexplained failure, recorded not waved away.** During verification a single full-workspace
run reported exactly one failing test, between runs that were green before and after (four clean
full runs since). The command that caught it summed result counts rather than keeping the failure
block, so which test failed is unknown — my error. This story's own tests are ruled out by
stress-running them (40 consecutive runs of both convergence tests, 25 each of `id_tests` and
`validate_tests`), as are the two benchmark bounds and the previously-flaky rebuild-equals-sweep
test (12 runs each under added CPU load). Filed in `deferred-work.md` with `Review by: before epic
1 acceptance`, together with the fix for the diagnostic gap itself.

**Commands:**
- `cargo test --workspace` -- expected: clean, including the new tests in `id_tests`, `validate_tests` and `sweep_tests`
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: no output
- `cargo fmt --check` -- expected: no output
- `git grep -n "read_dir" crates/qdev-core/src/id.rs` -- expected: no matches; the allocator no longer touches the filesystem
