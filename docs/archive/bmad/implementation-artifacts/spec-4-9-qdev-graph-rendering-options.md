---
title: 'Story 4.9: qdev graph Rendering Options'
type: 'feature'
created: '2026-10-01'
status: 'done'
baseline_commit: 'fbdf0c7e503dd606dd345dc1b1f7249ef5a7e177'
route: 'dispatch'
review_loop_iteration: 0
context:
  - '{project-root}/docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev graph` only supports basic unfiltered or epic-filtered Graphviz DOT output; it lacks sprint filtering, cannot visually identify blocked stories or active lease holders, does not compute or highlight the longest dependency chain (critical path) for parallel planning tracks, and explicitly rejects `--json`, preventing integration with external renderers and agent tooling.

**Approach:** Extend `qdev graph` in `qdev-core` and `qdev-cli` with `--sprint <ID>` filtering, blocked story distinction (`peripheries=2` and `[BLOCKED]` in label), active lease holder annotation (`[lease: <holder>]` in label), and `--highlight-critical-path` calculation and rendering (`color="red", penwidth=2.0` on critical path nodes and edges); add full `--json` support emitting a structured `GraphPayload` (`nodes` and `edges`) in the standard JSON envelope, backed by a new `payload-graph.json` schema.

## Boundaries & Constraints

**Always:**
- Keep `qdev-core` headless and deterministic: graph filtering, blocked evaluation, lease resolution, critical path calculation, and JSON projection happen in `qdev-core::graph`.
- Support `--sprint <ID>` accepting sprint numbers (`5`) or canonical IDs (`sprint-5`) via `normalize_sprint_id`, filtering nodes to stories assigned to that sprint.
- Compute `blocked` per story using existing semantics: a story is blocked iff its status is not `done` and at least one `depends_on` dependency is not live-`done`.
- Resolve active story leases via `list_leases_with_storage`, attaching `lease_holder` to matching story nodes.
- When `--highlight-critical-path` is enabled, compute the longest directed dependency chain among `depends_on` relations in the filtered graph; break ties deterministically (lexicographically by story ID sequence); mark participating nodes and edges as `critical_path = true`.
- In DOT output:
  - Blocked stories must be visually distinct with `peripheries=2` and `\n[BLOCKED]` appended to node label.
  - Leased stories must display `\n[lease: <holder>]` appended to node label.
  - Critical path nodes and edges (when `--highlight-critical-path` is active) must be highlighted with `color="red", penwidth=2.0`.
  - Preserve existing status fill colors (`green`, `yellow`, `lightblue`, `orange`, `gray45`, `lightgray`) and styles (`solid`, `dashed`).
- In JSON mode (`qdev graph --json`):
  - Emit standard `JsonEnvelope<GraphPayload>` with `nodes` and `edges` arrays.
  - Each node includes `id`, `title`, `status`, `blocked`, `lease_holder`, `critical_path`, and `epic_id`.
  - Each edge includes `source`, `target`, `relation`, and `critical_path`.
  - Include top-level `critical_path` array (ordered story IDs) when `--highlight-critical-path` is passed.
- Register `PayloadKind::Graph` ("graph") and provide `crates/qdev-core/schemas/payload-graph.json` so `qdev schema payload graph` emits the valid schema.
- Require either `--dot` or `--json` output flag; bare `qdev graph` without either flag exits 2 (`usage_error`); combining `--dot` and `--json` exits 2 (`usage_error`).

