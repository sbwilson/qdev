---
title: 'Story 4.2: Attributed Rejections'
type: 'feature'
created: '2026-09-29'
status: 'done'
route: 'dispatch'
baseline_commit: 151e64e3b5cc312be236a34b5eeac4b95080f12c
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** When a command fails or refuses an action (e.g. gate failures, scope violations, lease conflicts, missing justifications, cross-team edits, blocked dependencies), agents receive unhelpful or unstructured error messages and must guess what rule was violated, causing hallucinated fixes or blind halts.

**Approach:** Implement structured rejection attribution in `qdev-core` and `qdev-cli` ensuring every refusal path attaches an attribution payload in `details` containing at least one of `constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`, plus human-readable rule text (`rule`), backed by a conformance test verifying all error codes and documentation directing the develop skill to quote cited IDs.

## Boundaries & Constraints

**Always:**
- Keep all error emissions compliant with the AD-13 JSON error envelope: `{"schema_version": "1", "error": {"code": "...", "message": "...", "details": {...}}}` on stdout with non-zero exit codes.
- Ensure every refusal path (`gate fail`, `scope violation`, `lease conflict`, `missing justification`, `cross-team edit`, `blocked dependency`, and sprint close refusals) populates `details` with at least one attribution key (`constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`) AND the human-readable rule text (`rule`).
- Keep `qdev-core` headless: error attribution data structures, builders, and standard error catalog belong in `qdev-core`, while CLI handlers format or forward them.
- Provide a conformance test suite iterating every refusal error code in the binary and testing every refusal path against the attribution schema.
- Document in `docs/compliance-and-safety.md` and `docs/cli-reference.md` that the `develop` skill instructs agents to quote cited IDs when reporting failures to humans.

