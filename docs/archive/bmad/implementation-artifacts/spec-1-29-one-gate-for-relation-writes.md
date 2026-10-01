---
title: 'One gate for relation writes'
type: 'bugfix'
created: '2026-09-12'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '188530cff6ebe8debd91bd10193f0a591b32f23e'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-context.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev relate` refuses a dangling target, an invalid source/target kind pair, and a dependency cycle before it writes, but `qdev update --field relations=…` replaces the relation map without those checks. It can therefore commit exactly the graph states that the dedicated command reports as logical failures, leaving a workspace that only later `validate` rejects.

**Approach:** All command-driven relation-map mutations will pass through one proposed-graph validation gate before any file, cache, or version change. The generic update surface will keep its whole-map replacement semantics, but it will be evaluated with the same relation-name, kind-pair, target-existence, and dependency-cycle rules as `relate`.

## Boundaries & Constraints

**Always:**
- A relation edge accepted through either command is valid against the proposed complete graph: each relation name is known, each source/target kind pair is allowed, every target exists, and `depends_on` contains no cycle.
- A rejected `update --field relations=…` leaves frontmatter bytes, cache rows, dirty state, and entity version unchanged, and uses the existing structured logical/usage failure contract for the matching guard.
- `relate` and relation-map updates invoke the same reusable gate; adding a later relation guard has one implementation site and tests must demonstrate both command surfaces use it.
- When a full `relations` replacement removes or changes edges, validation assesses that final map rather than treating it as an independent sequence of additions.
- The existing graph validator during hydration remains the backstop for hand-edited files; this change does not redefine its finding semantics.

**Never:**
- Do not silently drop, repair, or partially apply invalid relation entries.
- Do not make arbitrary `--field` writes validate or mutate unrelated frontmatter fields.
- Do not change relation schemas, allowed relation pairs, JSON envelopes, exit-code categories, or the write lock/atomic-write protocol.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Valid replacement | Existing story has relations; `update --field 'relations={depends_on: [E1S1]}'` is a valid final graph | The complete requested map replaces the old map, write succeeds, and normal hydration derives the corresponding graph | N/A |
| Dangling target | A replacement names `E9S9`, which is absent | No write; command reports `dangling_relation`, as `relate` does | Existing logical-failure envelope and exit code |
| Invalid kind pair | A replacement names a known relation whose endpoints are disallowed | No write; command reports `invalid_relation_kind` | Existing logical-failure envelope and exit code |
| Cycle in final map | A replacement makes the source participate in a `depends_on` cycle | No write; command reports `dependency_cycle` | Existing logical-failure envelope and exit code |
| Relation removal | A replacement removes an edge that was part of the cached graph | The final graph, not the former graph, is checked; a valid removal succeeds and cache rows match the file | N/A |
| Case-variant canonical key | `--field Status=ready` is supplied while the canonical key is `status` | The command rejects the non-canonical spelling and never appends an ignored second key | Usage failure naming the canonical key |

**Decision:** A `--field` key whose ASCII case differs from a canonical frontmatter key is rejected as a usage error. Canonicalization is not performed: a formerly ignored input must not become a mutation.

</frozen-after-approval>

## Code Map

- `crates/qdev-cli/src/main.rs:894-1004` -- `handle_update` parses arbitrary YAML `--field` values, rejects only managed keys, and dispatches directly to the generic core writer; its exact-case key patching causes the `Status` duplicate-key escape.
- `crates/qdev-cli/src/main.rs:1469-1614` -- `validate_relation_name` and `handle_relate` contain the current pre-write gate: entity lookups, kind-pair and dangling checks, then dependency-edge cycle detection. Extract or redirect this behaviour rather than copying it.
- `crates/qdev-core/src/dag.rs:11-69,154-205` -- authoritative relation-pair table and reusable pure predicates (`is_known_relation`, `is_valid_kind_pair`, `would_create_cycle`); do not add a second table or graph algorithm.
- `crates/qdev-core/src/write.rs:1617-1814` -- generic update patches, schema-validates, atomically writes, and marks cache dirty, but cannot make a relation-aware decision today.
- `crates/qdev-core/src/write.rs:2324-2708` -- dedicated relation writer merges a single edge and updates its relation-cache row; preserve its atomicity and no-op/version guarantees while directing it through the shared validation path.
- `crates/qdev-core/src/store/sqlite.rs:4347-4532` -- private hydration backstop for persisted graph findings. It is not the command gate and must not be called to mutate findings during a rejected write.
- `crates/qdev-cli/tests/relate_cli_tests.rs` -- existing public contract fixtures for dangling, invalid-kind, and cycle refusal.
- `crates/qdev-cli/tests/update_cli_tests.rs:704-1015` -- generic-field, schema, managed-key, conflict, and key-validation coverage; extend with relation-gate and canonical-case regression cases.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/dag.rs` and/or the smallest appropriate shared core seam -- expose one proposed-relation-map validation operation built from the existing relation table and cycle algorithm -- make the graph contract reusable without duplicating guards.
- [x] `crates/qdev-cli/src/main.rs` -- construct the intended final relation map for both `relate` and `update --field relations=…`, invoke the shared gate before either write path, and reject non-canonical case variants of canonical generic keys -- close both command-surface escapes.
- [x] `crates/qdev-core/src/write.rs` -- only if required by the shared seam, preserve all current frontmatter, lock, atomicity, cache, and version invariants while accepting prevalidated relation changes -- avoid a second validation implementation.
- [x] `crates/qdev-cli/tests/relate_cli_tests.rs` and `crates/qdev-cli/tests/update_cli_tests.rs` -- cover valid full-map replacement/removal plus dangling, invalid-kind, and cycle refusal through update, asserting no mutation after every refusal; add the approved case-variant regression -- prove the shared contract.

