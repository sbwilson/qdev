---
title: 'Route `qdev create story` through the story-1.8 write path'
type: 'refactor'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'f93e8c01b88985faa84701fa6ab5583977db33e6'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-8-write-path-atomic-files-locking-frontmatter-patching.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev create story` is the only writer in the tool that does not use the write path story 1.8 built. `handle_create_story` (`crates/qdev-cli/src/main.rs:757`) assembles frontmatter by `format!`-ing strings, takes no advisory lock, writes with a bare `OpenOptions::create_new`, never validates the result against the story schema, never touches the cache, and resolves the author itself — recording `type: human` unconditionally, ignoring `QDEV_AUTHOR_TYPE`. Epic 1's retrospective filed this as action item 4.

Four consequences, in the order they bite:

- **Attribution is wrong for agents.** An agent running `qdev create story` is recorded as a human. Every other mutation honours `resolve_author`; this one cannot, because it never calls it. AD-12 makes attribution audited, so this is a false audit record, not a cosmetic gap.
- **No lock, no atomic write.** A create racing a concurrent `qdev update` or sweep writes directly to the destination path with no advisory lock and no temp-file-then-rename, the two properties story 1.8 exists to guarantee.
- **Unvalidated output.** Nothing checks the generated frontmatter against the story schema. A future field added to the schema, or a title that breaks the hand-rolled quoting, produces a file that `qdev validate` rejects and hydration marks stale — created by qdev itself.
- **Cache lag.** The new story is invisible until the next boot sweep re-scans the tree. Every other write upserts the cache row and marks it dirty.

**Approach:** move story creation into `qdev-core`'s write module as a composition of the primitives that already exist there — build the frontmatter as a `serde_yaml` mapping, validate it with `validate_frontmatter`, take the advisory lock, write atomically, upsert the cache row and mark it dirty — and reduce `handle_create_story` to argument parsing, author resolution and output rendering, the same shape `handle_update` has.

## Boundaries & Constraints

**Always:**
- The composition lives in `qdev-core` (`write.rs`), not in the CLI. Per AD-2 the CLI parses arguments and renders output; core owns write logic. `handle_create_story` keeps id allocation, `resolve_author`, and envelope/text emission, and nothing else.
- Creating a story that already exists stays a conflict (`file_exists`, exit 5), and the existence check happens **while holding the advisory lock** — `write_file_atomic` renames over its destination, so the exclusivity `OpenOptions::create_new` used to provide must be re-established explicitly or it is silently lost.
- Author attribution comes from `resolve_author`, so `QDEV_AUTHOR_TYPE=agent` is recorded as an agent. `created_by` and `updated_by` are both set to the resolved author on creation.
- The generated file is validated against the story schema **before** it is written. A validation failure is a logical failure (exit 1) naming the offending fields, and leaves no file behind.
- The emitted frontmatter keeps its current field order and shape — `id`, `title`, `status`, then the optional `appetite` / `safety_class` / `target_modules` / `owners`, then `version`, `created_by`, `updated_by`. Existing `create_cli_tests.rs` assertions, including the escaped-title golden, must pass unchanged.
- The cache row is upserted and marked dirty through `upsert_cache_and_mark_dirty`, so the story is queryable without waiting for a sweep.
- **Decision (2026-09-10, Simon):** `qdev create story` gains `--author-type` and `--author-id`, for parity with `qdev update`. They are `resolve_author`'s first source, so a caller can attribute a single create without touching the environment; both are documented in `cli-reference.md` and each gets a test.
- The configured `[storage]` layout keeps being honoured for both allocation and the write target (established by `8220a63`).

**Never:**
- No new entity kinds and no `qdev create <other>` — `Story` is the only variant of `CreateCommands` and stays so.
- No change to `apply_entity_update`, `patch_frontmatter`, or any existing write-path function's behaviour. This story composes them; it does not modify them.
- No change to the JSON payload of `qdev create story` (`id`, `path`) or to its exit codes beyond the schema-validation case above.
- The story body template (`## Acceptance Criteria`) is not redesigned here.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy path | `qdev create story E12` in a clean workspace | `E12S1.md` written atomically, schema-valid, cache row present and dirty | N/A |
| Agent attribution | `QDEV_AUTHOR_TYPE=agent QDEV_AUTHOR_ID=bot qdev create story E12` | `created_by`/`updated_by` both `type: agent, id: bot` | N/A |
| Flag attribution | `--author-type agent --author-id bot`, env set to `human`/`alice` | flags win; `agent`/`bot` recorded | N/A |
| Bad author type | `QDEV_AUTHOR_TYPE=robot qdev create story E12` | refused before allocation or write | Exit 2 |
| Path occupied | target path already exists as a file or a directory | `file_exists` conflict, nothing written | Exit 5 |
| Concurrent create | a second writer holds `write.lock` | waits up to the 5 s timeout, then `lock_timeout` | Exit 5 |
| Schema-invalid output | generated frontmatter fails `validate_frontmatter` | named field errors, **no file created** | Exit 1 |
| Special-character title | `--title 'Feature "quoted": [special]'` | YAML-escaped exactly as today; file re-reads to the same title | N/A |
| Non-default `[storage]` | `specs_dir = "specs"` | allocation and write both under `specs/stories/` | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-cli/src/main.rs:757-905` (`handle_create_story`) -- the whole hand-rolled implementation: `format!`ed frontmatter (`:846-880`), the author fallback that skips `resolve_author` (`:812-817`), and the `OpenOptions::create_new` write plus its `file_exists` conflict arm (`:854-905`) -- what collapses into an argument-parse-and-render handler
- `crates/qdev-core/src/write.rs:829` (`upsert_cache_and_mark_dirty`), `:132` (`write_file_atomic`), `:62` (`acquire_write_lock`), `:200` (`Author`), `:800` (`current_iso8601`), `:809` (`sha256_digest`) -- the primitives to compose; all already public and used by `apply_entity_update`
- `crates/qdev-core/src/write.rs:1219` (`apply_entity_update`) and `:1192` (`EntityUpdateOptions`) -- the shape to mirror for the new create entry point: an options struct in, a result struct out, lock taken inside -- pattern, do not modify
- `crates/qdev-core/src/write.rs:1366-1450` (`story_detail_fields`) -- derives `epic_id`/`seq`/`appetite`/`safety_class`/`target_modules` from frontmatter-or-grammar for the `EntityRecord`; reuse it rather than deriving them a fourth way (three copies already exist — see `deferred-work.md`)
- `crates/qdev-core/src/schema.rs:440` (`validate_frontmatter`) and `:422` (`validate_frontmatter_detailed`) -- the pre-write gate; `_detailed` gives structured `ValidationError`s worth surfacing in the error `details`
- `crates/qdev-core/src/id.rs:451` (`allocate_next_story_id_in`) -- stays in the CLI handler, called before the lock as today -- do not move
- `crates/qdev-cli/src/main.rs:1456` (`resolve_author`) -- the shared resolver the handler must call; already returns a usage error for a bad type from either source
- `crates/qdev-cli/src/cli.rs:242` (`CreateStoryArgs`) -- current args: `epic`, `title`, `appetite`, `safety_class`, `module`, `owner`; the Open Question decides whether attribution flags join them
- `crates/qdev-cli/tests/create_cli_tests.rs` -- 15 tests including the escaped-title golden (`:283`) and the occupied-path conflict (`:305`); the contract that must survive -- verification
- `crates/qdev-core/src/store/sqlite.rs:70` (`stale INTEGER NOT NULL DEFAULT 0`) -- the created row is fresh, not stale -- keep the default

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/write.rs` -- add a create entry point mirroring `apply_entity_update`: options struct in, result struct out, frontmatter built as a `serde_yaml` mapping in the field order above -- core owns write logic
- [x] `crates/qdev-core/src/write.rs` -- inside that entry point: validate against the story schema, take the advisory lock, re-check the destination is free, `write_file_atomic`, then `upsert_cache_and_mark_dirty` with an `EntityRecord` built via `story_detail_fields` -- the write path, in order
- [x] `crates/qdev-core/src/lib.rs` -- re-export the new entry point and its option/result types alongside `apply_entity_update` -- public interface
- [x] `crates/qdev-cli/src/main.rs` -- reduce `handle_create_story` to: parse the epic id, allocate, `resolve_author`, call core, emit -- no `format!`ed YAML, no `OpenOptions`, no author fallback of its own
- [x] `crates/qdev-cli/src/cli.rs` -- add `--author-type` / `--author-id` to `CreateStoryArgs`, matching `UpdateArgs`' spelling, and pass them into `resolve_author` -- attribution parity
- [x] `crates/qdev-cli/tests/create_cli_tests.rs` -- add the matrix rows not already covered: agent attribution, bad author type, schema-invalid output leaves no file, cache row present without a sweep, lock contention -- verification
- [x] `docs/cli-reference.md` -- state that `create story` takes the advisory lock, validates before writing, and records the resolved author -- keep docs authoritative

