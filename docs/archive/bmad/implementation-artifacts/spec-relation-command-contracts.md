---
title: '`relate` and `unrelate` honour their contracts'
type: 'bugfix'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'e6b17726caf6ff5d1c65435c1e8d6222feae9a1b'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/cli-reference.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** the two commands that manage relations both report success for work they did not do. Pass 1 of the epic 1 cross-story review filed these as H8 and M1, both reproduced; pass 2 re-verified both as still reproducing, and found H8 had never been scheduled and M1's review-by point had elapsed with the story it named shipping untouched.

- **`--if-version` is not a fence on `relate`.** `apply_relation_change` returns its idempotent no-op *before* comparing the version, so `qdev relate E1S1 depends_on E1S2 --if-version 99` on an edge that already exists exits 0 reporting `changed: false, version: 2` — no comparison, no refusal. `qdev update --if-version 99` on the same entity exits 5 `version_mismatch`. An agent using the flag as a compare-and-swap fence reads that exit 0 as confirmation its expected version was current. `unrelate` rejects `--if-version` as an unknown argument, so the pair the docs present together have different concurrency contracts.
- **A typo'd relation name is silent on `unrelate` and misdescribed on `relate`.** `qdev unrelate E1S1 dependson E1S2` exits 0 reporting `changed: false` while the real `depends_on` edge survives — indistinguishable from the legitimate idempotent case, so a cleanup script reports the edge removed. `qdev relate` with the same input exits 1 `invalid_relation_kind` saying the relation "is not an allowed kind pair", which is false: it is not a relation at all. `allowed_kind_pairs` returns an empty slice for both an unknown name and `verifies` (a known relation with no pairs yet), so the caller cannot tell them apart.

**Approach:** an unknown relation name is a usage error naming the valid ones, from both commands, because that is what every other enum-valued argument in the binary does. `--if-version` is compared before any outcome is reported, including the no-op, and `unrelate` accepts it too. A genuinely disallowed *pair* keeps its own code and its own message.

## Boundaries & Constraints

**Always:**
- One list of relation names, beside `allowed_kind_pairs` so the two cannot drift, and both commands validate against it.
- An unknown relation name is a usage error (exit 2) naming the valid relations, from `relate` and `unrelate` alike — the same class as an unrecognised `--author-type` or entity kind.
- `--if-version` is honoured before any outcome is reported: a mismatch is exit 5 `version_mismatch` whether or not the requested change would have altered the file.
- `unrelate` accepts `--if-version` with the same meaning it has on `relate` and `update`.
- Removing an edge that is genuinely absent, with a *valid* relation name, stays an idempotent success (exit 0, `changed: false`). That behaviour was never the defect; being indistinguishable from a typo was.
- `invalid_relation_kind` keeps its meaning: a real relation whose source and target kinds are not an allowed pair. `verifies`, which has no allowed pairs yet, is a known relation and must not be reported as unknown.

