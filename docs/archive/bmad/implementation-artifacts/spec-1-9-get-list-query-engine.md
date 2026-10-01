---
title: '`get` / `list` Query Engine'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '6060bb4915313bea71a85d938cc00d00f2ac0232'
context:
  - 'docs/architecture.md'
  - 'docs/cli-reference.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** qdev has no way to read a single entity's isolated JSON projection or list/filter entities — every downstream workflow, gate, and agent skill needs deterministic reads from the cache, but no such command exists yet.

**Approach:** Add a cache-backed query module in `qdev-core` (entity projection with inherited constraints, direct relations, and computed `blocked`; a list query with epic/status/owner/module/sprint filters), and wire `qdev get`/`qdev list` CLI commands with JSON and compact-text output.

## Boundaries & Constraints

**Always:**
- Reads come only from the SQLite cache (`Store` trait methods) — never re-parse Markdown at query time.
- `get`/`list` are read-only: no write lock, no cache mutation.
- JSON output is byte-identical across runs for identical cache state: use insertion-ordered structures (e.g. `BTreeMap`/`Vec`, never `std::collections::HashMap`) for any serialized map/collection.
- `blocked` (stories only) is computed from the entity's own direct `depends_on` relation rows and each target's cached `status` — `true` if any target's status isn't `done`. No cycle traversal or graph algorithm (that belongs to Story 1.10's `qdev validate`).
- Bare-id resolution (`qdev get AD-43`) looks up `entities.id` directly (globally unique primary key) rather than re-deriving kind from ID grammar. An id containing `/` resolves as a constraint via `get_constraint`.
- `--owner me` resolves against the active identity (`config.identity.developer_id`, falling back to `resolve_git_email`) the same way `qdev update`'s attribution does.
- `qdev get`'s default output (no `--expand`) always includes `constraints` (with `inherited_from`), `relations`, and `blocked` — matches the epics.md example/AC literally. `--expand` only adds `scratch`; `--expand relations`/`--expand constraints` are accepted as no-ops for forward compatibility.

**Never:**
- No relation cycle detection, dangling-relation findings, or `qdev relate`/`unrelate` — Story 1.10.
- No `gates` field on the `get` payload — no story-to-gate association is modeled in the cache yet (Epic 3 hasn't started).
- No new table-formatting crate — hand-roll fixed-width text output, consistent with `AnnotatedConfig::to_text_report`.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Get by kind+id | `qdev get story E12S4 --json` | Projection with inherited constraints (`inherited_from`), direct relations, `blocked` | N/A |
| Get bare id / constraint | `qdev get AD-43`, `qdev get E12S4/NG-1` | Resolves via cache primary key, no kind needed; slash form returns constraint object | N/A |
| Get invalid | Unknown id, or given kind doesn't match the entity's actual kind | — | Exit 2, `entity_not_found` / `usage_error` |
| List with filters | `qdev list stories --epic E12 --status ready --owner me --module bridge --sprint 5` | Rows matching every filter (AND), ordered by id | N/A |
| List invalid | `--owner me` with no resolvable identity, or unknown kind (e.g. `widgets`) | — | Exit 2, `usage_error` |
| Text mode | No `--json` | Compact human block (`get`) / column table (`list`) | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/query.rs` (new) -- `EntityProjection`/`ConstraintProjection` (+ `inherited_from`) types, `QueryOptions`/`ListQueryOptions`, `query_entity`/`query_list` functions: cache lookup, constraint inheritance (own id + epic id via `get_constraints_for_owner`), relation grouping (`get_relations_for_source` grouped into an ordered map), and `blocked` computation -- new query engine
- `crates/qdev-core/src/store/mod.rs:236-240` -- extend `EntityFilter` with `epic_id`/`owner`/`module`/`sprint` -- list filter surface
- `crates/qdev-core/src/store/sqlite.rs:708-790` (`list_entities`) -- extend `WHERE` for epic/owner/module (JSON-array containment) and join `sprint_assignments` for `--sprint` -- filtered listing
- `crates/qdev-core/src/lib.rs` -- re-export `query` module -- public API
- `crates/qdev-cli/src/cli.rs:27-41` -- add `Get(GetArgs)`/`List(ListArgs)` variants; `GetArgs` mirrors `UpdateArgs`'s `target`/`id` shape (cli.rs:44-49) + `--expand`; `ListArgs` has required `kind` positional + `--epic --status --owner --module --sprint` -- CLI grammar
- `crates/qdev-cli/src/main.rs:161-215,726-746` -- add `Get`/`List` dispatch arms; reuse `handle_update`'s target/id resolution pattern for `handle_get` -- CLI execution
- `crates/qdev-cli/src/main.rs` (new fns) -- fixed-width text renderers for `get` (key/value block) and `list` (column table) -- text-mode output
- `crates/qdev-cli/tests/get_cli_tests.rs`, `crates/qdev-cli/tests/list_cli_tests.rs` (new) -- E2E tests per AC and I/O matrix -- CLI contract verification

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/mod.rs` -- extend `EntityFilter` with `epic_id`, `owner`, `module`, `sprint` fields -- list filter surface
- [x] `crates/qdev-core/src/store/sqlite.rs` -- extend `list_entities` SQL for the new filters -- filtered listing
- [x] `crates/qdev-core/src/query.rs` -- implement `query_entity`/`query_list`, constraint inheritance, relation grouping, `blocked` computation -- query engine
- [x] `crates/qdev-core/src/lib.rs` -- re-export query module -- public API
- [x] `crates/qdev-cli/src/cli.rs` -- add `Get`/`List` commands and arg structs -- CLI grammar
- [x] `crates/qdev-cli/src/main.rs` -- dispatch, id/kind resolution, JSON + text rendering -- CLI execution
- [x] `crates/qdev-core/tests/query_tests.rs`, `crates/qdev-cli/tests/get_cli_tests.rs`, `crates/qdev-cli/tests/list_cli_tests.rs` -- unit + E2E coverage -- verification

**Acceptance Criteria:**
- Given a hydrated cache, `qdev get story E12S4 --json` returns constraints (with `inherited_from`), relations, and `blocked` by default, with byte-identical JSON across repeated runs.
- Given `--expand scratch`, the scratchpad section is added and nothing else changes.
- Given `qdev list stories --epic E12 --status ready --owner me --module bridge --sprint 5`, every filter applies (AND), rows are ordered by id, and text mode renders a compact table.
- Given the entire test suite, `cargo test` passes with zero failures.

## Implementation Notes

- 2026-09-09 — Review pass surfaced 7 real, `patch`-routed findings (see Review Triage Log). No agent re-engagement channel was available in this environment, so the fixes were applied directly: a workspace-init guard (`ensure_query_workspace`) before `get`/`list` open the cache; owner/module `list` filters switched from an unescaped, case-insensitive SQL `LIKE` to exact Rust-side array-membership matching (`json_string_array_contains`); a `stale: bool` field added to both `EntityProjection` and `ListEntryProjection`, passed through from the already-computed `EntityRecord.stale`; `docs/cli-reference.md` §3's example updated to match the actual payload shape (`gates` removed, `kind`/`safety_class`/`stale` added); the `query.rs` module doc comment's "insertion-ordered" claim about `BTreeMap` corrected; `render_list_text` no longer truncates the `id` column; and `render_get_entity_text` collapses embedded newlines in scratch entry text. Added targeted tests for each (`get_cli_tests.rs`, `list_cli_tests.rs`, `query_tests.rs`); full workspace `cargo test`/`clippy -D warnings`/`fmt --check` all clean afterward.

## Spec Change Log

## Review Triage Log

### 2026-09-09 — Review pass (blind-hunter, edge-case-hunter, verification-gap)

- `medium` — [blind-hunter] `qdev get`/`qdev list` outside an initialized workspace silently create `.qdev/cache/cache.sqlite` and fail with a raw `sqlite_error: no such table: entities` (exit 4) instead of a clean usage error. Verified by reproduction: ran `qdev get story E1S1 --json` in an empty temp dir — it created `.qdev/cache/cache.sqlite` and returned exit 4 with that message. Violates the frozen boundary "no cache mutation." → **patch**
- `medium` — [edge-case-hunter] Same finding as above (`main.rs:908-916`, missing workspace-init guard before `open_query_store`). → **patch** (same entry)
- `medium` — [blind-hunter] No test exercises `get`/`list` before the cache has ever been hydrated — the exact gap that let the above ship unnoticed. Same root cause/fix as above. → **patch** (same entry)
- `medium` — [blind-hunter] Owner/module `LIKE '%"' || ?4 || '"%'` filters (`sqlite.rs:722-723`) don't escape SQL wildcard characters (`%`, `_`); an owner/team value containing `_` (plausible in a git-email owner) causes false-positive matches, contradicting `EntityFilter`'s own "Exact owner name/team to match" doc comment. Verified by reading the SQL and confirming no `ESCAPE` clause exists. → **patch**
- `medium` — [edge-case-hunter] Same finding as above. → **patch** (same entry)
- `medium` — [verification-gap, Other findings] Same finding as above, noted while tracing filter test coverage (`test_list_owner_filter_avoids_partial_name_collision` only covers substring containment, not wildcard characters). → **patch** (same entry)
- `low` — [blind-hunter] Owner/module `LIKE` matching is also ASCII case-insensitive by default in SQLite (`--owner Simon` would match stored `"simon"`), contradicting the same "exact match" doc comment. Real but low real-world impact since this codebase's fixtures/convention use lowercase identifiers throughout. Folded into the same fix as the wildcard-escape finding above (switching owner/module filtering to exact Rust-side array-membership comparison resolves both). → **patch** (same entry as wildcard-escape)
- `low` — [edge-case-hunter] Same case-insensitivity finding. → **patch** (same entry)
- `medium` — [verification-gap, Other findings] `EntityRecord.stale` (set by hydration/sweep when a source file currently fails to parse — story 1.7) is never read by `EntityProjection`/`ListEntryProjection`. `get`/`list` silently return last-known-good cached data for an entity whose file is currently unparseable, with no indication the data is out of sync. Verified: `query.rs` never references `.stale`; the field exists on `EntityRecord` and is populated by the sweep. The fix is a direct passthrough of already-computed data (no new store method, no policy decision), so this doesn't need a human decision — routes as a patch, not an intent gap. → **patch**
- `medium` — [blind-hunter] `docs/cli-reference.md` §3's canonical `get` JSON example still shows a `gates` field and omits `kind`/`safety_class`, but this spec's approved Boundaries deliberately exclude `gates` (Epic 3 not started) and the implementation includes `kind`/`safety_class`. The reference doc is now stale relative to actual behavior. Verified by reading `docs/cli-reference.md` §3 against `query.rs`'s `EntityProjection`. → **patch**
- `low` — [blind-hunter] `crates/qdev-core/src/query.rs`'s module doc comment describes `BTreeMap`/`Vec` as "insertion-ordered" — `BTreeMap` actually iterates key-sorted, not insertion-ordered. The byte-identical-JSON guarantee still holds (sorted order is deterministic too), but the stated rationale is factually wrong. Verified by reading the comment and `BTreeMap`'s documented ordering semantics. → **patch**
- `low` — [blind-hunter] `render_list_text`'s fixed-width `id` column (`ID_WIDTH = 12`) truncates the identifier itself via `truncate_field`, the one column whose purpose is unique identification. JSON mode (the authoritative output) is unaffected; this is a text-mode-only cosmetic issue. Verified by reading `render_list_text`/`truncate_field`. → **patch**
- `low` — [edge-case-hunter] A scratch entry's `text` field containing an embedded newline (`render_get_entity_text`, the scratch-line formatting block) would break the compact text-mode block's line alignment. JSON mode is unaffected. Verified by reading the formatting code — `entry.text` is embedded raw with no newline handling. → **patch**
- `low` — [blind-hunter] `qdev list epic --module bridge` (or any non-story kind combined with `--epic`/`--module`/`--sprint`) silently returns an empty list rather than a clear "filter doesn't apply to this kind" error, since those filters only resolve through the `stories` join. Verified by reading the SQL. **Rejected**: unlikely in everyday use (the documented grammar and this story's AC only exercise these filters with `stories`), and the fix requires adding a new validation guard/branch — more than a direct correction.
- `false` — [blind-hunter] `query_entity`'s slash-id (constraint) path ignores a mismatched `kind_hint` with no diagnostic (e.g. `qdev get epic E12S4/NG-1` succeeds identically to `qdev get story E12S4/NG-1`). **Rejected**: a constraint id is self-describing (its owner is embedded in the id itself), so the kind hint is redundant for this path by design — consistent with this same command's existing accepted-no-op handling of `--expand relations`/`--expand constraints`. No demonstrated harm: the correct constraint is still returned.
- `low` — [blind-hunter] `compute_blocked` issues one `store.get_entity()` call per `depends_on` relation row in a loop (N+1 pattern) rather than batching. Real but low-impact: stories rarely carry many `depends_on` edges, and architecture's own benchmark target is ~1,000 entities total. **Rejected**: unlikely to matter in everyday use, and the fix requires introducing a new batched-lookup `Store` method — more than a direct correction.
- `low` — [blind-hunter] `qdev list` has no pagination or result cap. **Rejected**: no requirement states one; architecture's own hydration benchmark targets ~1,000-entity repositories, a size an unbounded CLI list handles without issue, consistent with every other qdev command.
- `n/a` — [blind-hunter] The spec's frontmatter `context:` list doesn't include `docs/bmad/planning-artifacts/epics.md`, which the frozen Boundaries reference when justifying the `gates`-omission decision. **Rejected** per the standing rule: reject any finding whose fix is to edit this build's spec.
- `false` — [edge-case-hunter] A malformed/non-array JSON value in the cached `owners`/`target_modules` column would make `parse_json_string_array` silently return an empty list rather than erroring. **Rejected**: verified that column is written exclusively by this codebase's own hydration path (`sqlite.rs:4009,4114`), which always serializes a valid JSON array from parsed frontmatter — no reachable path in this codebase produces malformed JSON there. Consistent with this project's convention of trusting internally-controlled data rather than defending against unreachable states.
- `false` — [edge-case-hunter] `--expand scratch` on a target that resolves to a constraint (not an entity) is silently accepted with no effect. **Rejected**: constraints don't have their own scratchpad (it belongs to the owning entity), so this is consistent with the same command's existing accepted-no-op handling of inapplicable `--expand` values (`relations`/`constraints` are already no-ops by design). No demonstrated harm.

## Design Notes

- **Owner/module filter matching**: `owners`/`target_modules` are stored as JSON-array text (e.g. `["simon","team:core-platform"]`). Filter with `LIKE '%"' || ? || '"%'` against the quoted value to avoid partial-name collisions (e.g. `"sim"` must not match `"simon"`).
- **Constraint inheritance**: call `get_constraints_for_owner` twice — once with the entity's own id, once with its `epic_id` (stories only) — and tag the epic-owned rows `inherited_from: <epic_id>` in the response; the store layer has no combined query for this today.
- **`get` text mode**: single-entity `key: value` block (no table library needed). **`list` text mode**: fixed-width columns (`id`, `title`, `status`, `owner`(s)), truncated to terminal-friendly widths.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all unit, integration, and CLI tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
