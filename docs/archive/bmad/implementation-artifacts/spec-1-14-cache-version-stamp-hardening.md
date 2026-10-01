---
title: 'Story 1.14: Cache Version Stamp Hardening'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'c341604f84c178a21ae37da9f53ecd12684b9ec0'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-6-sqlite-cache-schema-migrations.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** cache validity is keyed on `PRAGMA schema_version`, which is not an application-controlled field. It is SQLite's *internal schema cookie*, incremented automatically on every DDL statement and used by SQLite to invalidate other connections' prepared statements. `inspect_cache_schema` requires it to equal `CACHE_SCHEMA_VERSION`, so the cache's health depends on a counter the application does not own.

This has already caused one confirmed defect and one near-miss, both found by the epic 1 retrospective:

- **`qdev init` produces a cache the next command destroys.** `init` stamps only `user_version`; the schema cookie ends up at 16 (one per `CREATE TABLE`). `inspect_cache_schema` requires both to equal 3, so it declares the fresh cache invalid and the next command takes the write lock, drops every table and rebuilds. Reproduced: after `qdev init`, `user_version=3 schema_version=16`, and a marker table planted into the post-init cache does not survive the next command. `init`'s `✔ cache schema v3` is untrue the moment it prints.
- **Any future DDL touch converts a healthy cache into a destructive full rebuild.** Recorded in `deferred-work.md` after the story 1.7 review, then realised in the v2→v3 migration, where `create_schema` stamping an unmigrated database current would have left every sweep dying on a missing column. That was mitigated by splitting `create_schema` from `stamp_cache_version`, but the underlying misuse is untouched.

Writing the cookie backwards is also documented by SQLite as unsafe: sibling connections can execute statements compiled against a schema that no longer matches.

**Approach:** make `user_version` the single application-owned version stamp and stop reading or writing `PRAGMA schema_version` entirely. Cache validity becomes `user_version == CACHE_SCHEMA_VERSION` plus the existing table-presence and column checks, which already carry the real signal. Fold `init`'s hand-rolled pragma write into the same `stamp_cache_version` path every other writer uses, so a freshly initialised cache is `Valid` on first inspection and no longer triggers a rebuild.

## Boundaries & Constraints

**Always:**
- `PRAGMA user_version` is the only version stamp read or written. `CACHE_USER_VERSION` and `CACHE_SCHEMA_VERSION` collapse into one constant; keep the name `CACHE_SCHEMA_VERSION` since it is already public API and referenced by `init.rs`, `main.rs` and four test files.
- `inspect_cache_schema` keeps its existing table-presence check (`ALL_TABLE_NAMES`) and its `entities.stale` column check unchanged — those detect a half-migrated cache that a version stamp alone cannot, and story 1.7's `test_half_migrated_v1_cache_is_reported_as_mismatch` pins that.
- A cache whose `user_version` is older than `CACHE_SCHEMA_VERSION` still rebuilds; one that is *newer* still surfaces the `schema_version_mismatch` conflict from `init::check_cache_status` rather than being silently rebuilt. (`ensure_cache` currently rebuilds it — reconcile the two so both refuse.)
- `init` produces a cache that `inspect_cache_schema` reports `Valid` on the very next call, with no rebuild. This is the acceptance criterion the story exists for.
- **Decision (2026-09-10, Simon):** the newer-than-supported refusal is absolute except for `qdev sync --rebuild`, which skips the boot refusal and rebuilds from files. That is the documented recovery path, and the `schema_version_mismatch` message must name it. Everything else — including `qdev doctor` — exits 5.
- The existing v2→v3 rebuild-on-mismatch behaviour is preserved for real users: a cache written by an older binary is still detected and rebuilt.
- `create_schema` continues not to stamp (established in the epic 1 fix pass); only `stamp_cache_version` writes the stamp, and only callers that have just created or just dropped-and-recreated the tables may call it.