**Acceptance Criteria:**
- Given a clean workspace, when `qdev create story E12` runs, then the story is queryable via `qdev get story E12S1` **without** an intervening sweep or rebuild, and its cache row is marked dirty.
- Given `QDEV_AUTHOR_TYPE=agent QDEV_AUTHOR_ID=bot`, when a story is created, then both `created_by` and `updated_by` record `agent`/`bot` — asserted by re-reading the file, not the payload.
- Given `--author-type agent --author-id bot` on the command line, when a story is created, then that attribution is recorded and it overrides any `QDEV_AUTHOR_TYPE`/`QDEV_AUTHOR_ID` in the environment; and `--author-type robot` is refused with exit 2.
- Given any created story, when it is re-read and validated with `validate_frontmatter`, then it is schema-valid; and when the whole workspace is validated, `qdev validate` reports no findings for it.
- Given a second process holding `write.lock`, when `qdev create story` runs, then it waits and exits 5 with `lock_timeout` rather than writing.
- Given the existing `create_cli_tests.rs` suite, when this story ships, then every test passes unchanged, including the escaped-title golden and the occupied-path conflict.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are all clean.

## Implementation Notes

- `qdev_core::create_story` (`crates/qdev-core/src/write.rs`) is the new entry point, mirroring
  `apply_entity_update`: `StoryCreateOptions` in, `StoryCreateResult` out, lock taken inside. In
  order: validate the author, reject a non-story id, build the frontmatter as a `serde_yaml`
  `Mapping` in the canonical field order, render the document, validate the exact bytes against
  the story schema (`validate_frontmatter_detailed`, so the failure carries structured field
  errors), take the advisory lock, re-check the destination with `symlink_metadata` (answers for
  a file, a directory or a dangling symlink alike, so the refusal never depends on an error
  kind), `write_file_atomic`, then `upsert_cache_and_mark_dirty` with an `EntityRecord` whose
  story detail fields come from the shared `story_detail_fields`.
