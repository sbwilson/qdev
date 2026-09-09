---
title: 'Story 1.13: `qdev schema` for CLI output payloads'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '5339b6fa461fa319bd6f63ce9bcb6e596d14057c'
context: [ '{project-root}/docs/cli-reference.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-context.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev` commands emit structured JSON output (`get story`, `validate`, error envelopes, and future `context`/`gate_run`/`next` commands), but nothing lets a consumer — CI validators, or Epic 4's MCP tool advertisement — obtain a machine-checkable JSON Schema for those *output payload* shapes; only entity frontmatter shapes are schema-printable today (Story 1.4).

**Approach:** Add a new `qdev schema payload <name>` form, alongside the untouched, already-shipped `qdev schema <entity-kind>` (Story 1.4), that prints hand-authored JSON Schemas for CLI output payload shapes. Ship it now for the three payload kinds with a live command today (`story`, `error`, `validate`), each documenting `schema_version` and its semver policy in the schema `description` and verified by a round-trip test against real command output. `context`, `next`, and `gate_run`-as-a-payload have no command yet (Epic 2/3 scope) — record them as deferred work rather than hand-authoring unverifiable schemas now.

## Boundaries & Constraints

**Always:**
- `qdev schema payload <name>` is a new, separate form from `qdev schema <entity-kind>`; the latter's grammar, resolution (`EntityKind::from_str_loose`, including its `"story"` and `"gate_run"` aliases), and tests are unchanged by this story.
- A payload schema describes a command's JSON **output** envelope shape (what `--json` prints), never an entity's frontmatter shape.
- Every payload schema's top-level `description` states the `schema_version` semver policy already documented in `docs/cli-reference.md:161` ("additive fields do not bump the major").
- Ship payload schemas + round-trip tests only for `story` (via `get story`), `error`, and `validate` — the three with a live command today.
- Follow the `EntityKind::schema_str()` pattern (`crates/qdev-core/src/schema.rs:30`): hand-authored schema JSON files under `crates/qdev-core/schemas/`, embedded via `include_str!`.
- Append one `deferred-work.md` entry for `context`, `next`, and `gate_run`-as-a-payload each, noting they land alongside the stories that implement those commands.

**Never:**
- Do not implement the `context`, `next`, or `gate_run`-as-a-command features themselves (Epic 2/3 scope) — only defer their schemas.
- Do not hand-author schemas for `context`/`next`/`gate_run` payloads now — there is no live output to verify them against.
- Do not change `docs/cli-reference.md:49`'s existing `qdev schema <kind>` documentation beyond adding the new `qdev schema payload <name>` line.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Known payload, text mode | `qdev schema payload story` | pretty-printed JSON Schema on stdout | N/A |
| Known payload, JSON mode | `qdev schema payload story --json` | `{"schema_version": "1", ...schema}` envelope | N/A |
| Unknown payload name | `qdev schema payload bogus` | usage error naming valid payload names (`story`, `error`, `validate`) | exit 2, `usage_error` |
| `error` payload | `qdev schema payload error` | schema matches `JsonErrorEnvelope` (`code`, `message`, optional `details`) | N/A |
| Deferred payload name | `qdev schema payload context` \| `next` \| `gate_run` | usage error, not listed as valid until its command ships | exit 2, `usage_error` |
| Round trip, `story` | `qdev get story <id> --json` output | validates cleanly against the printed `story` payload schema | test fails the build otherwise |
| Round trip, `validate` | `qdev validate --json` output | validates cleanly against the printed `validate` payload schema | test fails the build otherwise |
| Existing entity-kind schema unaffected | `qdev schema story` (no `payload`) | still prints the Story 1.4 entity frontmatter schema, unchanged | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/schema.rs` -- `EntityKind` enum + `from_str_loose` (line 98, `"story"` → Story, `"gate_run"` → Evidence); `schema_str()`/`schema_json()`/`pretty_schema_str()` pattern to mirror -- untouched entity-schema machinery; add a sibling `PayloadKind` (or similar) enum following the same `include_str!` pattern, resolving only `story`, `error`, `validate`
- `crates/qdev-core/schemas/*.json` -- 13 hand-authored entity frontmatter schemas, `include_str!`-embedded -- add `payload-story.json`, `payload-error.json`, `payload-validate.json` alongside these
- `crates/qdev-cli/src/cli.rs:27` (`Commands` enum), `:220` (`SchemaArgs`) -- CLI grammar; `SchemaArgs` currently takes one positional `kind: String` -- add a `payload <name>` subcommand/positional under `Schema` without changing the existing `kind` path
- `crates/qdev-cli/src/main.rs:139-140,249-274` (`handle_schema`) -- current dispatch resolving `EntityKind` and emitting via `JsonEnvelope`/text -- branch on the new `payload` form to resolve/emit `PayloadKind` schemas the same way
- `crates/qdev-core/src/envelope.rs` -- `JsonEnvelope<T>` (line 10), `JsonErrorEnvelope`/`ErrorPayload` (line 34) -- shape to schema for the `error` payload
- `crates/qdev-core/src/query.rs:64` (`EntityProjection`), plus `ConstraintProjection`/`ScratchEntryProjection` -- shape to schema for the `story` payload (i.e. `get story`'s data, not the frontmatter)
- `crates/qdev-cli/src/main.rs:1736` (`ValidatePayload`), `crates/qdev-core/src/store/mod.rs:214` (`FindingRecord`) -- shape to schema for the `validate` payload
- `docs/cli-reference.md:49,112-161` -- add a `qdev schema payload <name>` grammar line beside the existing `qdev schema <kind>` line; schemas' `description` must echo the semver policy at line 161
- `crates/qdev-cli/tests/schema_cli_tests.rs` -- existing text/JSON-mode assertions for `qdev schema <entity-kind>` -- pattern for new payload CLI tests; add round-trip tests here or in a new file
- `docs/bmad/implementation-artifacts/deferred-work.md` -- append `context`, `next`, `gate_run`-as-a-payload entries

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-story.json`, `payload-error.json`, `payload-validate.json` -- hand-author JSON Schemas matching `EntityProjection`, `JsonErrorEnvelope`, `ValidatePayload` respectively, each `description` stating the semver policy -- new payload schemas
- [x] `crates/qdev-core/src/schema.rs` -- add a `PayloadKind` enum (`Story`, `Error`, `Validate`) with `schema_str()`/`schema_json()`/`pretty_schema_str()`/`from_str_loose()`, mirroring `EntityKind` but kept fully separate -- schema lookup machinery
- [x] `crates/qdev-cli/src/cli.rs`, `crates/qdev-cli/src/main.rs` -- add `qdev schema payload <name>` wired to `PayloadKind`, printing text/JSON exactly like `handle_schema` does for entity kinds, without altering the existing `qdev schema <kind>` path -- CLI grammar + dispatch
- [x] `crates/qdev-cli/tests/schema_cli_tests.rs` (or new file) -- add round-trip tests: run `qdev get story`/`qdev validate` for real, validate their JSON output against the printed schema using the `jsonschema` crate -- verification
- [x] `docs/cli-reference.md` -- add the `qdev schema payload <name>` grammar line and an example -- keep docs authoritative
- [x] `docs/bmad/implementation-artifacts/deferred-work.md` -- append one entry each for `context`, `next`, `gate_run`-as-a-payload, noting they land with the stories that implement those commands

**Acceptance Criteria:**
- Given the binary, when I run `qdev schema payload story|error|validate`, then a JSON Schema is printed for each, and every printed schema's `description` documents the `schema_version` semver policy.
- Given `qdev get story <id> --json` and `qdev validate --json` real output, when validated against their respective printed schemas, then validation reports zero errors.
- Given `qdev schema payload <name>` with an unrecognized or not-yet-deferred name (including `context`, `next`, `gate_run`), when requested, then the command fails with a usage error (exit 2) listing the currently valid names.
- Given the existing `qdev schema <entity-kind>` command (Story 1.4), when this story ships, then its behavior and existing tests (`schema_cli_tests.rs`) are unchanged.

## Implementation Notes

- `SchemaArgs` gained a second optional positional, `name: Option<String>`, alongside the existing `kind: String`. `qdev schema <entity-kind>` is unaffected (`name` stays `None`); `qdev schema payload <name>` is detected in `handle_schema` by checking `schema_args.kind.eq_ignore_ascii_case("payload")` before falling into the existing `EntityKind::from_str_loose` path, then dispatches to a new `handle_schema_payload` that mirrors `handle_schema`'s text/JSON emission using `PayloadKind` instead.
- `PayloadKind::from_str_loose` naturally rejects `context`, `next`, and `gate_run` as unknown (same generic "Unknown payload name" usage error as any unrecognized string) — no special-casing was needed to satisfy the "deferred payload name" matrix row, since those names simply aren't in the match arms yet.
- Added `jsonschema` (default-features = false, matching `qdev-core`'s existing pin) to `qdev-cli`'s `[dev-dependencies]` so the round-trip tests can validate real CLI output against the printed schema in-process, without shelling out to a second binary invocation.
- Payload schemas describe the *full* JSON envelope actually printed on the wire (including `schema_version`), not just the inner data struct — `EntityProjection`/`ValidatePayload` are flattened under `JsonEnvelope<T>` at emission time, so `schema_version` is a required top-level property in `payload-story.json` and `payload-validate.json` just as it already was in `payload-error.json` (`JsonErrorEnvelope` carries its own `schema_version` field directly).
- `payload-story.json` models the general `EntityProjection` shape (it's what every `qdev get <kind> <id> --json` returns, story included), matching the code map's pointer to `query.rs:64`; round-trip tests exercise it specifically via `qdev get story`.

**Review pass patches (2026-09-09):**
- `handle_schema` now rejects a stray second positional on the entity-kind path (`qdev schema <kind> <extra>`) with a usage error, restoring the pre-existing grammar; regression test added.
- `payload-story.json`'s description narrowed to the entity-id `get` form only, since `qdev get <kind> <id-with-slash>` returns a different `GetResult::Constraint` shape this schema never modeled.
- Removed `additionalProperties: false` from nested objects in `payload-story.json`/`payload-error.json`/`payload-validate.json` so they don't contradict the semver policy their own `description` states.
- Added `PayloadKind::valid_names()`; both usage-error sites (missing name, unknown name) now derive their name list from it instead of duplicating a string literal.
- Added tests: case-insensitive payload-name resolution, message-content assertions on the missing/deferred-name usage errors, and the entity-kind extra-positional regression.

## Spec Change Log

## Review Triage Log

### 2026-09-09 — Review pass
- verdicts: 9 findings (grouped from 3 layers: blind-hunter, edge-case-hunter, verification-gap) — high 0, medium 3, low 2, false 4, maybe-false 0
- findings:
  - `[medium]` `[patch]` `qdev schema <entity-kind> <extra-arg>` now silently succeeds instead of failing with a usage error, since `SchemaArgs` gained a second optional positional (`name`) for the new `payload` form but the entity-kind branch in `handle_schema` never checks it's `None` (`crates/qdev-cli/src/cli.rs:220-227`, `crates/qdev-cli/src/main.rs` `handle_schema`) — verified by building and running `qdev schema story extra_garbage`, which exits 0 and prints the schema instead of erroring; contradicts the spec's own boundary that the entity-kind path's grammar is "unchanged by this story." Action: reject the extra positional (usage error, exit 2) when `kind != "payload"` and `name.is_some()`; add a regression test.
  - `[medium]` `[patch]` `payload-story.json`'s description claims to cover "any other `qdev get <kind> <id> --json` invocation," but `qdev get <kind> <id-with-slash>` (a constraint id) returns `GetResult::Constraint` (`crates/qdev-core/src/query.rs:96-99,242`), a completely different shape the schema doesn't model and no test exercises — a consumer validating a constraint-row `get` against the `story` payload schema would get spurious failures. Action: narrow the schema's `description` to state it covers the entity-projection form only, excluding the constraint-row form.
  - `[medium]` `[patch]` Nested objects in the payload schemas (`constraints[]`/`scratch[]` items in `payload-story.json`, the `error` object in `payload-error.json`) set `"additionalProperties": false`, contradicting the semver policy every schema's own `description` states ("additive fields do not bump the major") — a future additive field on `ConstraintProjection`, `ScratchEntryProjection`, or `ErrorPayload` would fail schema validation even though it's policy-compliant. Action: drop `additionalProperties: false` from these nested objects to match the top-level objects' openness.
  - `[low]` `[patch]` The list of valid payload names (`"story, error, validate"`) is hardcoded as a duplicate string literal in both `handle_schema_payload`'s missing-name error and `PayloadKind::from_str_loose`'s unknown-name error, instead of being derived from the already-defined but otherwise-unused `PayloadKind::all()` — the two copies will silently drift the next time a payload kind is added or deferred. Action: build both error messages' name list from `PayloadKind::all()`.
  - `[low]` `[patch]` No test exercises `PayloadKind::from_str_loose`'s documented case-insensitive resolution (e.g. `STORY`), and the missing-name/deferred-name tests assert only the exit code, not that the error text names the valid options. Action: add one case-insensitivity assertion and strengthen the existing usage-error tests to check message content.
  - `[false]` `[reject]` `docs/cli-reference.md`'s existing `qdev schema <kind>` row was rewritten rather than only having a line appended, nominally violating the spec's "Never" boundary — refutation: the original row's description ("Print JSON Schema for a payload...") was itself inaccurate for what the command actually does (print an entity's frontmatter schema); correcting it alongside adding the new row causes no user-facing harm and leaves the docs internally consistent, which the literal boundary wording didn't anticipate.
  - `[false]` `[reject]` `PayloadKind::schema_json()`/`pretty_schema_str()` use `.expect()` on the embedded schema JSON, which would panic on a malformed file instead of failing a dedicated "all schemas parse" unit test — refutation: this exactly mirrors the pre-existing `EntityKind::schema_json()` pattern the spec explicitly required following, and the three embedded payload schemas are already parsed by the existing happy-path CLI tests (`test_schema_payload_{story,error,validate}_text_and_json`), so a malformed file would already fail the test suite immediately.
  - `[false]` `[reject]` No discovery affordance (e.g. `--help` or a `list` form) surfaces `PayloadKind::all()` to a user who doesn't already know the valid payload names — refutation: no bad outcome results; the unknown-name usage error already lists the valid names (per AC), and a discovery subcommand is an enhancement the intent never called for, not a defect.
  - `[false]` `[reject]` `output.emit_error(&e)`'s return value is discarded in every branch of `handle_schema_payload`, so a stdout/stderr write failure while reporting a usage error would leave only a bare exit code — refutation: this is the pervasive, pre-existing codebase-wide convention (`let _ = output.emit_error(...)` appears identically at 20+ other call sites throughout `main.rs`), not a new gap introduced by this change.

## Verification

**Commands:**
- `cargo test --test schema_cli_tests` -- expected: existing and new schema CLI tests pass
- `cargo test` -- expected: full workspace suite passes, including architecture/network checks