**Acceptance Criteria:**
- Given a relation graph, when the same invalid edge is attempted through `relate` and `update --field relations=…`, then both refuse it with the same error code/category and neither changes the workspace.
- Given a full relation-map replacement that changes multiple edges, when its final graph is valid, then the update succeeds atomically and immediate reads observe exactly that map.
- Given an existing dependency cycle that a map replacement removes, when the proposed final graph has no cycle, then the update is accepted.
- Given a future caller uses the shared relation-validation operation, when it proposes an invalid name, pair, target, or dependency graph, then it receives the same single-source rule rather than a copied guard list.
- Given the workspace, when `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check` run, then all are clean.

## Implementation Notes

Planning choice: validation must operate on the proposed complete source relation map, replacing the source's prior edges in the graph before checking cycles. Running `would_create_cycle` against the unmodified cached graph would incorrectly reject a valid replacement that removes an old cycle edge.

Implemented a pure `qdev_core::validate_proposed_relation_map` over supplied entity and relation data. Both `relate` and a well-formed generic `relations` replacement now construct the same proposed map and call it before the writer; malformed YAML retains the generic writer's schema-error path. The existing generic writer required no code change: it retains its lock, atomic write, cache-dirty, and version handling after successful preflight.

The canonical-key rejection list covers every top-level key in the embedded entity schemas, so no recognized field can be silently appended under a different ASCII case.

## Design Notes

The dedicated command currently validates one candidate edge against cache data, while generic update replaces a whole YAML map. The shared seam should normalize both forms to the same question: “is this source entity’s proposed relation map valid in the graph that would exist after this write?” The existing SQLite sweep remains the safety net for manual edits; using it as a preflight would incorrectly mutate findings and inspect persisted rather than proposed state.

## Verification

**Commands:**
- `cargo test --workspace` -- passed outside the sandbox; its local-listener network isolation test cannot bind in the sandbox
- `cargo clippy --workspace --all-targets -- -D warnings` -- passed
- `cargo fmt --check` -- reports pre-existing formatting drift in unrelated `validate_cli_tests.rs`, `sqlite.rs`, `validate.rs`, `write.rs`, and `sweep_tests.rs`; modified story files are formatted

## Review Triage Log

- **low / patch** — `update --field relations={unknown_relation: [...]}` reaches the shared unknown-name gate but had no update integration test. Add the same no-mutation assertion used for the other three update refusals.
- **medium / patch** — final-graph cycle removal was covered only by the pure core gate test; an `update` fixture must prove the CLI constructs a replacement map rather than validating old source edges.
- **high / defer** — relation validation occurs before the pre-existing advisory write lock, so concurrent writers can validate the same snapshot then commit a cycle. This was already true for `relate`; closing it needs a transactional redesign rather than a local guard.
- **medium / defer** — generic `apply_entity_update` leaves relation rows stale until the next boot sweep, unlike the dedicated writer. The behavior predates this story and normal CLI invocations hydrate the dirty row; immediate in-process cache consistency is deferred.
- **medium / patch** — a combined `--field kind=… --field relations=…` update preflights using cached old source kind although the generic writer validates patched kind. Derive the proposed kind for the shared gate and add a regression fixture.
- **medium / patch** — the initial manual canonical-key list omitted valid entity-schema keys, including `gate`, `kind`, `story_id`, and `seq`; complete the inventory and pin a previously omitted spelling.
- **false** — rejecting a write when the proposed complete graph retains an unrelated cycle is intentional: the frozen rule requires the complete proposed `depends_on` graph to contain no cycle, not merely that this command introduce none.
- **false** — duplicate `--field relations` arguments cannot validate one value and write another: `patch_frontmatter` stores updates in a map with last occurrence winning, matching the preflight’s reverse search.
- **low / patch** — malformed relation-map values deliberately take the generic schema-validation path, but that documented branch has no non-mutation regression test. Add one scalar or malformed-map assertion.
- **medium / patch** — the edge-case review independently confirmed `Gate` was absent from the canonical-key list; it is covered by the complete inventory fix and needs a regression assertion.
- **high / defer** — the edge-case review independently confirmed the pre-lock concurrency window. It is the same pre-existing relation-write race recorded above; a transactional redesign is required to close it.
- **medium / defer** — `cargo fmt --check` fails on unrelated baseline formatting drift. The story files are formatted; restoring repository-wide formatter cleanliness is separate work.
