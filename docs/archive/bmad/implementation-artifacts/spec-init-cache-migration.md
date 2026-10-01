---
title: '`qdev init` can migrate a cache that has rows in it'
type: 'bugfix'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 1
baseline_commit: 'c00eb9ae1cbe1ea1f9e3f3f57549a41e39e66476'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-3-qdev-init.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev init` cannot migrate any cache that has rows in it, and the confirmation gate it fails inside protects nothing.

- **The migration fails on every real workspace.** `drop_all_user_tables` guards itself with `PRAGMA foreign_keys = OFF`, which SQLite documents as **a no-op inside a transaction**. `ensure_cache`'s rebuild calls that helper outside any transaction, so the pragma takes effect and the rebuild works; `init::initialize_cache` wraps it in `BEGIN IMMEDIATE`, so it does not, and the first `DROP TABLE entities` fails against any child row. Reproduced twice, once by me: a cache holding a single story, stamped to an older version, then `qdev init --yes` → `sqlite_error: Failed to drop table entities during migration: FOREIGN KEY constraint failed`, exit 4, cache left at the old version. The next `qdev list` migrates the same cache successfully. Every migration fixture in the suite uses an *empty* cache, which is why it has always passed.
- **The gate it fails inside is decorative.** `init` refuses an older cache without `--yes` (exit 3, `needs_confirmation`), and every other command performs the same drop-and-rebuild silently at boot. So a user who upgrades the binary, runs `qdev init`, and is told a migration needs confirming can run any read command instead and have it happen anyway. Two approved boundaries contradict each other: story 1.3 requires the confirmation, story 1.6 requires the unconditional boot rebuild.

**Approach:** make the drop work regardless of the caller's transaction state, so the helper cannot be correct for one caller and broken for another. Then settle the contradiction between the two boundaries rather than leaving a gate that fires only where it is broken.

## Boundaries & Constraints

**Always:**
- `drop_all_user_tables` works whether or not the caller has a transaction open. It is called from both a transactional path (`init`) and a non-transactional one (`ensure_cache`), and a helper that depends on which is a defect waiting for the other caller.
- A migration is lossless in both paths: the Markdown files are the source of truth (AD-3/FR-101), and after either path the cache holds what a full rebuild would.
- Whatever the two paths do about confirmation, they agree. A user must not be told a migration needs permission by one command and have it performed silently by the next.
- **Decision (2026-09-11, Simon):** option A — an older cache needs no confirmation from anyone. `init` migrates it exactly as every other command already does, because the cache is a rebuildable index (AD-3/FR-101): asking permission to rebuild it protects nothing, and a refusal the next command ignores is worse than no refusal. **Story 1.3's frozen boundary is renegotiated**, and carries an amendment note saying so, in the manner of story 1.6's.
- `init` keeps accepting `--yes`, now with no effect on the cache: it existed only for this gate, and turning `qdev init --yes` into a usage error would break every script and CI job that passes it. Its help text and the CLI reference say it is retained for compatibility.
- `init` remains idempotent on an already-current cache, and its `--yes` handling for the *other* things it confirms is untouched.

**Never:**
- No cache schema change, no `CACHE_SCHEMA_VERSION` bump, and no change to the stamping rule or the newer-than-supported refusal.
- No change to what `ensure_cache` does on a *newer* cache — that stays the `schema_version_mismatch` refusal with `sync --rebuild` as the recovery.
- No change to `init`'s other confirmations (missing `--name`/`--developer`/`--team` stay exit 3).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Populated older cache, `init` | cache with entity and child rows, older version | migrates and re-stamps; no FK error | N/A |
| Empty older cache, `init` | older version, no rows | migrates, as today | N/A |
| Populated older cache, boot | any other command | migrates and re-stamps, as today | N/A |
| Current cache, `init` | already at the current version | idempotent, reports already-initialized | N/A |
| Newer cache | stamped above this binary | `schema_version_mismatch`, exit 5, unchanged | Exit 5 |
| After either migration | same tree | cache equals what a rebuild produces | N/A |
| Older cache without `--yes` | populated older cache, `qdev init --non-interactive` and no `--yes` | migrates, exit 0 — no `needs_confirmation` refusal | N/A |
| `--yes` still accepted | `qdev init --yes` | accepted, no effect on the cache | N/A |
| Missing flags | `qdev init --non-interactive` with no `--name` | still exit 3 naming the flag | Exit 3 |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/store/sqlite.rs` (`drop_all_user_tables`) -- `PRAGMA foreign_keys = OFF`, the drop loop, then `PRAGMA foreign_keys = ON`. The pragma is a no-op inside a transaction; `PRAGMA defer_foreign_keys = ON` is the form that works inside one and resets itself at commit -- the fix
- `crates/qdev-core/src/init.rs` (`initialize_cache`) -- both branches wrap `drop_all_user_tables` + `create_schema` + stamp in `BEGIN IMMEDIATE` … `COMMIT`, which is what disables the guard -- the broken caller
- `crates/qdev-core/src/store/sqlite.rs` (`reset_and_rebuild`) -- calls the same helper with no transaction open, which is why the boot path works -- the working caller, and the reason this went unnoticed
- `crates/qdev-core/src/init.rs` (`initialize_cache`'s `!allow_migration` branch) -- the exit-3 `needs_confirmation` refusal, reachable only from `init` because `init` is dispatched before the `ensure_cache` stage -- the gate the Open Question settles
- `crates/qdev-cli/src/main.rs` (`ensure_cache` boot call) and `crates/qdev-core/src/store/sqlite.rs` (`CacheSchemaStatus::Mismatch` → `reset_and_rebuild`) -- the silent boot migration the gate is supposed to be guarding -- the other half of the contradiction
- `crates/qdev-core/tests/init_tests.rs` -- the migration fixtures, every one of which builds a cache with no rows, which is exactly why a passing suite never caught this -- the gap to close
- `crates/qdev-cli/tests/init_cli_tests.rs` -- the `--yes` / no-`--yes` migration tests, whose fate follows the Open Question -- verification
- `docs/bmad/implementation-artifacts/spec-1-3-qdev-init.md` -- the confirmation boundary is at line 30, inside `<frozen-after-approval>`; option A needs an amendment note there, as spec-1-6 carries for its own renegotiation -- the boundary in play

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- `drop_all_user_tables` defers foreign keys in a way that works inside a transaction as well as outside it -- one helper, both callers
- [x] `crates/qdev-core/tests/init_tests.rs` -- a migration fixture whose cache has entity *and* child rows, so the FK path is exercised; assert the migrated cache equals what a rebuild produces -- the instance and the invariant
- [x] `crates/qdev-core/src/init.rs` -- remove the migration confirmation gate and the `allow_migration` plumbing behind it; the interactive prompt in `handle_init` goes with it -- no refusal the next command ignores
- [x] `crates/qdev-cli/src/cli.rs` -- `init --yes` stays accepted with no cache effect, and its help says so -- do not break scripts that pass it
- [x] `crates/qdev-cli/tests/init_cli_tests.rs` -- cover every matrix row, including a populated cache end to end through the binary -- verification
- [x] `docs/architecture.md`, `docs/cli-reference.md` -- state what an older cache does and that both paths do the same thing -- keep docs authoritative
- [x] `docs/bmad/implementation-artifacts/spec-1-3-qdev-init.md` -- an amendment note on the renegotiated confirmation boundary, and its two migration matrix rows marked superseded -- the record says what changed and why

**Acceptance Criteria:**
- Given a cache holding at least one entity with child rows, stamped to an older version, when `qdev init --yes` runs, then it migrates and re-stamps with no `FOREIGN KEY constraint failed`, and the resulting cache matches what `qdev sync --rebuild` produces from the same tree.
- Given the same cache, when any other command boots, then it migrates as it does today — the two paths agree on the outcome.
- Given a populated older cache and no `--yes`, when `qdev init --non-interactive` runs, then it migrates and exits 0 — there is no command sequence in which a user is refused a migration and then has it performed silently by the next command.
- Given `qdev init --yes`, when it runs, then the flag is accepted and changes nothing about the cache.
- Given a cache already at the current version, when `qdev init` runs, then it reports already-initialized and changes nothing.
- Given a cache stamped newer than supported, when `qdev init` runs, then it still exits 5 `schema_version_mismatch`.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

**The fix.** `drop_all_user_tables` now issues `PRAGMA foreign_keys = OFF; PRAGMA
defer_foreign_keys = ON;` before the drop loop, and both are load-bearing: `foreign_keys` covers
the non-transactional caller (`SqliteStore::reset_and_rebuild`), `defer_foreign_keys` the
transactional one (`init::initialize_cache`). SQLite clears `defer_foreign_keys` at each COMMIT
or ROLLBACK, so nothing resets it; the trailing `PRAGMA foreign_keys = ON` re-enables
enforcement for the non-transactional caller and stays a no-op inside a transaction. The helper
is `pub`, so its contract is "works with or without a transaction open" rather than "works for
these two callers" — after review triage `init` no longer holds a transaction across the drop,
which makes the transactional half of the contract carried by
`test_drop_all_user_tables_inside_an_open_transaction_with_child_rows` rather than by an `init`
fixture.

**Confirmed empirically, not from the docs alone**, since the reason the bug bit at all was a
mechanism nobody had checked:

- The failure reproduces exactly as the Intent describes — a populated cache stamped to v2, then
  `qdev init --yes` → `sqlite_error: Failed to drop table entities during migration: FOREIGN KEY
  constraint failed`, exit 4, cache left at v2.
- Why enforcement is on at all in `init`, which never sets the pragma: the bundled SQLite is
  compiled with `SQLITE_DEFAULT_FOREIGN_KEYS=1` (`libsqlite3-sys-0.28.0/build.rs:123`), so every
  connection enforces by default. Without that, `init`'s raw connection would have had foreign
  keys off and the bug would not exist — this is the load-bearing half of the explanation and it
  is recorded in the rustdoc on the helper.
- The pragma semantics were checked directly against SQLite: `foreign_keys = OFF` inside
  `BEGIN IMMEDIATE` does not prevent the FK error; `defer_foreign_keys = ON` does, and the drop
  of every table commits cleanly; outside a transaction, `defer_foreign_keys` reads back as `0`
  after the first autocommitted statement, which is why it cannot replace `foreign_keys = OFF`
  for the other caller.

**`init`'s migration *is* the boot path's migration** (revised in review triage — the first
implementation hand-rolled the drop/create/stamp and then repopulated separately, which review
showed was a second copy of `reset_and_rebuild` kept in step by comment and already drifted in
stamp order). `initialize_cache` now classifies an existing cache with `inspect_cache_schema` and,
on `Mismatch`, acquires `<cache_dir>/write.lock` and calls `store.reset_and_rebuild(root,
storage)` — the same call `ensure_cache` makes, holding the same lock every other cache mutator
holds. The frozen boundary requiring the two paths to agree ("after either path the cache holds
what a full rebuild would") is now satisfied by there being one path;
`test_populated_older_cache_migrates_and_equals_a_rebuild` still asserts the equality table by
table, and it now also holds for a cache stamped current but structurally incomplete, which the
version-only detection called healthy.

**The gate is gone**, along with `InitOptions::allow_migration`, the `needs_confirmation` refusal
in `init::initialize_cache`, the duplicated non-interactive pre-check in `handle_init`, and the
interactive "Migrate cache schema from vN to vM? [y/N]" prompt. `qdev_core::init` still inspects
the cache upfront — before touching the filesystem — so a *newer* cache is the same exit-5
`schema_version_mismatch` refusal as before, now raised in one place instead of two.
The pre-flight is `verify_cache_compatible` (renamed from `check_cache_status` in review triage,
which returned a three-variant `CacheStatus` describing a decision nobody makes any more).
`InitArgs::yes` stays parsed and is read by nobody; its rustdoc, which is also its `--help` text,
says it is retained for compatibility. No `#[allow(dead_code)]` is needed — clap's derived
`update_from_arg_matches` counts as a use, checked by building with `-D warnings` after removing
the attribute rather than assumed.

**Test-fixture note.** Every new fixture, in the original pass and in review triage, was checked
to fail without the change it pins — reverting only the pragma reproduces the FK error, reverting
only the repopulation fails the rebuild-equality assertion, reverting detection to the
`user_version` comparison fails only the structural test, and repopulating through
`StorageConfig::default()` fails only the configured-layout test. Two comparison caveats: `sync_meta` is excluded from the
table-by-table equality because `last_synced_at` is a wall-clock stamp of when the pass ran, and
the fixture is deliberately finding-free (both stories parse, and the one relation points at the
other) so `findings.found_at` cannot make the comparison time-dependent either — the test asserts
that emptiness rather than skipping the table.

## Review Triage Log

### 2026-09-11 — Review pass (blind-hunter 14 findings, edge-case-hunter, verification-gap)

**Patched:**

- `[high]` `[patch]` **The migration was a second, hand-rolled copy of `reset_and_rebuild`, kept in step by comment** (blind #1), and **the two copies stamped and repopulated in opposite orders** (blind #2) — `init`'s migration branch now calls `store.reset_and_rebuild(workspace_root, storage)`, under an acquired `<cache_dir>/write.lock`, and step 6's separate repopulation is deleted. One implementation cannot drift from itself, the stamp-before-repopulate window is gone (the shared path stamps only after the rows are back), and `init` is no longer the one cache mutator outside the advisory-lock protocol. `initialize_cache` returns the `SweepSummary`.
- `[high]` `[patch]` **Detection covered version mismatch but not structural mismatch** (blind #3) — `init`'s classification of an existing cache is now `inspect_cache_schema`, the boot path's own inspector. A cache stamped current but missing a table was "up to date" to `init`, which reported `already_initialized` and repaired nothing, while the next command dropped and rebuilt it — the same divergence class this story exists to remove. New test `test_structurally_incomplete_cache_is_repaired_rather_than_called_up_to_date`, mutation-verified by reverting detection to the `PRAGMA user_version` comparison (fails; the configured-layout test still passes, so the two pin different things).
- `[med]` `[patch]` **`check_cache_status`'s return value was dead in production, and its `>` branch in `initialize_cache` unreachable via `init`** (blind #4, blind #5, edge #4) — `check_cache_status` and the three-variant `CacheStatus` are retired in favour of `verify_cache_compatible(root, storage) -> Result<(), QdevError>`: with the migration unconditional the `Ok` variants described a decision nobody made, and a reader could not tell the pre-flight refusal from a report. The newer-cache arm inside `initialize_cache` is kept, not asserted away — it is reachable by losing a race with a newer binary between the pre-flight and the rebuild, and the right outcome there is the shared refusal rather than a rebuild over a newer cache. Its comment says exactly that.
- `[med]` `[patch]` **The repopulation's result was thrown away** (blind #6) — `InitResult` gains `cache_files_rehydrated: Option<usize>`, text output prints `✔ cache schema migrated to v3 (N files rehydrated)`, and the JSON envelope carries the field. Named for what it counts: `SweepSummary::parsed` is files — entity Markdown plus the scratch, evidence and config files a rebuild also reads — and it is what `sync --rebuild` reports. My first version called it `cache_entities_rehydrated`, which the configured-layout test disproved on its first run (2 files, 1 entity); deriving a second, differently-defined number for the same pass would have been worse than reporting the one the sweep already has.
- `[med]` `[patch]` **No test exercised `drop_all_user_tables` directly inside an open transaction, and neither fixture covered the schema's other foreign key** (blind #7, blind #8) — `crates/qdev-core/tests/store_tests.rs::test_drop_all_user_tables_inside_an_open_transaction_with_child_rows` builds both parent/child pairs (`stories.id -> entities(id)` and `sprint_assignments.sprint_id -> sprints(id)`), asserts each parent really is protected by deleting it outside the helper first, then drops inside `BEGIN IMMEDIATE` and commits. Mutation-verified: removing `defer_foreign_keys` reproduces `Failed to drop table entities during migration: FOREIGN KEY constraint failed`. This is the pin a third caller of the public helper would break without.
- `[low]` `[patch]` **The CLI test's rationale was factually wrong about which table the foreign key protects** (blind #9) — `constraints` has no foreign key at all. The comment now names `stories.id REFERENCES entities(id)` as the key the drop order has to survive and says the `constraints` row is there only for realism; `row_counts` returns the `constraints` count too, so the fixture asserts the rows it mentions.
- `[med]` `[patch]` **No test for `--yes` plus a missing required flag** (blind #10) — `test_yes_does_not_satisfy_the_required_flags` runs all three omissions with `--yes` present, expecting exit 3 naming the flag and nothing scaffolded. The regression an inert-flag refactor invites is precisely `--yes` reading as blanket consent.
- `[med]` `[patch]` **The repopulation reads `layout.effective` but no fixture used a non-default layout** (blind #12, and the verification-gap agent's main finding) — `test_migration_under_a_configured_layout_repopulates_from_the_configured_tree` migrates a cache under a configured `specs_dir`/`state_dir`/`cache_dir`, asserts the rows come back and that no default-layout cache appears alongside. Mutation-verified by repopulating through `StorageConfig::default()` (fails; the structural test still passes).
- `[low]` `[patch]` **The spec shipped `status: 'in-progress'`** (blind #13) — front matter now records the terminal state, `review_loop_iteration: 1`.
- `[low]` `[patch]` **spec-1-3's Approach sentence was left unstruck and doubly stale** (blind #14) — the `v1` and the "migration confirmation" in the first paragraph a reader hits are now struck with the replacement inline, matching how every other superseded claim in that file is marked.
- `[low]` `[patch]` **`epics.md`'s story 1.3 acceptance criterion still required the confirmation** (edge #3) — marked superseded with the reason and a pointer here. Left unmarked, a re-drive of story 1.3 from the epic would have reintroduced the gate.
- `[low]` `[patch]` **`drop_all_user_tables`'s rustdoc claimed the trailing pragma "restores the connection default"** — it forces enforcement on, which is the default only by coincidence, and the doc named two specific callers where the contract is really "either caller". Reworded; the architecture paragraph too.

**Filed, not fixed** (all in `deferred-work.md`, each with a `Review by:`): `--yes`'s retirement (a breaking CLI change, so not taken inside a fix story); `init` delegating its *whole* cache path to `ensure_cache` rather than just the migration branch (blocked on two contract questions — the boot sweep's cost on a healthy cache, and `init`'s need to report the database in `created_files`); and `dump_all_tables` iterating `ALL_TABLE_NAMES` rather than `sqlite_master`, which would silently exclude a newly added table from the migration-equals-rebuild comparison.

**Note on the matrix.** The structurally-incomplete-cache scenario is not added to the I/O matrix: that block is `<frozen-after-approval>` and this is not a human renegotiation. It is covered by the acceptance criterion's spirit ("the two paths agree on the outcome") and pinned by a named test.

## Spec Change Log

- 2026-09-11 — Created from the epic 1 cross-story review pass 2 (findings NEW-1 and NEW-2), the last high-severity code defects that review named.
- 2026-09-11 — Open Question answered by Simon: option A, the gate goes and story 1.3's boundary is renegotiated. `init --yes` stays accepted but inert on the cache, so scripts passing it keep working. Matrix, tasks and AC extended. One story because the gate and the broken migration are the same event: the refusal fires only in the path that cannot perform the thing it is refusing.

## Design Notes

- `PRAGMA foreign_keys` is documented as a no-op inside a transaction; `PRAGMA defer_foreign_keys` exists for exactly this case and resets itself at the end of the transaction. Setting both makes the helper correct for either caller, which is the property that was missing — not a bigger hammer. **Verified in implementation** (see Implementation Notes), including the reason enforcement is on in a connection that never asks for it: `SQLITE_DEFAULT_FOREIGN_KEYS=1` in the bundled build.
- Dropping and recreating every table inside one transaction is safe with deferred enforcement because there are no rows left to violate anything at commit time: the child tables are dropped in the same transaction as their parents.
- The reason this survived every review: the helper is correct at one call site and broken at the other, and the only fixtures that reach the broken one build an empty cache. A test that migrates a cache with rows in it is the whole difference.