**Never:**
- Never break existing CLI text output formatting or change existing exit code semantics (1 for logical failure, 2 for usage error, 3 for policy refusal, 4 for infrastructure failure, 5 for conflict).
- Never allow a refusal path to emit `details: null` or empty `details` without attribution fields.
- Never hardcode transient or machine-specific paths as identifiers in attribution fields.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Scope violation with no-go | Bound gate `qdev-scope` fails touching path violating no-go `NG-2` | Exit 1; `details` has `gate_id: "qdev-scope"`, `constraint_id: "E12S4/NG-2"`, `rule: "..."` | `ExitCode::LogicalFailure` |
| Scope violation without no-go | Bound gate `qdev-scope` fails touching path outside `target_modules` | Exit 1; `details` has `gate_id: "qdev-scope"`, `policy: "target_modules"`, `rule: "..."` | `ExitCode::LogicalFailure` |
| Gate run logical fail | Bound external gate returns `fail` | Exit 1; `details` has `gate_id: "<id>"`, `rule: "..."` | `ExitCode::LogicalFailure` |
| Lease conflict on claim | `qdev claim story E12S4` when already leased by user `amelia` | Exit 5; `details` has `holder: "amelia"`, `policy: "single_lease_holder"`, `rule: "..."` | `ExitCode::Conflict` |
| Lease release by non-holder | `qdev lease release E12S4` by user `bob` without `--force` | Exit 3; `details` has `holder: "amelia"`, `policy: "lease_ownership"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Missing justification | `qdev transition story E12S4 in-progress` (backward) without `--justification` | Exit 3; `details` has `policy: "justification_required"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Missing justification on skip-gates | `qdev transition story E12S4 review --skip-gates` without `--justification` | Exit 3; `details` has `policy: "justification_required"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Non-interactive cross-team mutation | `qdev update story E12` owned by team `infra` by member of `frontend` non-interactively | Exit 3; `details` has `policy: "cross_team_governance"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Non-interactive out-of-lease mutation | `qdev update story E12S2` non-interactively when story E12S1 is leased | Exit 3; `details` has `policy: "lease_scope"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Blocked dependency | `qdev transition story E12S4 in-progress` where dependency `E12S1` is not `done` | Exit 3; `details` has `blocking_ids: ["E12S1"]`, `policy: "dependency_order"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Sprint close unacceptable DW | `qdev sprint close 5` with open DW having `risk: unacceptable` without rationale | Exit 3; `details` has `blocking_ids: ["DW-..."]`, `policy: "deferred_work_rationale"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Sprint close active lease | `qdev sprint close 5` with story in progress having an active lease | Exit 3; `details` has `holder: "<user>"`, `policy: "lease_lifecycle"`, `rule: "..."` | `ExitCode::PolicyRefusal` |
| Preflight push refusal | `qdev hook pre-push` when working tree is dirty outside scope | Exit 3; `details` has `policy: "preflight_guard"`, `rule: "..."` | `ExitCode::PolicyRefusal` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/errors.rs` -- Error taxonomy and `QdevError`: Add `RejectionAttribution` builder, `with_attribution(...)`, and canonical `REFUSAL_CATALOG` / error code registry for conformance assertion.
- `crates/qdev-core/src/gate/builtin/scope.rs` -- `execute_scope_gate`: Ensure failure payloads and outcome contain `gate_id`, `policy: "target_modules"`, `constraint_id` (when matching no-go), and human-readable `rule`.
- `crates/qdev-core/src/transition.rs` -- `TransitionGateHook` and `TransitionEngine`: Ensure gate failure, missing justification, blocked dependency, and hook guardrails attach attribution fields and `rule`.
- `crates/qdev-core/src/lease.rs` -- `claim_lease_with_storage` and `release_lease_with_storage`: Add `holder`, `policy`, and `rule` to error details.
- `crates/qdev-cli/src/main.rs` -- `handle_mutation_governance`: Add `policy`, `holder` (if leased), and `rule` to cross-team and out-of-lease non-interactive and override refusals.
- `crates/qdev-core/src/sprint.rs` -- `close_sprint`: Attach `blocking_ids`, `holder`, `policy`, and `rule` to sprint close refusals.
- `crates/qdev-core/src/hook.rs` -- `run_pre_push`: Attach `policy: "preflight_guard"` and `rule` on preflight refusal.
- `crates/qdev-core/tests/attribution_conformance_tests.rs` -- Conformance test suite iterating all refusal error codes in the binary and exercising each of the 6 refusal paths.
- `docs/compliance-and-safety.md` & `docs/cli-reference.md` -- Document attributed rejections format, attribution fields, and the `develop` skill requirement to quote cited IDs.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/errors.rs` -- Define `RejectionAttribution` struct/builder, helpers on `QdevError` (`policy_refusal_attributed`, `logical_failure_attributed`, etc.), and canonical catalog of all refusal error codes in binary -- foundational attribution contract
- [x] `crates/qdev-core/src/gate/builtin/scope.rs` -- Populate `gate_id: "qdev-scope"`, `policy: "target_modules"`, `constraint_id` (when matching no-go), and human-readable `rule` on scope gate failures -- scope rejection attribution
- [x] `crates/qdev-core/src/transition.rs` -- Populate attribution fields in `TransitionGateHook` (gate_id, constraint_id, rule), `check_dependencies` (blocking_ids, policy, rule), and state validation (policy: "justification_required", rule) -- transition rejection attribution
- [x] `crates/qdev-core/src/lease.rs` -- Populate `holder`, `policy`, and `rule` on lease conflicts and release rejections -- lease rejection attribution
- [x] `crates/qdev-cli/src/main.rs` -- Populate `policy`, `holder`, and `rule` in mutation governance (`needs_confirmation` and `needs_justification`) -- cross-team & out-of-lease mutation attribution
- [x] `crates/qdev-core/src/sprint.rs` -- Populate `blocking_ids`, `holder`, `policy`, and `rule` on sprint close refusals -- sprint close rejection attribution
- [x] `crates/qdev-core/src/hook.rs` -- Populate `policy: "preflight_guard"` and `rule` on hook preflight refusals -- git hook preflight attribution
- [x] `crates/qdev-core/tests/attribution_conformance_tests.rs` -- Unit & integration conformance tests iterating all refusal error codes and asserting presence of at least one of `constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`, plus `rule` across all 6 refusal paths -- conformance gate
- [x] `docs/compliance-and-safety.md` & `docs/cli-reference.md` -- Update documentation with schema details for attributed rejections and explicitly document the `develop` skill requirement -- documentation integrity

**Acceptance Criteria:**
- Given any refusal path (gate fail, scope violation, lease conflict, missing justification, cross-team edit, blocked dependency), when it occurs, then the error envelope's `details` includes at least one of `constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`, plus the human-readable text of the rule.
- Given the error code catalog in the binary, a conformance test iterates every refusal error code in the binary and asserts the attribution fields (`rule` and at least one of `constraint_id`, `gate_id`, `policy`, `blocking_ids`, holder) are present.
- Given the `develop` skill documentation and specification in `docs/compliance-and-safety.md`, it explicitly instructs agents to quote the cited ID when reporting a failure to a human.

## Implementation Notes

- **Attribution Model (`qdev-core::errors`):**
  Added `RejectionAttribution` with builder methods (`with_constraint_id`, `with_gate_id`, `with_policy`, `with_blocking_ids`, `with_holder`), `apply_to_value`, and `apply_to_error`.
  Added constructor helpers on `QdevError`: `logical_failure_attributed`, `policy_refusal_attributed`, `conflict_attributed`, and method `.with_attribution(...)`.
  Exported `REFUSAL_CATALOG` (`&[RefusalCatalogEntry]`) with 16 refusal error codes, standard rule text, and required attribution keys.
- **Gate Contract Integrity:**
  Added `rejection_attribution()` helper on `GateRunOutcome` without altering `GateRunPayload` (which preserves strict adherence to `payload-gate-run.json` and `payload-gate-set.json` `additionalProperties: false`).
  In `transition.rs`, when a transition gate fails, the attribution from `outcome.rejection_attribution()` is attached directly into `QdevError::with_attribution(...)`, placing attribution in the error envelope `details` (matching `payload-error.json`).
- **Refusal Paths Covered:**
  - Scope gate failures (`gate_id: "qdev-scope"`, `policy: "target_modules"`, `rule`, optional `constraint_id`).
  - Transition guardrails: `--skip-gates` refusal (`policy: "transition_guard"`), backwards transitions (`policy: "backward_transition"`), terminal transitions (`policy: "terminal_state"`), blocked dependencies (`policy: "dependency_precondition"`, `blocking_ids`).
  - Story leases: claim conflict on already leased (`policy: "story_lease"`, `holder`), release denied (`policy: "story_lease"`, `holder`).
  - Mutation governance: interactive confirmation / `--yes` requirement (`policy: "cross_team_governance"` or `"lease_scope"`, optional `holder`), override justification (`policy: "justification_required"`).
  - Sprint close: open deferred work (`policy: "unacceptable_deferred_work"`, `blocking_ids`), active story leases (`policy: "active_story_lease"`, `holder`), non-active sprint (`policy: "sprint_status"`).
  - Hook preflight: dirty worktree or non-scoped changes (`policy: "preflight_guard"`).
- **Documentation:**
  Updated `docs/compliance-and-safety.md` and `docs/cli-reference.md` detailing the attributed rejection format, field requirements, and mandatory instructions for develop skills / agents to quote the cited IDs when notifying humans.
- **Verification:**
  10/10 conformance tests in `crates/qdev-core/tests/attribution_conformance_tests.rs` pass.
  Full workspace test suite `cargo test --workspace` passes cleanly with zero regressions.

## Spec Change Log

## Review Triage Log

| # | Reviewer Layer | Location | Claim / Finding | Verdict | Route | Resolution |
|---|---|---|---|---|---|---|
| 1 | Blind Hunter | `crates/qdev-core/src/gate/mod.rs`, `transition.rs` | `GateRunOutcome::rejection_attribution()` defined but unused in transition gate runs | `high` | `patch` | Adopted `outcome.rejection_attribution()` across all gate hooks in `transition.rs` and `review.rs`. |
| 2 | Edge Case Hunter | `crates/qdev-core/src/review.rs` | Sprint review gate failure did not attach structured attribution to error envelope | `high` | `patch` | Attached `outcome.rejection_attribution()` to both `gate_failed` and `gate_infra_failure` in `review_sprint`. Added test `sprint_review_gate_failure_attaches_attribution`. |
| 3 | Verification Gap | `crates/qdev-core/src/chore.rs` | `lease_held` and `out_of_allowlist` refusals emitted without structured attribution | `high` | `patch` | Attached `RejectionAttribution` with `policy: "story_lease"` / `"allowlist"` and added to `REFUSAL_CATALOG`. |
| 4 | Verification Gap | `crates/qdev-core/src/sprint.rs` | `close_reason_required` refusal lacked attribution and catalog entry | `medium` | `patch` | Added `close_reason_required` to `REFUSAL_CATALOG` and attached `policy: "sprint_lifecycle"` attribution in `sprint.rs`. |
| 5 | Blind Hunter | `crates/qdev-core/tests/attribution_conformance_tests.rs` | Conformance suite did not test gate infrastructure failure path | `medium` | `patch` | Added `test_conformance_path_gate_infra_failure` covering `gate_infra_failure` exit 4 and attribution. |
| 6 | Edge Case Hunter | `crates/qdev-core/src/errors.rs` | Non-object existing `details` were overwritten by `with_attribution` | `medium` | `patch` | Updated `with_attribution` to preserve existing details (wrapping in `previous_details` if non-object). |
| 7 | Edge Case Hunter | `crates/qdev-core/src/hook.rs` | Preflight hook error details dropped diagnostics and remediation | `low` | `patch` | Attached hook diagnostics, summary, and remediation commands to `details` alongside attribution. |
| 8 | Edge Case Hunter | `crates/qdev-cli/src/main.rs`, `handlers/scratch.rs` | Interactive empty input prompt refusals had rule text but uncataloged policy | `low` | `patch` | Attached structured `policy: "justification_required"` to interactive empty justification prompts. |
| 9 | Verification Gap | `crates/qdev-core/src/errors.rs` | `RejectionAttribution` lacked helper methods `apply_to_value` and `apply_to_error` | `low` | `patch` | Implemented `apply_to_value` and `apply_to_error` on `RejectionAttribution`. |
| 10 | Blind Hunter | `crates/qdev-cli/tests/` | CLI tests lacked explicit assertions on attribution fields in `--json` | `low` | `patch` | Added assertions for `policy`, `rule`, and `holder` in `scope_cli_tests`, `scratch_cli_tests`, `init_cli_tests`, `constraint_tests`. |
| 11 | Blind Hunter | `crates/qdev-core/src/gate/mod.rs` | `GateRunPayload` was not modified to include attribution | `false` | `reject` | `GateRunPayload` schema specifies `additionalProperties: false`. Attribution belongs in `GateRunOutcome` and error envelopes, not `GateRunPayload`. |

## Design Notes

- **Attribution Fields Contract:**
  Every refusal error envelope emitted under `--json` has:
  ```json
  {
    "schema_version": "1",
    "error": {
      "code": "<error_code>",
      "message": "<human-readable summary>",
      "details": {
        "constraint_id": "<ID optional>",
        "gate_id": "<ID optional>",
        "policy": "<policy_name optional>",
        "blocking_ids": ["<ID optional>"],
        "holder": "<user/session optional>",
        "rule": "<human-readable rule text required>"
      }
    }
  }
  ```
- **Error Code Catalog:**
  `qdev-core::errors` exports `REFUSAL_CATALOG`, an array of all refusal error codes used across the binary (`gate_failed`, `gate_infra_failure`, `already_leased`, `lease_not_found`, `needs_justification`, `needs_confirmation`, `story_blocked`, `unacceptable_deferred_work`, `active_story_lease`, `sprint_already_closed`, `sprint_not_active`, `carry_over_not_allowed`, `tty_required`, `human_required`, `preflight_refusal`, `scope_violation`), each with its required attribution fields and standard rule text.
- **Backward Compatibility:**
  Existing fields in `details` (e.g. `story_id`, `target_status`, `from_status`, `worktree_path`, `failures`) remain intact for backward compatibility; the attribution fields are additive.

## Verification

**Commands:**
- `cargo build --workspace` -- expected: compiles cleanly with no warnings
- `cargo test -p qdev-core --test attribution_conformance_tests` -- expected: all conformance tests pass
- `cargo test --workspace` -- expected: entire workspace test suite passes with zero regressions