**Never:**
- No change to the relation set itself, to `allowed_kind_pairs`' contents, or to the DAG rules.
- No new relation names, no change to the frontmatter shape, and no change to how relations are stored or hydrated.
- `apply_relation_change` keeps accepting any relation name it is given: its job is to merge and write what it was asked for, and the enum belongs at the command surface, as `--author-type`'s does.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Fence on a no-op relate | edge already present, `--if-version` stale | `version_mismatch`, exit 5 — not exit 0 | Exit 5 |
| Fence on a real relate | edge absent, `--if-version` stale | `version_mismatch`, exit 5, nothing written | Exit 5 |
| Fence satisfied | `--if-version` matches | the change applies, version bumped | N/A |
| Fence on unrelate | `unrelate --if-version` stale | `version_mismatch`, exit 5 | Exit 5 |
| Typo'd relation, unrelate | `unrelate E1S1 dependson E1S2` | usage error naming the valid relations; the real edge survives | Exit 2 |
| Typo'd relation, relate | `relate E1S1 dependson E1S2` | the same usage error — not `invalid_relation_kind` | Exit 2 |
| Disallowed pair | `relate E1S1 traces_to E1S2` (story→story) | `invalid_relation_kind`, exit 1, unchanged from today | Exit 1 |
| Known relation, no pairs | `relate` with `verifies` | reported as a disallowed pair, never as an unknown relation | Exit 1 |
| Absent edge, valid name | `unrelate` an edge that is not there | exit 0, `changed: false` | N/A |
| Already-present edge | `relate` an edge that is there, no `--if-version` | exit 0, `changed: false` | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/write.rs` (`apply_relation_change`) -- the no-op return sits above the version check, so `if_version` is never compared on that path; the check must move ahead of the outcome. Note `old_version` is already parsed before it -- H8's core half
- `crates/qdev-core/src/dag.rs` (`allowed_kind_pairs`) -- eight match arms, one per relation, returning `&[]` for both `verifies` and any unknown name. The name list belongs here, with the match derived from it or asserted against it -- the one list
- `crates/qdev-cli/src/main.rs` (`handle_relate`) -- validates the kind pair via `is_valid_kind_pair` and reports `invalid_relation_kind` for an unknown name too, because it cannot distinguish the two empty slices -- M1's misdescription
- `crates/qdev-cli/src/main.rs` (`handle_unrelate`) -- no relation-name check, no kind check, no `if_version`: it passes `if_version: None` unconditionally -- M1's silence and H8's other half
- `crates/qdev-cli/src/cli.rs` (`UnrelateArgs`) -- has `--author-type`/`--author-id` but no `--if-version`, unlike `RelateArgs` -- the missing flag
- `crates/qdev-core/src/write.rs` (`RelationChangeOptions.if_version`) -- already carries the field; only `handle_unrelate` never sets it -- reuse
- `crates/qdev-core/src/store/sqlite.rs` (`validate_relations_graph`) -- reports `invalid_relation_kind` for an unknown relation name found in a *file*, which is a workspace defect rather than a usage error and stays as it is -- do not change
- `crates/qdev-cli/tests/relate_cli_tests.rs` -- 19 tests; the `--if-version` coverage exercises only the changed path, which is why H8 shipped -- the gap to close
- `docs/cli-reference.md` -- documents `--if-version` on `update` only, and presents `relate`/`unrelate` as a pair -- keep docs authoritative

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/dag.rs` -- one list of relation names beside `allowed_kind_pairs`, with a test asserting the two agree so a ninth relation cannot be added to one and not the other -- the one list
- [x] `crates/qdev-core/src/write.rs` -- compare `if_version` before returning any outcome, so the fence holds on the idempotent path -- H8
- [x] `crates/qdev-cli/src/cli.rs` + `main.rs` -- `unrelate` accepts `--if-version`; both handlers reject an unknown relation name with a usage error naming the valid ones, before any other check -- M1 and H8's other half
- [x] `crates/qdev-cli/tests/relate_cli_tests.rs` -- cover every matrix row, including the fence on both no-op paths and `verifies` still being a disallowed pair rather than an unknown relation -- verification
- [x] `docs/cli-reference.md` -- document `--if-version` on all three mutating commands and the relation-name refusal -- keep docs authoritative

**Acceptance Criteria:**
- Given an edge that already exists, when `qdev relate` runs with a stale `--if-version`, then it exits 5 `version_mismatch` and the file is unchanged.
- Given any edge state, when `qdev unrelate` runs with a stale `--if-version`, then it exits 5 `version_mismatch`.
- Given a misspelled relation name, when either `qdev relate` or `qdev unrelate` runs, then it exits 2 with a message naming the valid relations, and any real edge between those entities survives.
- Given `verifies`, when `qdev relate` runs, then it is refused as a disallowed kind pair (exit 1), never as an unknown relation.
- Given an absent edge and a valid relation name, when `qdev unrelate` runs, then it exits 0 reporting `changed: false`.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean, and every existing `relate`/`unrelate` test passes unchanged except any that asserted the defective behaviour, which is called out in the Spec Change Log.

## Implementation Notes

- **The one list is now one table.** `dag.rs` holds `RELATION_KIND_PAIRS`: architecture.md §8's
  eight rows, each name beside the `(source_kind, target_kind)` pairs it allows.
  `allowed_kind_pairs`, `relation_names` and the new `is_known_relation` are all derived from it,
  so a ninth relation cannot be added to one view and not the other — there is no second list to
  forget, which is stronger than a test asserting two lists agree. `allowed_kind_pairs` keeps its
  signature and its contents; `dag_tests` pins the table's names and order, and pins `verifies` as
  known-but-pairless against an unknown name that is neither.
- **The fence moved above every outcome.** `apply_relation_change` now reads the declared version
  through `frontmatter_version` and calls `check_if_version` immediately after parsing the
  frontmatter — before the merge, before the idempotent no-op return, before any write. Both
  helpers are new and shared: `patch_frontmatter`'s inline version-inspection block and its two
  `version_mismatch` constructions were replaced by calls to them, so there is exactly one reader
  of `version:` and one place that error is built. As a side effect the version
  `apply_relation_change` *reports* is now read the same way `patch_frontmatter` reads it (it used
  to go through `serde_yaml`, which answers `None` for a quoted `version: "3"` where the line
  scan answers `3`).
