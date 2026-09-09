---
title: '`qdev graph --dot`'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'oneshot'
review_loop_iteration: 0
context:
  - 'docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** There is no way to see the story dependency graph as a whole — `qdev relate`/`unrelate` (story 1.10) manage individual edges, but nothing renders the graph. This was deferred out of story 1.10 to keep that spec in scope; see `docs/bmad/implementation-artifacts/deferred-work.md`.

**Approach:** Add `qdev graph --dot [--epic E12]` (`crates/qdev-cli`): a new read-only `Graph` command that lists `story` entities (optionally filtered to one epic via the existing `EntityFilter.epic_id`, same as `qdev list stories --epic`), fetches their `depends_on`/`extends`/`supersedes` relations restricted to edges between included stories, and prints a Graphviz `digraph` to stdout — nodes labelled with the story id/title and colored by status (draft=lightgray, ready=lightblue, in-progress=yellow, review=orange, done=green, superseded/abandoned=gray45 dashed border), edges labelled with the relation name. This story is `--dot`-only: no `--json` output and no `--sprint`/`--highlight-critical-path` (those belong to story 4.9, backlog, out of scope here). Follow `qdev schema`'s existing precedent (`main.rs`, `handle_schema`) for a command that bypasses the universal JSON envelope and emits raw text directly. Add `crates/qdev-cli/tests/graph_cli_tests.rs` following the `relate_cli_tests.rs`/`get_cli_tests.rs` harness pattern (`qdev init --non-interactive` into a temp workspace, seed story files with frontmatter relations, run the command, assert on the DOT text): cover an epic-filtered graph, an unfiltered graph, and an empty workspace producing a valid empty `digraph {}`.

</frozen-after-approval>

## Implementation Notes

- Added `Commands::Graph(GraphArgs)` (`crates/qdev-cli/src/cli.rs`) with `--dot` and `--epic`; `handle_graph`/`render_graph_dot`/`dot_status_style`/`dot_escape` (`crates/qdev-cli/src/main.rs`) reuse `query_list`/`ListEntryProjection` (same `EntityFilter.epic_id` path as `qdev list stories --epic`) for nodes and `Store::list_relations` for edges, filtered to `depends_on`/`extends`/`supersedes` with both endpoints in the node set.
- `--dot` is required (no other output mode exists yet) and `--json` is explicitly refused (`needs_confirmation`, exit 3) rather than silently ignored, since this story deliberately excludes `--json` output (story 4.9).
- `crates/qdev-cli/tests/graph_cli_tests.rs` (new, 9 tests): unfiltered graph, epic filter in both edge directions, every status color plus `extends`/`supersedes` edges, DOT quote/backslash escaping, empty workspace, and the three refusal paths (`--json` alone, bare `graph`, `--dot --json` together).
- Verified with `cargo test --workspace` (zero failures), `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings), `cargo fmt --check` (clean).

## Review Triage Log

### Blind-hunter pass

- `medium` — `handle_graph` never checked `cli.json`, so `qdev graph --dot --json` silently dropped `--json` and emitted plain DOT text with exit 0 instead of an error or an envelope. Given this story explicitly excludes `--json`, a script passing a global `--json` flag would get an unexpected output shape with no signal. **Fixed**: `handle_graph` now takes `cli: &Cli` and refuses (`needs_confirmation`, exit 3) when `cli.json` is set; covered by `test_graph_dot_with_json_is_refused`.
- `low` — no test covered `ready`/`review`/`superseded`/`abandoned` status colors or the `extends`/`supersedes` edge relations, despite being distinct branches in `dot_status_style`/`GRAPH_EDGE_RELATIONS`. **Fixed**: added `test_graph_dot_status_colors_and_edge_relations`.
- `low` — `dot_escape` (quote/backslash escaping) was untested. **Fixed**: added `test_graph_dot_escapes_quotes_and_backslashes_in_labels`.
- `low` — the epic filter was only tested excluding an *edge into* the filtered set (excluded story depends on an included one); the reverse (included story depends on an excluded one) was unverified. **Fixed**: added `test_graph_dot_epic_filter_excludes_edge_to_story_outside_filter`.
- `low` — bare `qdev graph` (no flags at all) — the most likely real mistake — had no direct test, only `graph --json`. **Fixed**: added `test_graph_bare_no_flags_is_refused`.
- `false` — flagged `fillcolor="gray45"` for `superseded`/`abandoned` as possibly not matching "gray45 dashed border." Rejected: the spec's own wording ("gray45 dashed border") is satisfied literally by `style="filled,dashed"` + `fillcolor="gray45"` — a gray-filled node with a dashed outline — which is what was intended and implemented; no ambiguity to resolve.

### 2026-09-09 — Cross-story review of stories 1.9-1.13 (bmad-review)

**Frozen boundary needs renegotiation — `qdev graph` refusal exit code.**

Boundaries state that `--json` (and, by extension, a missing `--dot`) is refused with
`needs_confirmation`, exit 3. Both refusals now return `usage_error`, exit 2.

Why: AD-13 reserves exit 3 for refusals a human has to act on — preflight, lease, governance,
missing confirmation — and exit 2 for a malformed invocation. Both `graph` refusals are about the
shape of the flags, and every other flag rejection in the binary uses exit 2 (`--expand` with an
unknown value, `schema` with an extra positional, `--changed` combined with `--fix-ids`). Leaving
`graph` on exit 3 taught two contradictory meanings for the same code, so a caller branching on
"a human must confirm something" versus "I passed the wrong flags" was sent down the wrong path
by a plain typo.

The three refusal tests were renamed and now assert exit 2 and a `usage_error` code. If exit 3
was deliberate here, the boundary needs to say what a human is being asked to confirm — nothing
about these two refusals is recoverable by confirming anything.
