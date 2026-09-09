---
title: 'Story 1.14: Cache Version Stamp Hardening'
type: 'bug'
created: '2026-09-10'
status: 'backlog'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'TBD'
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
| Half-migrated cache | current `user_version`, missing table or missing `entities.stale` | `Mismatch` → rebuild, as today | N/A |
| DDL touched by an external tool | cookie changed, `user_version` intact, tables intact | **`Valid`** — the cookie is no longer consulted | N/A |
| In-memory store | `SqliteStore::open_in_memory` | schema created and stamped; `Valid` | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:22-23` -- collapse `CACHE_SCHEMA_VERSION` / `CACHE_USER_VERSION` into a single constant -- version identity
- `crates/qdev-core/src/store/sqlite.rs` (`stamp_cache_version`) -- write only `PRAGMA user_version` -- the single stamp
- `crates/qdev-core/src/store/sqlite.rs` (`inspect_cache_schema`) -- drop the `PRAGMA schema_version` read and its comparison; keep the table and column checks -- validity rule
- `crates/qdev-core/src/store/sqlite.rs` (`reset_and_rebuild`) -- stamp through `stamp_cache_version` rather than its own pragma batch -- rebuild path
- `crates/qdev-core/src/init.rs` (`initialize_cache`) -- call `store::stamp_cache_version` instead of hand-writing `PRAGMA user_version`; ideally delegate the whole fresh-cache path to `ensure_cache` -- the init defect
- `crates/qdev-core/src/init.rs` (`check_cache_status`) and `sqlite.rs` (`ensure_cache`) -- reconcile the newer-than-supported case so both refuse rather than one refusing and one rebuilding -- consistency
- `crates/qdev-core/tests/sweep_tests.rs`, `crates/qdev-core/tests/init_tests.rs`, `crates/qdev-cli/tests/cache_cli_tests.rs`, `crates/qdev-cli/tests/init_cli_tests.rs` -- drop `PRAGMA schema_version` assertions; add the init-produces-Valid test -- verification

## Tasks & Acceptance

**Execution:**
- [ ] `crates/qdev-core/src/store/sqlite.rs` -- single version constant; `stamp_cache_version` writes only `user_version`
- [ ] `crates/qdev-core/src/store/sqlite.rs` -- `inspect_cache_schema` no longer reads the schema cookie
- [ ] `crates/qdev-core/src/init.rs` -- `initialize_cache` stamps through the shared path
- [ ] `crates/qdev-core/src/init.rs` + `sqlite.rs` -- one consistent rule for a newer-than-supported cache
- [ ] tests -- remove cookie assertions; add the regression tests below
- [ ] `docs/architecture.md` §10 -- state the cache version and that `user_version` alone carries it

**Acceptance Criteria:**
- Given a clean directory, when `qdev init` runs, then `inspect_cache_schema` returns `Valid` on the next call and the following command performs no rebuild — asserted by planting a marker table into the post-init cache and finding it intact afterwards.
- Given a cache whose `PRAGMA schema_version` cookie has been changed by an external DDL statement but whose `user_version` and tables are intact, when any command boots, then the cache is `Valid` and is not rebuilt.
- Given a cache stamped with a `user_version` newer than this binary supports, when any command boots, then it fails with `schema_version_mismatch` and exit 5 rather than being silently rebuilt.
- Given a cache written by an older binary, when any command boots, then it is still detected and rebuilt losslessly (existing behaviour preserved).
- Given the entire test suite, `cargo test --workspace` passes with zero failures, and no test asserts on `PRAGMA schema_version`.

## Implementation Notes

## Spec Change Log

- 2026-09-10 — Created from epic 1 retrospective action item 3 (Simon's decision: story-shaped, not a patch). Subsumes retrospective action item 1: finding B1 (`init` producing a cache the next command rebuilds) is a symptom of this misuse, so fixing the root cause closes both. Also closes the first `deferred-work.md` item recorded after the story 1.7 review.

## Design Notes

- The reason this survived thirteen stories and every per-story review: `PRAGMA schema_version` is instructed by story 1.6's own frozen spec (`spec-1-6-*.md:23,30,84`), so each story implemented it faithfully. That frozen boundary needs renegotiating as part of this story, or a re-drive of 1.6 reintroduces the misuse.
- The table-presence and column checks in `inspect_cache_schema` are what actually detect a bad cache; the cookie comparison has only ever produced false negatives. Removing it strictly reduces spurious rebuilds.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual: `qdev init` in a clean directory, then read `PRAGMA user_version`, plant a marker table, run `qdev list stories`, and confirm the marker survives