- The document is rendered rather than emitted by `serde_yaml`'s serializer. A `Mapping`
  serialized by `serde_yaml` would drop the title's quotes and expand `target_modules` into a
  block sequence, changing every existing reader's and golden test's expected shape. The
  renderer keeps flow-style sequences, block-mapped attribution, and an always-double-quoted
  title (free text can never be re-read as another YAML type), and sends every other scalar
  through `serde_yaml`'s own emitter so it is quoted only when it must be.
- Schema validation now applies to values that were previously written unchecked: an
  `--appetite`/`--safety-class` outside the schema enum is a logical failure (exit 1) naming the
  field instead of a story `qdev validate` later rejects. This is the intended matrix row, and
  the only behaviour change beyond the four defects, so a caller passing an out-of-enum appetite
  today starts getting exit 1.
- `resolve_author` runs *before* allocation in the handler, so a bad `QDEV_AUTHOR_TYPE` is
  refused without scanning the specs directory.

### Review pass adjustments (2026-09-10)

- The cache upsert is guarded by `if cache_db_path.is_file()`. Unconditional, it created a
  16-table *unstamped* `cache.sqlite` in a bare directory, and the next `qdev init` there read
  that as a v0 cache needing a confirmed migration — `create story` followed by `init` exited 3
  demanding `--yes`. In a real workspace boot always creates the cache first, so the frozen
  "queryable without a sweep" guarantee is unaffected.