- **The name enum sits at the command surface.** `validate_relation_name` in `main.rs` is called
  first thing by both `handle_relate` and `handle_unrelate` — before the store is opened and
  before the ids are resolved, so a typo is reported as a typo rather than as a missing entity.
  It is a plain `usage_error` (exit 2), the same code an unrecognised `--author-type` gets, and
  the message names all eight relations. `apply_relation_change` still accepts any name, as the
  Never list requires for `--fix-ids`.
- **`unrelate` gained `--if-version`** in `UnrelateArgs` and passes it through instead of the
  hardcoded `None`.
- **Verification.** Each of the three fixes was reverted in turn to confirm the new tests fail
  without it: reverting the core fence fails the two no-op-path fence tests; reverting the CLI
  `if_version` plumbing fails the two `unrelate` fence tests; removing the two
  `validate_relation_name` calls fails the three unknown-name tests. The `verifies` test passes
  before and after by design — it is the regression guard that the new refusal does not swallow a
  known relation.
- **Prose kept in step.** `architecture.md` §8 now states the enum split (unknown name = exit 2 at
  the command surface; disallowed pair = `invalid_relation_kind`; the write path accepts any name
  for `--fix-ids`' sake) and §11's optimistic-concurrency item names all three commands and the
  before-the-outcome rule; `cli-reference.md` gains "Relations, and what `relate`/`unrelate`
  refuse" and "`--if-version` on the three mutating commands", and its command table shows the
  flag on both. `spec-1-10`'s Implementation Notes and its review-triage row 5 (which rejected the
  `unrelate --if-version` asymmetry as "by design") carry dated supersession notes, and the three
  deferred-work entries for H8 and M1 are struck through in place per that ledger's convention.
- **Two deferred entries whose review-by point was "with the H8/M1 contract work"** are
  deliberately not covered here — `--fix-ids` exiting 0 on a workspace plain `validate` fails, and
  `relate`'s precheck reading a stale cache row — both outside this spec's boundaries. Rather than
  let the named moment elapse silently a second time, each entry now records that this work landed
  without covering it, and re-points at epic 1 acceptance / epic 2 planning.

## Spec Change Log

- 2026-09-11 — Implemented. No frozen section changed and no existing test asserted the
  defective behaviour, so nothing was deleted: the pre-existing stale-`--if-version` relate test was *strengthened* (it asserted only `.failure()`; it now pins exit 5 and
  `version_mismatch`), and every other pre-existing `relate`/`unrelate` test passes unchanged.
  The Code Map's suggestion of a name list "with the match derived from it or asserted against
  it" was taken in its first form: the match arms were replaced by one table, so the two cannot
  drift by construction.
- 2026-09-11 — Created from epic 1 cross-story review findings H8 and M1. Both were triaged as blocking in pass 1 and then dropped: H8 was never scheduled or filed, and M1's ledger entry named the identity-resolution story, which shipped without touching it. Pass 2 re-verified both as reproducing. One story because they are the same seam — the relation write path and its two handlers — and no Open Questions, because the answers follow from contracts the rest of the binary already keeps.

## Review Triage Log