**Never:**
- Never modify existing frontmatter or database state; `qdev graph` is strictly a read-only query and projection command.
- Never traverse or highlight non-`depends_on` relations as dependency chains (relations like `extends` or `supersedes` remain rendered edges, but do not form dependency chains).
- Never allow non-deterministic ordering: nodes, edges, and critical path tie-breaking must be stable across runs.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Sprint filter DOT | `qdev graph --dot --sprint 5` | Only stories assigned to sprint 5 and edges between them emitted in DOT | exit 0 |
| Sprint filter JSON | `qdev graph --json --sprint 5` | Envelope with `nodes` and `edges` for sprint 5 stories | exit 0 |
| Blocked and leased story DOT | Blocked story leased by simon | Node has `peripheries=2`, label has `[BLOCKED]` and `[lease: simon]` | exit 0 |
| Critical path DOT | `--dot --highlight-critical-path` | Longest `depends_on` chain nodes and edges have `color="red", penwidth=2.0` | exit 0 |
| Critical path JSON | `--json --highlight-critical-path` | Participating nodes/edges have `critical_path: true`; payload has `critical_path: [...]` | exit 0 |
| Both filters combined | `--dot --epic E1 --sprint 5` | Intersected stories belonging to E1 and assigned to sprint 5 | exit 0 |
| Invalid sprint format | `qdev graph --dot --sprint abc` | Usage error naming invalid sprint identifier | exit 2, `usage_error` |
| Empty filter values | `qdev graph --dot --epic ""` | Usage error rejecting empty filter value | exit 2, `usage_error` |
| Conflicting flags | `qdev graph --dot --json` | Usage error: cannot combine `--dot` and `--json` | exit 2, `usage_error` |
| Missing output flag | `qdev graph` | Usage error: requires either `--dot` or `--json` | exit 2, `usage_error` |
| Empty workspace | `qdev graph --json` in empty project | `{"schema_version":"1","nodes":[],"edges":[]}` | exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/graph.rs` (new) -- Graph domain models (`GraphNode`, `GraphEdge`, `GraphPayload`), headless `build_story_graph` query engine, longest dependency chain DFS algorithm, and DOT renderer helper -- Core graph engine.
- `crates/qdev-core/src/lib.rs` -- Export `build_story_graph`, `render_graph_dot`, `GraphPayload`, `GraphNode`, `GraphEdge` -- Core library exports.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Graph` in enum, `all()`, and `from_str_loose` -- Schema payload registration.
- `crates/qdev-core/schemas/payload-graph.json` (new) -- Hand-authored JSON Schema for graph payload -- Schema artifact.
- `crates/qdev-cli/src/cli.rs` -- Update `GraphArgs` with `--sprint` and `--highlight-critical-path` -- CLI argument parsing.
- `crates/qdev-cli/src/main.rs` -- Update `handle_graph` to validate flags, call `build_story_graph`, and emit either DOT text or JSON envelope -- CLI handler wiring.
- `crates/qdev-cli/tests/graph_cli_tests.rs` -- Integration tests for sprint filtering, blocked styling, lease annotations, critical path highlighting, JSON output, and schema validation -- CLI integration coverage.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/graph.rs` -- Implement `GraphNode`, `GraphEdge`, `GraphPayload`, `build_story_graph`, critical path finder, and DOT rendering logic -- Core graph domain model and computation.
- [x] `crates/qdev-core/src/lib.rs` & `schema.rs` -- Re-export graph module symbols and add `PayloadKind::Graph` -- Core module registration.
- [x] `crates/qdev-core/schemas/payload-graph.json` -- Define schema for graph payload -- Schema validation.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `sprint` and `highlight_critical_path` arguments to `GraphArgs` -- CLI flag definition.
- [x] `crates/qdev-cli/src/main.rs` -- Wire `handle_graph` to core graph engine and handle `--dot` / `--json` emission -- CLI command dispatch.
- [x] `crates/qdev-cli/tests/graph_cli_tests.rs` -- Add tests verifying all acceptance criteria: sprint filtering, blocked distinction, lease holder, critical path highlighting, and JSON emission -- Test verification.

**Acceptance Criteria:**
- Given Story 1.10's DOT output, when running `qdev graph --dot --sprint 5 --highlight-critical-path`, then the graph is filtered to sprint 5, blocked stories have `peripheries=2` and `[BLOCKED]` in label, leased stories show `[lease: <holder>]`, and the longest dependency chain is highlighted with `color="red", penwidth=2.0`.
- Given `qdev graph --json [--sprint <N>] [--highlight-critical-path]`, then valid JSON envelope containing `nodes` and `edges` matching `payload-graph.json` is emitted to stdout.
- Given bare `qdev graph` or `qdev graph --dot --json`, then command exits 2 with a structured usage error.
- Given `qdev schema payload graph`, then the JSON Schema for graph payload is emitted to stdout.

## Implementation Notes

- Implemented `crates/qdev-core/src/graph.rs` providing `GraphNode`, `GraphEdge`, `GraphPayload`, `StoryGraphOptions`, `build_story_graph`, and `render_graph_dot`.
- Headless `build_story_graph` handles sprint and epic filtering, computes `blocked` via `get_live_entity_for_derivation` (unmet `depends_on`), attaches active leases from `list_leases_with_storage`, and calculates longest directed dependency chains via deterministic memoized DFS.
- Implemented DOT rendering with `peripheries=2` and `[BLOCKED]` for blocked stories, `[lease: <holder>]` for leased stories, and `color="red", penwidth=2.0` on critical path nodes and edges.
- Added `crates/qdev-core/schemas/payload-graph.json` and registered `PayloadKind::Graph` across `qdev-core::schema`.
- Updated `GraphArgs` in `crates/qdev-cli/src/cli.rs` with `--sprint` and `--highlight-critical-path`.
- Updated `handle_graph` in `crates/qdev-cli/src/main.rs` to validate output flags (`--dot` or `--json`, mutually exclusive), empty filter values, and sprint ID normalization, delegating to `build_story_graph`.
- Added comprehensive integration tests in `crates/qdev-cli/tests/graph_cli_tests.rs` covering sprint filtering, blocked/lease decorations, critical path calculation and styling in DOT and JSON, empty workspace, and schema validation.
- All workspace tests (`cargo test`), clippy checks (`cargo clippy --workspace --all-targets -- -D warnings`), and formatting checks (`cargo fmt --check`) pass cleanly.

## Spec Change Log

## Review Triage Log

## Design Notes

Critical path determination finds the longest directed path in the `depends_on` subgraph of included stories. Because `qdev relate` and hydration sweeps prevent cycles, the subgraph is an unweighted DAG. A memoized DFS computes the maximum chain length from each node, breaking ties deterministically by comparing candidate node ID sequences lexicographically. Edges and nodes on the chosen chain are tagged with `critical_path: true` and rendered with `color="red", penwidth=2.0`.

Blocked stories are computed using the live SQLite cache: any story not in status `done` with at least one unmet `depends_on` requirement is marked `blocked: true`, receiving `peripheries=2` and `[BLOCKED]` in Graphviz DOT. Leases are looked up via `list_leases_with_storage` against the workspace root, appending `[lease: <holder>]` to the label.

## Verification

**Commands:**
- `cargo test --test graph_cli_tests` -- expected: all existing and new graph CLI tests pass.
- `cargo test --test schema_cli_tests` -- expected: `qdev schema payload graph` tests pass.
- `cargo test` -- expected: complete test suite passes with zero regressions.
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero clippy warnings.
- `cargo fmt --check` -- expected: formatting clean.