**Never:**
- No change to the cache schema itself — no new tables, no new columns, no `CACHE_SCHEMA_VERSION` bump. This story changes only how the existing version is stamped and checked.
- No second version dimension reintroduced under another name. If one is ever wanted, it belongs in the `sync_meta` table as an ordinary row, not in a pragma.
- No change to `sweep_workspace`, hydration, or any `Store` method.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Fresh init | `qdev init` in a clean directory | `inspect_cache_schema` → `Valid` immediately; the next command runs **no** rebuild | N/A |
| Repeat command | any command after `init` | cache rows survive; no write lock taken for a rebuild | N/A |
| Older cache | `user_version` < current | rebuild, as today | N/A |
| Newer cache | `user_version` > current | `schema_version_mismatch` conflict, exit 5 — not a silent rebuild | Exit 5 |
| Newer cache, explicit rebuild | `user_version` > current, `qdev sync --rebuild` | boot refusal skipped; cache rebuilt from files and re-stamped to current | N/A |
| Half-migrated cache | current `user_version`, missing table or missing `entities.stale` | `Mismatch` → rebuild, as today | N/A |
| DDL touched by an external tool | cookie changed, `user_version` intact, tables intact | **`Valid`** — the cookie is no longer consulted | N/A |
| In-memory store | `SqliteStore::open_in_memory` | schema created and stamped; `Valid` | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/store/sqlite.rs:26-27` -- `CACHE_SCHEMA_VERSION: u32 = 3` and `CACHE_USER_VERSION: u32 = 3`; delete `CACHE_USER_VERSION`, keep `CACHE_SCHEMA_VERSION` (public API, re-exported from `lib.rs:21,44` and `store/mod.rs:13-15`) -- version identity
- `crates/qdev-core/src/store/sqlite.rs:3327-3341` (`stamp_cache_version`) -- currently `PRAGMA user_version = N; PRAGMA schema_version = N;`; drop the second statement and its doc mention -- the single stamp
- `crates/qdev-core/src/store/sqlite.rs:3459-3552` (`inspect_cache_schema`) -- delete the `PRAGMA schema_version` read (3486-3496) and its half of the comparison at 3497; keep the `ALL_TABLE_NAMES` presence check and the `entities.stale` column check that follow -- validity rule
- `crates/qdev-core/src/store/sqlite.rs:370-400` (`reset_and_rebuild`) -- replace its inline two-pragma batch with a `stamp_cache_version(conn)` call; fix the stale `// ... pragmas are 2` comment -- rebuild path
- `crates/qdev-core/src/store/sqlite.rs:289-320` (`open_in_memory`) -- already `create_schema` + `stamp_cache_version`; no change needed -- in-memory store
- `crates/qdev-core/src/store/sqlite.rs:211-214` (`CacheSchemaStatus`) -- `Valid | Mismatch`; a newer cache lands in `Mismatch` today. A third variant is the natural way to let `ensure_cache` refuse; matched only in `ensure_cache` (3584-3603) and tests -- reconciliation
- `crates/qdev-core/src/store/sqlite.rs:3560-3625` (`ensure_cache`) -- both `needs_rebuild` matches rebuild on any `Mismatch`; make the newer case return the same `schema_version_mismatch` conflict `init::check_cache_status` raises -- reconciliation
- `crates/qdev-core/src/init.rs:487-496` and `:537-546` (`initialize_cache`) -- two hand-written `PRAGMA user_version = {}` batches inside `BEGIN IMMEDIATE` blocks; call `store::stamp_cache_version(&conn)` in both -- the init defect
- `crates/qdev-core/src/init.rs:188-208` (`check_cache_status`) -- already reads only `user_version` and already raises the newer-cache conflict; the behaviour `ensure_cache` must match, not change -- reference behaviour
- `crates/qdev-cli/src/main.rs:164-170` -- boot calls `ensure_cache` for every command in an initialised workspace, before dispatch; `crates/qdev-cli/src/main.rs:2029-2036` -- `qdev sync --rebuild` is reached only after that boot call -- constrains the Open Question above
- `crates/qdev-core/src/store/sqlite.rs:2849-2859` (`cache_schema_version`), `crates/qdev-core/src/doctor.rs:79-115` -- already read `user_version` only; leave unchanged -- do not touch
- `crates/qdev-core/tests/store_tests.rs:73-119` (`test_user_version_and_schema_version_pragmas`) -- drop its cookie assertion and rename; `:1262-1305` (`test_schema_version_only_mismatch_triggers_rebuild`) -- delete outright, it pins the behaviour this story removes -- verification
- `crates/qdev-cli/tests/cache_cli_tests.rs:99-115,170-175,240-256` -- three `PRAGMA schema_version` / `CACHE_USER_VERSION` assertion blocks to drop or retarget at `user_version` -- verification
- `crates/qdev-cli/tests/init_cli_tests.rs:620-640` (`test_future_schema_version_conflict_fails_with_yes`) -- existing newer-cache coverage through `init`; the `ensure_cache` side needs an equivalent -- verification
- `docs/architecture.md:245` (§10 SQLite Cache Schema) -- no version stamp is documented at all; add the cache version and that `PRAGMA user_version` alone carries it -- docs

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- delete `CACHE_USER_VERSION`, update the `lib.rs` / `store/mod.rs` re-exports, and make `stamp_cache_version` write only `PRAGMA user_version` -- one application-owned stamp
- [x] `crates/qdev-core/src/store/sqlite.rs` -- `inspect_cache_schema` no longer reads or compares the schema cookie; table and column checks unchanged -- validity rule
- [x] `crates/qdev-core/src/store/sqlite.rs` -- `reset_and_rebuild` stamps through `stamp_cache_version` instead of its own pragma batch -- single stamping path
- [x] `crates/qdev-core/src/init.rs` -- both `initialize_cache` branches stamp through `store::stamp_cache_version` -- the init defect
- [x] `crates/qdev-core/src/store/sqlite.rs` + `crates/qdev-core/src/init.rs` -- one rule for a newer-than-supported cache: `ensure_cache` raises the same `schema_version_mismatch` conflict as `check_cache_status`, with a message naming `qdev sync --rebuild` -- consistency
- [x] `crates/qdev-cli/src/main.rs` -- let `qdev sync --rebuild` past the boot refusal (only for that command + flag) so a newer cache stays recoverable in-tool -- recovery path
- [x] `crates/qdev-core/tests/store_tests.rs` -- delete `test_schema_version_only_mismatch_triggers_rebuild`, strip the cookie assertion from `test_user_version_and_schema_version_pragmas`, add the newer-cache `ensure_cache` test -- verification
- [x] `crates/qdev-cli/tests/cache_cli_tests.rs`, `crates/qdev-cli/tests/init_cli_tests.rs` -- drop remaining `PRAGMA schema_version` assertions; add the init-produces-`Valid` regression test -- verification
- [x] `docs/architecture.md` §10 -- state the cache version and that `user_version` alone carries it -- docs