- For the same reason the advisory lock is skipped when the cache directory does not exist:
  outside a workspace there is no other qdev writer to serialize against, and taking it created a
  stray `.qdev/cache/write.lock`. `create story` in a bare directory now writes only the story
  file. `requires_workspace`'s doc comment was updated — it still claimed create "creates the
  cache as it goes".
- `render_yaml_scalar` falls back to a JSON-quoted string for any value `serde_yaml` renders as a
  multi-line block scalar, and `Author::validate` rejects control characters in an id.
- `resolve_author` treats an empty or whitespace-only `--author-id` / `QDEV_AUTHOR_ID` as absent.
- `StoryCreateResult` lost `kind`, `version` and `frontmatter` — no caller read them.

## Spec Change Log

- 2026-09-10 — Created from epic 1 retrospective action item 4, after action item 7 (single author-resolution path) landed in `f93e8c0` — this story consumes that resolver rather than reimplementing it.
- 2026-09-10 — Open Question answered by Simon: option B, `create story` gains `--author-type`/`--author-id`. Recorded in the frozen block; Tasks, AC and the I/O matrix extended accordingly.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | `create_story`'s advisory lock is exercised by no test: the CLI lock test is satisfied by the boot sweep, which takes the same lock before the handler runs. Deleting the lock left the whole suite green (verification-gap, pre-verified). | high | patch | Reproduced. Fixed by a core-level contention test that bypasses boot; re-mutating the lock away now fails it. |
| 2 | `render_yaml_scalar` splices a multi-line block scalar into one `key: value` line, so an author id containing a newline produces a document that no longer parses — surfacing as `schema_validation_failed` about qdev's own output (blind, edge). | medium | patch | Reproduced by the reviewer against the built binary. Two fixes: the renderer falls back to a JSON-quoted single-line scalar for any multi-line value, and `Author::validate` refuses control characters outright. |
| 3 | `--author-id ""` / `QDEV_AUTHOR_ID=` is accepted, allocates an id, then fails inside core with "Author ID cannot be empty" — contradicting the "refused before scanning" comment (blind, edge). | medium | patch | Confirmed. Empty or whitespace-only values are now treated as absent, so resolution falls through to config identity as if unset. |
| 4 | Outside a workspace `create story` still created `.qdev/cache/write.lock` and its parents; only `cache.sqlite` was guarded (blind, edge, verification-gap). | medium | patch | Confirmed by running the binary in a bare directory. The lock is now skipped when the cache directory does not exist, and the test asserts no `.qdev` tree at all. **This triage also caught a mistake in the guard's first placement — it landed in `apply_entity_update` rather than `create_story` (identical surrounding text, first-match replace), which is why finding 1's mutation initially appeared to pass.** |
| 5 | A non-default `[storage] cache_dir` is unverified even though the lock path and the cache path both derive from it; hardcoding the default left the suite green (verification-gap, blind). | medium | patch | Accepted as filed. New `test_create_story_honours_configured_cache_dir` asserts the row, the dirty mark and the lock all land under the configured directory and nothing at the default one. |
| 6 | `test_create_story_file_conflict`'s comment still explains the conflict via `OpenOptions::create_new` and the Windows error split — code this change deletes (blind). | low | patch | Confirmed. Rewritten to describe the in-lock `symlink_metadata` check, keeping the reason a *directory* is planted. |
| 7 | Tests inherit ambient `QDEV_AUTHOR_TYPE`/`QDEV_AUTHOR_ID` from the runner, so a developer with those exported fails the attribution assertions (edge). | low | patch | Confirmed by inspection. All 13 commands that do not set the vars themselves now `env_remove` both. |
| 8 | `StoryCreateResult::kind`/`version`/`frontmatter` are never read, and the new `file_exists` error `details` are unasserted (blind). | low | patch | Confirmed. Unused fields dropped; the JSON-mode conflict now asserts `code`, `details.id` and `details.path`. |
| 9 | `docs/cli-reference.md` overclaims — "every mutating entity command" covers commands that use different composition — and omits the one user-visible behaviour change (out-of-enum `--appetite`/`--safety-class` now exits 1) (blind). | medium | patch | Confirmed. The paragraph is narrowed to `create story` and `update`, the enum refusal and its valid values are documented, and the bare-directory behaviour is stated. |
| 10 | The lock test only proves the timeout, never that a create succeeds once the lock is released, and an early assertion failure leaves the holder thread holding it for 8 s (blind). | low | patch | Confirmed. The new core test asserts recovery and the holder releases on a deadline regardless. |
| 11 | Story-id allocation happens outside the advisory lock, so two concurrent creates can both allocate `E12S1`; the loser gets `file_exists` rather than `E12S2` (blind, verification-gap). | medium | defer | Real, but the frozen Code Map deliberately keeps allocation in the CLI ahead of the lock, and moving it is a design change this story excluded. Filed with the retry contract to settle. |
| 12 | `write_file_atomic`'s rename can still clobber a file created between the in-lock check and the rename by a writer that does not hold the lock (edge). | medium | defer | Real but narrow: every qdev writer holds the lock, so it needs an external process writing that exact path inside a microsecond window. An exact fix (create-new the destination, or `hard_link`) is more than a patch. |
| 13 | A cache-upsert failure after a successful write leaves the file on disk with a non-zero exit, and nothing covers that partial state (blind, edge). | low | defer | Checked: `apply_entity_update` propagates the same error the same way (`write.rs:1393`), so this is the existing write path's contract, not something this change introduced. Filed once for both. |
| 14 | The 5-second lock timeout is hardcoded at five call sites while the docs now promise it as a contract (blind). | low | defer | Pre-existing duplication — three of the five sites predate this change. A named `pub const` is a small cleanup, not part of this story. |
| 15 | The cache upsert is conditional (`if cache_db_path.is_file()`), so parity with `apply_entity_update`'s unconditional upsert is not exact (edge, filed as a claim). | false | rejected | Deliberate, and the difference is unreachable in a real workspace: boot runs `ensure_cache` before any command, so the cache always exists. The guard exists because the unconditional version created an unstamped cache in bare directories, making the next `qdev init` demand `--yes`. Recorded in Implementation Notes. |
| 16 | `docs/cli-reference.md`'s command table advertises `qdev create epic\|adr\|requirement\|hazard\|prd`, which do not exist (blind). | low | defer | Pre-existing documentation overclaim, untouched by this change and outside its intent. |
| 17 | Sprint status still shows this action item open, and the spec's frozen Intent cites "action item 4" where the retrospective's own table numbers it 5 (blind). | low | patch | Status is advanced at hand-off. The numbering discrepancy is inside `<frozen-after-approval>`; flagged to the human rather than edited. |

## Design Notes

- **The `create_new` exclusivity trap.** Today's implementation gets its "already exists" conflict for free from `OpenOptions::create_new(true)`. `write_file_atomic` renames a temp file over the destination, which *succeeds* on an existing path. Swapping one for the other without an explicit check would silently turn a refused create into a clobbered file — the single most dangerous line in this change. The check belongs inside the lock, since checking before taking it is a race.
- Both `created_by` and `updated_by` are set on creation because the hydration and query layers expect both to be present; a story with only `created_by` reads as schema-invalid.
- `serde_yaml` serializes a `Mapping` in insertion order, so the current field order survives without a custom serializer, and it handles the title escaping the hand-rolled `serde_json::to_string` currently approximates.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual: `qdev init` a clean directory, `QDEV_AUTHOR_TYPE=agent QDEV_AUTHOR_ID=bot qdev create story E12`, then `qdev get story E12S1 --json` with no sweep in between, and confirm the file records the agent author
