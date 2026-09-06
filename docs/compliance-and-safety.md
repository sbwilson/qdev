# qdev Quality, Verification Gates & Compliance Support

> Code hygiene, the gate engine and result contract, ratchets, evidence bundles, Git preflight, and how qdev's outputs support IEC 62304 and ISO 14971 processes.

## Table of Contents

- [Compact Citations & Code Hygiene](#code-hygiene)
- [The Gate Engine](#gate-engine)
- [Ratchets & Baselines](#ratchets)
- [Evidence Bundles](#evidence)
- [Compliance Support Mapping](#iec-compliance)
- [Git Preflight & Hooks](#git-preflight)

---

<a id="code-hygiene"></a>

## 1. Compact Citations & Code Hygiene

LLMs frequently inject forensic development memoirs into source comments:

```rust
// ❌ ANTI-PATTERN
// ⭐ **STORY 2.10 — THE FIRST `import RustBridge` IN THE RUNNING APP.**
// `DiscoveryPaneView` (the TCA pane) is replaced here by the Rust-driven
// one. The reducer itself is untouched ... `AW-NFR-16` requires the
// evidence at a commit PRECEDING the deletion, which is 2.11's.
```

### Compact citations

| Entity | Citation | Full context lives in |
| --- | --- | --- |
| Story | `// [E12S10] Rust-driven mount replaces TCA pane (see AD-43)` | `docs/specs/stories/E12S10.md` |
| Decision | `// [DEC-2b91] Extended attributes table shown here per ruling` | `docs/state/decisions/` |
| ADR | `// [AD-43] Bounded join; no async sleep` | `docs/specs/adrs/` |
| Hazard | `// [HAZ-14] Safety PIP engagement latch` | `docs/specs/hazards/` |
| Deferred work | `// [DW-7f3a] Temporary zero-copy bypass` | `docs/state/dw/` |
| Constraint | `// [E12S4/NG-2] Frame buffers intentionally untouched` | Story frontmatter |

The comment prefix follows the language (`//`, `///`, `#`, `/* */`, `"""`). The bracket token is what the linter and impact analysis match.

### The hygiene linter (`qdev hygiene check`)

v1 is **lint and report only**. The linter:

1. Extracts comments per configured language using a language-aware tokenizer (not raw regex over source).
2. Flags comment blocks exceeding `max_inline_comment_lines`, matches against `forbid_patterns`, and narrative markers (review rounds, wave numbers, story banners).
3. Reports `file:line`, the rule ID, and the offending excerpt in JSON and text. Exit 1 on findings.
4. Runs on the diff by default (`--diff`) and on paths on request.

The agent performs the rewrite and moves reasoning into the scratchpad with `qdev scratch append`. Automated `--fix` with diff preview is deferred to v2 because distinguishing a memoir from legitimate design documentation is a judgment call.

The `develop` projection includes this directive verbatim:

> Write standard code comments. Cite entities with compact bracket tags such as `[E12S4]`, `[AD-43]`, `[DEC-2b91]`. Never write narrative history, story summaries, or review commentary in code; put reasoning in the scratchpad with `qdev scratch append`.

---

<a id="gate-engine"></a>

## 2. The Gate Engine

Per **AD-5**, gates are external executables. The engine provides execution, timeouts, taxonomy, and summarisation.

### Execution

- Inherits the parent environment, then applies `[environment]` from config.
- Sets `QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, `QDEV_MODULE_PATHS` (JSON).
- Enforces `timeout_ms` per gate with platform-native termination (process group on Unix, job object on Windows).
- Captures stdout and stderr into bounded ring buffers (default 1 MB each).
- Runs `depends_on` gates first; `gate run --all` topologically orders them.

### Result contract

A gate may write JSON to stdout or to `$QDEV_RESULT_FILE`:

```json
{
  "status": "fail",
  "summary": "1 of 18 tests failed",
  "failures": [
    {"location": "crates/bridge/tests/c_abi_round_trip.rs:142",
     "message": "assertion failed: left == right\n  left: EffectsUnavailable\n right: EventNotUnderstood"}
  ],
  "metric": null,
  "constraint_ids": []
}
```

If no result document is present, qdev uses the exit code and the last 40 lines of stderr. If `output_adapter` names a shipped adapter (`cargo`, `xcodebuild`, `pytest`, `generic-tap`), the adapter extracts failures from raw output. Adapters are optional conveniences, never required.

### Failure taxonomy

| Status | Meaning | Agent instruction in payload |
| --- | --- | --- |
| `pass` | Exit 0 or result `pass` | Continue |
| `fail` | Non-zero exit, result `fail`, or ratchet regression | Fix the cited failures |
| `infra` | Timeout, missing executable, signal, non-JSON result when JSON was declared | **Stop and alert the human.** Do not retry or modify code |

### Receipts

```
[PASS] c-abi-round-trip | 18 tests | 8f1b2c4 | 1.2 s | evidence docs/state/evidence/E12S4/8f1b2c4-c-abi-round-trip.json
[FAIL] c-abi-round-trip (exit 101) | crates/bridge/tests/c_abi_round_trip.rs:142 | assertion failed: left == right
[INFRA] c-abi-round-trip | timeout after 300 s | last stderr: "waiting for device..." | halt and alert
```

### Attributed rejections

When a gate or review fails because of a constraint, the payload carries `constraint_id` and the constraint text, so the agent never guesses:

```json
{"status": "fail", "constraint_ids": ["E12S4/NG-2"],
 "failures": [{"location": "crates/video/frame.rs", "message": "Path outside target_modules; violates E12S4/NG-2: Do not touch frame buffers"}]}
```

Boundary and constraint checks are built-in gates (`qdev-scope`, `qdev-hygiene`, `qdev-deps`) that exist because they need qdev's own data, not project knowledge.

---

<a id="ratchets"></a>

## 3. Ratchets & Baselines

A ratchet is a gate with `kind = "ratchet"` that reports a numeric `metric`. The engine compares it against the baseline for the current integration branch stored under `docs/state/baselines/<branch>/<gate>.json`.

- `direction = must_not_increase | must_not_decrease`.
- A regression is a `fail` with the delta in the summary.
- `qdev gate baseline <id> --set` records the current value with author and commit; baselines are committed and reviewable.
- Sprint close records the baseline values in the release snapshot.

---

<a id="evidence"></a>

## 4. Evidence Bundles

Every gate run writes `docs/state/evidence/<story>/<sha>-<gate>.json`:

```json
{
  "schema_version": "1",
  "gate": "c-abi-round-trip", "story": "E12S4", "commit": "8f1b2c4",
  "status": "pass", "exit_code": 0, "duration_ms": 1200, "metric": null,
  "summary": "18 tests passed",
  "output_sha256": "…", "run_by": {"type": "agent", "id": "claude-code"},
  "ran_at": "2026-09-06T09:52:11Z", "verifies": ["FR-102"], "skipped_locally": false
}
```

Evidence is committed. `qdev review sprint` assembles the traceability matrix from `verifies` and `traces_to` plus these records. Signing evidence is deferred to v2.

---

<a id="iec-compliance"></a>

## 5. Compliance Support Mapping

qdev is not SaMD and is not a substitute for a quality management system. Its outputs are **designed to support** the following activities. Whether they satisfy an auditor depends on the project's own process.

| Activity | Clause | qdev output |
| --- | --- | --- |
| SOUP inventory and vulnerability review | IEC 62304 §5.3.3, §5.3.4, §8.1.2 | `qdev soup audit` records per release; SBOM via configured command |
| Known residual anomalies | §5.8.7, §9 | Deferred work with risk level and rationale; anomaly report at sprint close |
| Architectural segregation | §5.3.5 | Module registry layers plus the built-in `qdev-deps` gate |
| Verification records | §5.5.5, §5.7.5 | Evidence bundles per gate run per commit |
| Requirement traceability | §5.1.1, §7.3.3 | `traces_to`, `verifies`, `mitigates` relations; matrix export |
| Change impact | §6.2.3 | `qdev impact` over the relation graph and module paths |
| Risk control traceability | ISO 14971 §7 | Hazard entities linked to stories and gates |

Reviewer/developer phase separation (`develop` versus `review` skills, different models) is a useful practice but is **not** claimed as verification independence.

**Tool validation.** A tool that generates design-history evidence may itself require validation under the project's QMS. A self-test suite and validation report package is on the v2 roadmap.

---

<a id="git-preflight"></a>

## 6. Git Preflight & Hooks

### `qdev preflight`

Runs before `/qdev-develop` and `/qdev-create-story` and on `pre-push`:

1. **Scope check.** Uncommitted changes must fall within the leased story's `target_modules` paths (or declared chore paths). Otherwise: refuse, exit 3, list offending paths.
2. **Integration freshness.** In `story-branch` mode the guard checks that (a) the local integration branch is not behind its remote, and (b) the story branch's merge-base with the integration branch is within `max_integration_staleness_commits`. A story branch being "behind" the integration branch is expected and is not an error. In `trunk` mode it checks the current branch against its remote.
3. **Cache health.** Zero blocking validation findings.

```
✖ preflight: feature/E12S4-buffer is 27 commits behind develop at merge-base (limit 20).
  Rebase onto develop before continuing E12S4.
```

### Hooks

`qdev install hooks` writes two-line shims:

```sh
#!/bin/sh
exec qdev hook pre-commit "$@"
```

| Hook | Runs |
| --- | --- |
| `pre-commit` | `qdev hygiene check --diff`, `qdev validate --changed`, optional secret/PHI pattern gate |
| `pre-push` | `qdev preflight` |
| `prepare-commit-msg` | Only when `[commit_messages] enabled = true`: drafts a message from the leased story and recent scratchpad entries |

Shims are identical on all platforms; logic lives in the binary.
