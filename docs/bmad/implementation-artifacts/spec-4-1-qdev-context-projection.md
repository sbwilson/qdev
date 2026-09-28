---
title: 'Story 4.1: `qdev context` Projection'
type: 'feature'
created: '2026-09-28'
status: 'done'
route: 'dispatch'
baseline_commit: ecbff4bbe274ca1a02e71765ecfa95624bf4f762
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Agents must read whole repositories or hand-assemble context from dozens of Markdown files; no command produces exactly the phase-specific context an agent needs within a token budget, so qdev's core token-efficiency thesis is unimplemented.

**Approach:** Add `qdev context <story-id> --phase specify|develop|review [--budget N] [--stats] [--format md] [--json]` that projects a deterministic, token-budgeted payload of priority-ordered sections from the hydrated cache plus entity files, truncating lowest-priority sections first and listing what was truncated. Assembly, budgeting, and rendering live in qdev-core; the CLI is a thin handler.

## Boundaries & Constraints

**Always:**
- Target must be a story id; any other entity kind is a usage error.
- Deterministic: fixed section priority order, ids/seq/path-sorted collections, no timestamps in output.
- Token estimation uses the documented estimator `qdev_core::scratch::estimate_tokens` (chars/4, div_ceil).
- Read-only: no cache writes, no file writes, no git mutations, never prompts (safe in non-interactive mode).
- Develop sections in AC priority order: story spec body, constraints (own + inherited, full IDs), resolved module paths, governing ADR excerpts (Rule + Prevents), linked requirement titles, scratchpad summary, bound gates, hygiene directive.
- `--phase specify`/`--phase review` include the section sets from `docs/architecture.md` §12 (specify: epic goal+constraints, sibling story titles, ADR summaries, linked requirements; review: develop set + diff summary, gate receipts, evidence paths).
- Default budget per phase when `--budget` is omitted: specify 800, develop 1200, review 2500 (§12 typical budgets).