Pass 1 (2026-09-11) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | The new `frontmatter_version` replaced a `serde_yaml` read with a decimal-only scan, so `version: 0x03` — schema-valid, reported as `3` by `qdev get` — read as "no version": `--if-version 3` refused the version the entity declares, and an unfenced write **reset the counter to 1**, rolling it backwards and defeating every later fence (verification-gap and edge, reproduced against the binary). | high | patch | Re-verified by me before fixing. The YAML read is restored with the line scan as a fallback for frontmatter YAML cannot parse as a whole; a table test now covers `0x03`, `0o3`, `+3`, quoted and comment-trailing forms. Mutation-verified. |
| 2 | `apply_entity_update` never adopted the "single reader" the change advertises: it still read the version through `extract_frontmatter`, so one call could report `old_version: 3` for a write that computed `version: 1` (verification-gap, edge). | high | patch | Confirmed. Routed through `frontmatter_version`; the redundant second parse is gone with it. |
| 3 | The rationale for keeping the name check out of core — "`--fix-ids` rewrites relation names it finds in files" — is **false**, and had been copied into four documents: `relations` is `additionalProperties: false` in the entity schemas, so a file with an unknown relation name is a `schema_violation`, never hydrates, and never reaches `rewrite_relations_to` (verification-gap, verified against the binary). | medium | patch | Confirmed by reading the schemas. The placement is still right — the enum belongs where the user types it, as `--author-type`'s does — but the reason was fiction. Corrected in all four places, with the correction saying so rather than quietly substituting a better reason. |
| 4 | `unrelate` still skips `ensure_query_workspace`, and spec-1-10's triage row 3 recorded that as an open `patch` in the very handler this story edits (blind). | false | rejected | Checked and disproved: the guard was centralised into the boot-path `requires_workspace` check, which classifies `Unrelate` as requiring a workspace. Verified — `qdev unrelate` outside a workspace exits 2 with the same message `get`/`list` give and creates nothing. spec-1-10's row 3 is annotated as resolved-by-centralisation so it stops reading as open. |
| 5 | The positive no-op fence case was untested: `--if-version` *matching* on an already-present edge, which is what would catch an over-eager fence (blind). | medium | patch | Confirmed. Added, asserting exit 0, `changed: false`, and no version bump. |
| 6 | The three resolved ledger entries recorded resolution by striking through the `summary:` value and appending a paragraph to `evidence:`, whose earlier sentences still read "reproducing" — corrupting both values for anything that consumes the file (blind). | medium | patch | Fair, and my own convention from earlier in the session was the precedent. Keyed entries now carry a `resolved:` line and leave the original values intact; the ledger's header states both forms and says not to edit the values. |
| 7 | `check_if_version` fires twice on a relate write — once in `apply_relation_change`, once inside `patch_frontmatter` — so a reader cannot tell which is the contract (blind). | low | defer | Real redundancy, and deliberate: `patch_frontmatter`'s check is the *only* one on the `update` path, so it cannot be removed, and passing `None` from the relation path would leave the engine's guard untested. Worth one comment rather than a change; filed. |
| 8 | A malformed version (`version: abc`) is reported as a missing one, sending the operator after the wrong problem (blind). | low | defer | Real. Distinguishing needs a third state in the reader (`Option<Result<u64, _>>` or an `invalid_version` conflict). Filed. |
| 9 | A stale `--if-version` combined with a disallowed pair, dangling target or cycle exits 1 or 2 rather than 5, because the CLI prechecks run before core's fence (edge). | low | defer | Confirmed by reading the order. Both refusals are true of that invocation; which one a caller should see is a contract decision, not an oversight. Filed. |
| 10 | `verifies` appears in no entity schema's `relations` properties, so once a kind pair exists for it a `verifies` edge would fail schema validation rather than the kind-pair refusal the docs describe (verification-gap). | low | defer | Confirmed. It is an Epic 3 prerequisite (the `Gate` kind does not exist yet), not a defect in this story. Filed against that epic. |
| 11 | `relation` stays a `String` with no `value_parser`, so the eight names appear in neither `--help` possible-values nor shell completion — the two places a user would catch a typo before making it (blind). | low | patch | Fair. The arg help strings keep the list for readability, and `validate_relation_name` now says why clap does not own the check: the JSON envelope's error shape depends on it. Filed as an option rather than taken. |
| 12 | `GRAPH_EDGE_RELATIONS` is a second hand-written name list that the one table cannot derive (blind, edge). | low | patch | Correct that it cannot be derived — it is a rendering subset, not the whole set — so a test now asserts every member *is* a relation name, which is the part that can go stale. |
| 13 | The new `architecture.md` §8 paragraph is hard-wrapped where the section is not, and names `dag.rs` where the rule would do (blind). | low | patch | Both corrected. |
| 14 | This spec shipped with `status: 'in-progress'` while its tasks were all `[x]` and the ledger entries it resolves point at it (blind). | low | patch | Advanced at hand-off, as the workflow does. |

## Design Notes

- **Why the enum check sits at the command surface, not in core.** The same split already exists for `--author-type`: the enum is enforced where the user types it, and core stays a mechanical writer of what it was asked for.
  **The reason this spec originally gave was wrong**, and a reviewer disproved it against the binary: it claimed `--fix-ids` rewrites unrecognised relation names found in files, so core had to tolerate them. It cannot — `relations` is `additionalProperties: false` in the entity schemas, so a file carrying an unknown relation name is a `schema_violation`, is never hydrated, and never reaches `rewrite_relations_to`. The placement is still right; the justification was fiction, and it had been copied into four documents before anyone checked it.
- **Why an unknown name cannot be inferred from `allowed_kind_pairs`.** It returns `&[]` for `verifies` — a known relation with no allowed pairs yet — and for anything unrecognised, so the two are indistinguishable at that call. A separate list is not duplication; it is the missing distinction.
- The version check moving above the no-op return is the whole of H8's core fix, and it is the same shape as the defect the `--fix-ids` abort had: a `return` placed before the step that was supposed to guard it.