**Acceptance Criteria:**
- Given a clean directory, when `qdev init` runs, then `inspect_cache_schema` returns `Valid` on the next call and the following command performs no rebuild — asserted by planting a marker table into the post-init cache and finding it intact afterwards.
- Given a cache whose `PRAGMA schema_version` cookie has been changed by an external DDL statement but whose `user_version` and tables are intact, when any command boots, then the cache is `Valid` and is not rebuilt.
- Given a cache stamped with a `user_version` newer than this binary supports, when any command boots, then it fails with `schema_version_mismatch` and exit 5 rather than being silently rebuilt.
- Given a cache stamped newer than supported, when `qdev sync --rebuild` runs, then the cache is rebuilt from files, re-stamped to `CACHE_SCHEMA_VERSION`, and the command exits 0 — while `qdev doctor` on the same cache still exits 5.
- Given a cache written by an older binary, when any command boots, then it is still detected and rebuilt losslessly (existing behaviour preserved).
- Given a half-migrated cache — current `user_version`, missing table or missing `entities.stale` — when any command boots, then it is still reported `Mismatch` and rebuilt (story 1.7's `test_half_migrated_v1_cache_is_reported_as_mismatch` still passes).
- Given the whole workspace, `cargo test --workspace` passes with zero failures, and `grep -r "PRAGMA schema_version" crates` returns no hits outside a test that deliberately perturbs the cookie.

## Implementation Notes

- `CacheSchemaStatus` gained a third variant, `NewerThanSupported { found, supported }`.
  `ensure_cache` returns on it (at both the pre-lock and under-lock inspections) via the new
  `store::newer_cache_conflict(found, supported)` constructor, which `init::check_cache_status`
  now also calls — so the boot path and the init path raise a byte-identical
  `schema_version_mismatch` conflict, and the message names `qdev sync --rebuild`.
- The `qdev sync --rebuild` escape hatch is implemented in `crates/qdev-cli/src/main.rs` at the
  boot `ensure_cache` call: the error is swallowed only when its code is
  `schema_version_mismatch` *and* the parsed command is `Sync` with `rebuild` set. Everything
  else — a plain `qdev sync` included — still exits 5. `handle_sync --rebuild` then opens the
  store directly and `reset_and_rebuild` drops, recreates, repopulates and re-stamps.
- The second `user_version > CACHE_SCHEMA_VERSION` branch inside `init::initialize_cache` was
  routed through the same constructor as `check_cache_status`, so all three refusal sites share
  one message.
- **Deviation from an acceptance criterion's stated probe.** AC-1 says to assert "no rebuild"
  by planting a *marker table* into the post-init cache. That probe cannot work: the
  table-presence check the story is required to keep unchanged also asserts an exact table set
  (`existing_tables.len() != ALL_TABLE_NAMES.len()`), so an extra table is itself a `Mismatch`
  and *causes* the rebuild it was meant to detect. Verified manually: a marker table does not
  survive, for that reason and not the one the story fixes. The tests use a marker **index**
  instead — `CREATE INDEX ... ON entities(kind)` — which moves the schema cookie exactly like a
  table would, is invisible to the table-set check, and is dropped by any rebuild along with its
  table. The behaviour AC-1 exists to pin is fully covered:
  `test_init_produces_cache_valid_on_next_command` asserts `inspect_cache_schema` → `Valid`
  directly on the cache `qdev init` wrote, then that the marker index survives the next command.
- `crates/qdev-core/tests/sweep_tests.rs` was not in the Code Map but held eight
  `CACHE_USER_VERSION` references and five `PRAGMA schema_version` writes/reads (legacy-cache
  fixtures and assertions). All were retargeted at `user_version`; the two fixtures that wrote
  the cookie backwards to make a downgraded cache "look healthy" no longer need to.
- Matrix audit (step-03): the frozen matrix's **In-memory store** row had no test asserting the
  stamp — `open_in_memory` is exercised by dozens of tests, but none checked `user_version` or
  the table set. Added `store_tests.rs::test_open_in_memory_creates_schema_and_stamps_version`.
  Every other matrix row was already covered by a test that ran and passed.

## Spec Change Log

- 2026-09-10 — Created from epic 1 retrospective action item 3 (Simon's decision: story-shaped, not a patch). Subsumes retrospective action item 1: finding B1 (`init` producing a cache the next command rebuilds) is a symptom of this misuse, so fixing the root cause closes both. Also closes the first `deferred-work.md` item recorded after the story 1.7 review.
- 2026-09-10 — Story 1.6's intent contract amended by human renegotiation to remove the `PRAGMA schema_version` instruction and forbid it going forward, so this story no longer contradicts an approved boundary. Note the amendment leaves 1.6's Implementation Notes and Review Triage Log as historical record, including `test_schema_version_only_mismatch_triggers_rebuild` — this story removes that test.
- 2026-09-10 — Planning pass: Code Map re-derived against the tree at `c341604`; Tasks and Verification sharpened. Two decisions recorded in Design Notes (no `ensure_cache` delegation from `init`; `CACHE_USER_VERSION` deleted, not aliased). One intent gap raised and answered by Simon (newer-cache recovery via `qdev sync --rebuild`), recorded in the frozen block.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | `newer_cache_conflict`'s message names `qdev sync --rebuild`, but two of its three call sites are in `init`, reached in a bare directory where `requires_workspace` (`main.rs:157-162`) refuses `sync` — the advice cannot be followed. Filed by blind-hunter and verification-gap. | medium | patch | Confirmed: `init_cli_tests.rs:621-659` pins exactly that state and asserts no `qdev.toml` is created. |
| 2 | `docs/architecture.md:254` says the newer cache is "recoverable **only** with `qdev sync --rebuild`"; deleting `.qdev/cache/cache.sqlite` also recovers, since a missing file inspects `Mismatch`. | medium | patch | Same root cause as #1 — one recovery over-claimed. Verified at `sqlite.rs:3489-3491`. |
| 3 | `docs/architecture.md:248` hardcodes "version is **3**" in prose; it will drift when `CACHE_SCHEMA_VERSION` moves. | low | patch | Real; fix is a direct reword, so the low-rejection rule does not apply. |
| 4 | `init_cli_tests.rs:626` comment still reads "user_version = 3 (newer than supported v2)"; the constant is 3 and the test stamps `CACHE_SCHEMA_VERSION + 1`. | low | patch | Confirmed by reading the test; stale since the v3 bump. Direct correction. |
| 5 | `test_newer_cache_refused_everywhere_except_sync_rebuild` asserts the re-stamp but never that entity rows came back from the Markdown files — "rebuilt from files" is unpinned. | low | patch | Confirmed: the test's only post-rebuild assertions are `user_version` and a `doctor` exit code. |
| 6 | The under-lock `NewerThanSupported` arm in `ensure_cache` (`sqlite.rs:3630-3637`) is unreachable by any test; replacing it with `=> true` leaves the suite green. Filed pre-verified by verification-gap, disposition `defer`. | medium | defer | Accepted as filed. Closing it needs a two-process harness this repo has no precedent for; the pre-lock arm covers every non-racing case. |
| 7 | `docs/architecture.md` §10's abbreviated DDL is still the v1 shape (no `findings`, no `entities.stale`), so the added paragraph's column check reads against a listing that lacks the column. | medium | defer | Real but pre-existing — epic-1 retrospective action item 6 already owns it; this diff added a paragraph, it did not create the contradiction. |
| 8 | `sync --rebuild` discards a newer binary's cache with no warning or confirmation, while `init`'s comparable drop gates behind `--yes`. Filed by blind-hunter and edge-case-hunter. | low | rejected | The cache is a rebuildable index by architecture (AD-3: wiping `.qdev/cache/` is zero data loss), and the user typed `--rebuild` after being told to. A confirmation prompt adds a branch for no user-visible gain. |
| 9 | `reset_and_rebuild` failing between `drop_all_user_tables` and `stamp_cache_version` leaves a newer stamp over empty tables, so boot keeps refusing. | low | rejected | Self-healing: `sync --rebuild` is exactly the command exempt from the refusal, so a retry re-runs the rebuild. Guarding it would add an error-path stamp write. |
| 10 | `ensure_cache`'s `Err(_) => true` arm rebuilds destructively, bypassing the new refusal when `inspect_cache_schema` itself errors. | low | rejected | Pre-existing arm, unchanged by this diff; reaching it needs a cache that is both newer-stamped and unreadable, in which case the version is unknown and rebuilding a corrupt cache is the intended behaviour. |
| 11 | A `user_version` stamped above `i32::MAX` wraps negative, fails the `u32` read, and lands in the `Err` arm instead of `NewerThanSupported`. | low | rejected | Requires a deliberately absurd stamp no qdev binary writes; the fix widens the read to `i64` and adds a sign branch. |
| 12 | `qdev doctor` exiting 5 on a newer cache contradicts `doctor.rs:91-92` ("a diagnostic that fails on a broken cache is no diagnostic at all"). | false | rejected | That comment is about a half-migrated cache whose row counts fail to read — still true and still reachable (`sweep_tests.rs:1695`). The newer-cache refusal is a different, deliberate, human-decided gate recorded in the frozen block. |
| 13 | The conflict code is still `schema_version_mismatch` and its detail keys `current_version`/`supported_version`, which now misdescribe what was compared. | low | rejected | The frozen block names `schema_version_mismatch` explicitly and it is a public JSON contract; renaming it would breach an approved boundary. |
| 14 | AC-1's stated probe (plant a marker **table**) was implemented as a marker **index**. | false | rejected | The table-presence check the story must keep also asserts an exact table set (`sqlite.rs:3550`), so a marker table is itself a `Mismatch` and causes the rebuild it was meant to detect. Disclosed in Implementation Notes; behaviour fully pinned. |
| 15 | The last AC's grep clause ("no hits outside a test that deliberately perturbs the cookie") is unsatisfiable — the surviving hits are two doc comments and no such test remains. | low | rejected | Correct as an observation, but its only fix is to edit this build's spec, which triage rejects. The AC's substance holds: no code or test reads or writes the cookie. |
| 16 | `sprint-status.yaml` says `in-progress` while the spec frontmatter says `in-review`; retro action items 1 and 2, which this story closes, are still `open`. Filed by blind-hunter and verification-gap. | low | patch | Bookkeeping, settled at hand-off: sprint status advances with the story and the two subsumed action items are closed there. |
| 17 | `type: 'bug'` → `'bugfix'` is unexplained in the change log. | low | rejected | Fix is to edit this build's spec, which triage rejects. `bugfix` is the template's enumerated value; `bug` was not. |

Patches applied in this session (the step-03 subagent could not be re-engaged — messaging is
disabled here): rows 1–5. `newer_cache_conflict`'s message and `recovery` detail now name
deleting the cache file alongside `qdev sync --rebuild`; `docs/architecture.md` §10 drops the
"only" and states the version as the value of `CACHE_SCHEMA_VERSION`; `init_cli_tests.rs:626`'s
stale "supported v2" comment is corrected; `test_newer_cache_refused_everywhere_except_sync_rebuild`
now plants `E12S1.md` and asserts the row is back in `entities` after the rebuild. Row 16 is
settled at hand-off. Full verification re-run green after the patches.

## Design Notes

- The reason this survived thirteen stories and every per-story review: `PRAGMA schema_version` was instructed by story 1.6's own frozen intent contract, so each story implemented it faithfully and each per-story review judged it correct against the spec. Only a cross-story retrospective could see it.
- **That boundary has already been renegotiated** (2026-09-10, Simon): `spec-1-6-*.md` now names `user_version` alone in its Approach, Boundaries and acceptance criteria, and carries a Boundaries "Never" forbidding any read or write of the cookie. A re-drive of story 1.6 will therefore not reintroduce the misuse, and this story is free to remove it from the code without contradicting an approved contract.
- The table-presence and column checks in `inspect_cache_schema` are what actually detect a bad cache; the cookie comparison has only ever produced false negatives. Removing it strictly reduces spurious rebuilds.
- **Planning decision — no delegation of `init`'s fresh-cache path to `ensure_cache`.** The frozen Code Map floated it as "ideally". `initialize_cache` owns semantics `ensure_cache` does not have: a `BEGIN IMMEDIATE` transaction around create-and-stamp, and the `--yes` policy refusal before a migrating drop-and-recreate (`init.rs:511-525`). Folding them together would move a confirmation gate into the boot path for every command. Routing both branches through `stamp_cache_version` fixes the defect with none of that risk; the fresh-cache and migration paths stay in `init.rs`.
- **Planning decision — `CACHE_USER_VERSION` is deleted, not kept as an alias.** It is `pub` and re-exported (`lib.rs:21`, `store/mod.rs:14`) but has exactly two consumers, both in `crates/qdev-cli/tests/cache_cli_tests.rs`. Leaving an alias behind would preserve the two-dimension mental model the story exists to remove. Pre-1.0, no external consumers.
- Once the cookie read is gone, the init defect is fixed by construction — `initialize_cache` already writes only `user_version`. Routing it through `stamp_cache_version` is about leaving one stamping path, so the next writer cannot reinvent a second.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- `grep -rn "PRAGMA schema_version" crates docs` -- expected: no hits outside a test that deliberately perturbs the cookie
- Manual: `qdev init` in a clean directory, read `PRAGMA user_version` (expect 3), plant a marker table, run `qdev list stories`, and confirm the marker survives