**Never:**
- No `[hygiene]`/`[synthesis]` config keys (4.3/4.8 own those) — the directive is a built-in default constant.
- No diff content, full scratchpad ledger, model names, or persona text in the payload.
- No changes to gate execution, the state machine, or hydration.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| develop happy path | `qdev context E12S4 --phase develop --budget 1200 --json` in a workspace | Envelope payload with sections in priority order, per-section tokens, `truncated` list; exit 0 | N/A |
| over budget | section sum > `--budget` | Lowest-priority sections truncated (line-trimmed) or dropped until sum fits; `truncated` records each in drop order | exit 0 |
| top section alone > budget | spec body exceeds budget | Highest-priority section kept whole; sum may exceed budget; `--stats` surfaces the overrun | exit 0 |
| review, no git baseline | merge-base vs integration branch unresolvable | diff summary section present but empty with a reason note | exit 0 |
| markdown | `--format md` (any phase) | Deterministic Markdown rendering of the same payload | N/A |
| both `--json` and `--format md` | conflicting output modes | exit 2 usage error | usage error |
| non-story target | `qdev context E12 --phase develop` | exit 2 naming the kind and that a story is required | usage error |
| unknown id | `qdev context E99S9 --phase develop` | exit 1 entity not found | logical error |
| outside a workspace | no `qdev.toml` ancestor | existing boot guard fires | usage error, exit 2 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/schema.rs` -- `PayloadKind` enum + `schema_str()` includes (~L145; stale comment claims `context` "intentionally absent" — update); `extract_frontmatter_str` (L381)
- `crates/qdev-core/src/envelope.rs` -- `JsonEnvelope<T>` / `JsonErrorEnvelope`
- `crates/qdev-core/src/errors.rs` -- `ExitCode` (0/1/2/3/4/5), `QdevError` constructors
- `crates/qdev-core/src/query.rs` -- `build_entity_projection` (~L204): own + inherited-epic constraints tagged `inherited_from` via `get_constraints_for_owner` — reuse the pattern
- `crates/qdev-core/src/scratch.rs` -- `estimate_tokens` (L112), `summarize_scratch_entries(entries, last_n, budget)` (L355) — reuse for scratchpad summary + budgeting
- `crates/qdev-core/src/store/mod.rs` -- `Store` trait: `get_entity`, `list_entities`, `get_constraints_for_owner`, `get_relations_for_source`, `get_scratchpad_entries`, `list_gates`, `get_gate_runs_for_story`; `EntityRecord` (`source_path`, `epic_id`, `seq`), `GateRunRecord`
- `crates/qdev-core/src/modules.rs` -- `ModuleRegistry` (`get`, `to_module_paths_map`) from `Config.modules`
- `crates/qdev-core/src/config/types.rs` -- `Config` (L345), `HygieneConfig` (L142), `GateConfig.on_transition` (L266)
- `crates/qdev-core/src/transition.rs` -- L299: `on_transition == "review"` gate filter = the develop bound-gate set
- `crates/qdev-core/src/gate/builtin/mod.rs` -- `BUILTIN_GATE_SCOPE/DEPS/HYGIENE` (run first, unconditionally)
- `crates/qdev-core/src/hygiene/mod.rs` -- `resolve_diff_files` (merge-base vs `[git] integration_branch`) pattern for the diff summary
- `crates/qdev-core/src/next.rs` -- file-read precedent: `root.join(&entity.source_path)` + `extract_frontmatter_str` (~L252)
- `crates/qdev-cli/src/cli.rs` -- `Cli` (global `--json`), `Commands` enum (L36), `GraphArgs` (L421) per-command-flag pattern
- `crates/qdev-cli/src/main.rs` -- dispatch (L143), `requires_workspace` (L556), `handle_get` (L1777) boot sequence: workspace guard → `open_query_store` → core call → `OutputEmitter`; text renderers (~L2517)
- `crates/qdev-core/tests/query_tests.rs` -- `setup_e12s4` (L49) reference fixture: E12S4 under E12, NG-1 + inherited E12/RH-2, depends_on E12S3, traces_to FR-102, governed_by AD-43, modules bridge/foundation; `tests/fixtures/story/valid.md` is the E12S4 file
- `crates/qdev-core/schemas/` -- add `payload-context.json`
- `docs/architecture.md` §12 (L599), `docs/compliance-and-safety.md` §1 (L18; directive quote ~L60), `docs/cli-reference.md` (row L53; MCP list L623)

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/context.rs` (new) -- `build_context(...)`: resolve story (usage error if other kind, logical error if absent), read spec body from `source_path` (frontmatter stripped), assemble phase sections in priority order, apply budget by line-trimming/dropping lowest-priority sections first, render text + Markdown; re-export in `lib.rs` -- the projection must be pure core (no I/O beyond file reads)
- [x] `crates/qdev-cli/src/cli.rs` -- `ContextArgs { id, phase, budget: Option<u32>, stats: bool, format: Option<String> }` + `Commands::Context` variant -- clap surface for the projection
- [x] `crates/qdev-cli/src/handlers/context.rs` (new) + `handlers/mod.rs` + `main.rs` -- handler mirroring `handle_get`'s boot sequence; dispatch arm; `requires_workspace` arm `true` -- thin CLI wiring
- [x] `crates/qdev-core/src/schema.rs` + `crates/qdev-core/schemas/payload-context.json` -- `PayloadKind::Context` + payload JSON schema; fix the stale deferral comment -- 1.13 invariant: shipped payloads are schematized
- [x] `crates/qdev-core/tests/context_tests.rs` (new) -- reference-fixture develop sum ≤ 1,200 tokens, per-phase section sets, truncation order + `truncated` list, `--stats` numbers, byte-identical repeat builds, I/O-matrix error cases
- [x] `docs/cli-reference.md` -- context row gains `--format md`; document phases, default budgets, estimator, truncation semantics, payload shape -- agents and users discover the contract

**Acceptance Criteria:** (verbatim intent from epics.md Story 4.1)
- Given story E12S4, when `qdev context E12S4 --phase develop --budget 1200 --json`, then the payload contains, in priority order: the story spec body, constraints with IDs including inherited ones, resolved module paths, governing ADR excerpts (Rule and Prevents), linked requirement titles, a scratchpad summary, bound gates, and the hygiene directive from `docs/compliance-and-safety.md` §1; and `--phase specify` / `--phase review` include the §12 sections; and over budget, lowest-priority sections truncate first with the payload listing what was truncated; and `--stats` reports per-section estimated tokens with the reference fixture under 1,200 for develop; and output is deterministic with a Markdown mode (`--format md`).

## Implementation Notes

## Spec Change Log

## Review Triage Log

- `crates/qdev-cli/tests/context_cli_tests.rs:604` -- medium: Pre-verified gap: `test_context_is_read_only_across_repeated_runs` does not assert `sync_meta` remains unmutated across `qdev context` runs. (route: patch)
- `crates/qdev-core/src/context.rs:1029` -- low: Pre-verified gap: Truncation rendering in Text and Markdown modes is unasserted by tests. (route: patch)
- `crates/qdev-core/src/context.rs:1066` -- low: Pre-verified gap: Markdown `--stats` table rendering is unasserted by tests. (route: patch)
- `crates/qdev-core/src/context.rs:725` -- medium: Untracked file path in git status porcelain may be wrapped in quotes for paths with spaces, breaking `fs::read_to_string`. (route: patch)
- `crates/qdev-core/src/context.rs:456` -- medium: Linked requirement entity without title is misreported as `(unresolved)` dangling reference even though entity row exists. (route: patch)
- `crates/qdev-core/src/context.rs:1030` -- medium: Truncation headers claim "lowest priority first" while `apply_budget` records in walk order (priority 1..N). (route: patch)
- `crates/qdev-cli/src/main.rs:263` -- false: Uninitialized cache crash: query commands mirror `handle_get` and fail on missing tables if `qdev sync` never ran; standard workspace behavior. (rejected)
- `crates/qdev-core/src/context.rs:735` -- low: Untracked binary files are silently omitted from diff section changes map if `read_to_string` fails. (route: patch)
- `crates/qdev-core/src/context.rs:680` -- medium: Git rename syntax in `diff --numstat` can produce `old => new` path syntax unless `--no-renames` is passed. (route: patch)
- `crates/qdev-core/src/context.rs:282` -- medium: Empty story spec returns `(spec body is empty)` instead of the uniform `(none)` empty section marker. (route: patch)
- `crates/qdev-core/src/context.rs:1061` -- false: Markdown heading collision: Level-2 headings for top sections are required by spec design. (rejected)
- `crates/qdev-core/src/context.rs:381` -- low: Multi-line ADR rule in specify summary breaks one-line-per-ADR structure. (route: patch)
- `crates/qdev-core/src/context.rs:200` -- medium: Headings inside fenced code blocks in ADR/epic markdown bodies could prematurely truncate extraction. (route: patch)
- `crates/qdev-core/src/context.rs:504` -- false: Shadowed built-in gates: built-ins run unconditionally first per Code Map L63 and Design Note 96. (rejected)
- `crates/qdev-core/src/context.rs:645` -- low: Empty strings in gate run evidence paths produce empty lines in evidence section. (route: patch)
- `crates/qdev-core/src/context.rs:536` -- false: Stale entities in sibling stories: Story 1.20 specifies retained stale rows are surfaced in reporting. (rejected)
- `crates/qdev-cli/tests/context_cli_tests.rs:789` -- low: Missing CLI test coverage for `--format md --stats`. (route: patch)

## Design Notes

- **Budget algorithm:** walk sections highest→lowest priority; each takes complete lines that fit the remaining budget; a section that cannot fit is dropped and recorded in `truncated` (drop order, names + full tokens). The first section is exempt: if it alone exceeds the budget it is kept whole (exit 0; `--stats` shows the overrun). Deterministic line trimming only — never mid-line splits.
- **Hygiene directive (develop):** built-in `&str` constant, verbatim from `docs/compliance-and-safety.md` §1 ("Write standard code comments. Cite entities with compact bracket tags such as `[E12S4]`, `[AD-43]`, `[DEC-2b91]`. Never write narrative history…"). 4.3 makes it config-overridable; both ship the same default text.
- **ADR excerpt:** `decision` frontmatter field is the Rule; `prevents` frontmatter field is Prevents (schema-declared, no code reads them yet). If the ADR body has `## Rule`/`## Decision`/`## Prevents` headings, body sections win over frontmatter.
- **Bound gates (develop):** built-ins `qdev-scope`/`qdev-deps`/`qdev-hygiene` first (execution order), then configured `[[gates]]` with `on_transition` containing `review` (transition.rs:299 semantics), id-sorted; entries carry id, kind, transitions.
- **Review extras:** diff summary = files changed since merge-base with `[git] integration_branch` with per-file +/− line counts (numstat; reuse the hygiene `resolve_diff_files` pattern; unresolvable → empty + note); gate receipts = latest run per gate (id, status, summary); evidence paths = the runs' evidence paths.
- **Output modes:** default text and `--format md` are two deterministic renderings of one payload built in core; `--json` emits the envelope; `--json` + `--format md` together is a usage error.
- **`--stats`:** JSON gains a `stats` object (budget, total, per-section tokens) only when the flag is set; text mode prints the stats table.
- **Dangling relation targets** (e.g. `governed_by`/`traces_to` pointing at an entity with no live row): the section keeps the target id, omits the unresolvable text, marks it `(unresolved)` — a dangling reference never fails the payload.
- **Empty sections** (no scratchpad entries, no gates, no relations): the section is still present with a `(none)` marker, so the payload's section set depends only on the phase, not on data shape.
- **`--budget 0`/negative:** usage error, exit 2.

## Verification

**Commands:**
- `cargo build --workspace` -- expected: compiles
- `cargo test -p qdev-core context` -- expected: new suite green (budget, truncation, stats, determinism, errors)
- `cargo test --workspace` -- expected: no regressions (clap, payload schema list, envelope tests)
- Scratch workspace check: `git init` + `qdev init` in a temp dir, create a story, run `qdev context <story> --phase develop --budget 1200 --stats --format md` -- expected: exit 0, sections in order, stats table under budget
