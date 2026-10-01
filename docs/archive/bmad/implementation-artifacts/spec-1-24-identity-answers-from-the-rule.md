---
title: 'Which file holds entity X is answered by the identity rule, not by the filesystem'
type: 'bugfix'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 1
baseline_commit: '32c94bb391c01e299bdf8fcca14f355de7b8b67a'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** the resolver asks the filesystem a question the identity rule is supposed to answer.
`find_file_in_dir` short-circuits on `dir.join("<id>.md").is_file()` before it consults the rule,
and on a case-insensitive filesystem — macOS and Windows, the platforms this is developed on — the
OS answers for spellings the rule does not admit. Two consequences, both verified against the
binary:

- **A phantom duplicate.** With a single `docs/specs/stories/e1s1.md` on disk, the probe resolves
  it as `E1S1.md` and the directory scan finds it again as `e1s1.md`; the two paths differ as
  strings, so `qdev update E1S1` exits 2 with *"Multiple entity files match ID 'E1S1' … ["E1S1.md",
  "e1s1.md"]"* — naming a file that does not exist, for an entity `qdev get` returns at exit 0.
  An entity readable but unwritable is pass-1 H2's class, reached through a new door (P3-2).
- **`.MD` resolves for writers on one platform and not another.** `filename_carries_id` requires a
  literal `.md`, so the rule says an `E1S7.MD` name carries no id — and `qdev validate` says so in
  an `entity_file_off_convention` warning. But the probe resolves it anyway on macOS, so `update`
  and `relate` succeed and report the path `…/E1S7.md`, a spelling no file has, which is also what
  they write to the cache until the next sweep corrects it. On Linux the same command fails. The
  warning is factually wrong on the platform where the write just succeeded (P3-6).

**Approach:** delete the probe and let the rule answer — every candidate judged by
`filename_carries_id`, so two matches mean two files. Then make the rule's consequences honest: a
refusal that names the file actually holding the id and the rename that fixes it, and a warning
that is true on every platform. Finally, make the sites enumerable, so the next place that maps an
id to a path cannot quietly become a second answer.

## Boundaries & Constraints

**Always:**
- One answer to "which file holds entity X", and it is the identity rule: `<id>.md`,
  `<id>-<slug>.md`, `<id>_<slug>.md` in the entity's kind directory, **id and extension both
  matched case-insensitively**.
- **Decision (2026-09-11, Simon): option B.** The extension is matched case-insensitively for
  *resolution* as well as for occupancy, so `E1S7.MD` resolves for writers on every platform and
  the two halves of the identity rule agree completely on that axis. The deliberate asymmetry
  recorded in `spec-one-id-in-use-rule.md` — occupancy case-insensitive, resolution literal — is
  renegotiated: it existed so an unresolvable name could still own an id, and the extension was
  never the part of the rule that made a name unresolvable. `<id>.md` stays the canonical name
  every writer creates; `.MD` becomes a legal spelling rather than a repairable defect.
- The same answer on every filesystem. No resolution outcome may depend on whether the host is
  case-sensitive.
- "Multiple entity files match" means two distinct files, not two spellings of one.
- A refusal is actionable: when an id is held by a file the rule does not resolve, the message
  names that file and the rename that would fix it — `qdev validate` already computes both.
- `qdev validate`'s `entity_file_off_convention` warning says only things that are true of the
  write path on every platform.
- The rule's sites are **enumerable by a test**, not by memory: after this story, a new place that
  maps an id to a path or probes the filesystem for one fails a check rather than silently
  becoming a second answer.

**Never:**
- No change to what the *occupancy* half answers (`id_carried_by_filename`, `ids_in_use`): the
  set of ids a workspace owns is unchanged by this story. Option B makes resolution agree with
  occupancy on the extension; it must not widen or narrow occupancy itself.
- No change to which directory an entity's file lives in, and no new rename or repair behaviour.
  `--fix-ids` keeps what it does today; its own defects belong to story 1-27.
- No fix for the unreadable-directory hole in the same function (`read_dir` failing yields "no
  matches"). That is story 1-28's rule, and doing it here would blur two invariants.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Lone lowercase name | only `e1s1.md`, declaring `E1S1` | `update`/`relate`/`unrelate` resolve it; one match | N/A |
| Genuinely two files | `E1S1.md` and `E1S1-copy.md`, both declaring `E1S1` | usage error naming both, exit 2 | Exit 2 |
| Canonical name | `E1S1.md` | resolves, as today | N/A |
| Slugged names | `E1S1-buffer.md`, `E1S1_buffer.md` | resolve, as today | N/A |
| Uppercase extension | `E1S7.MD` declaring `E1S7` | resolves for writers on every platform; no off-convention warning; the reported and cached path is the real one (`.MD`) | N/A |
| Both spellings, case-sensitive host | `E1S1.md` and `E1S1.MD` both declaring `E1S1` | two distinct files → usage error naming both | Exit 2 |
| Id held by an unresolvable name | `notes.md` declaring `E1S9` | refusal names `docs/specs/stories/notes.md` and the rename to `E1S9.md`, exit 2 | Exit 2 |
| Off-convention warning | any file the write path will not resolve | warning is true on macOS and on Linux | N/A |
| Nothing on disk | no file declares or carries `E1S1` | not found, as today | Exit 2 |
| Second answer added | new code maps an id to a path outside the resolver | a test fails | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/write.rs:1294` (`find_file_in_dir`) -- the probe at `:1298`
  (`dir.join(canonical_file_name(id)).is_file()`) is the defect; the `read_dir` loop at `:1304`
  already applies the rule correctly. Deleting the probe is most of the fix -- the change
- `crates/qdev-core/src/write.rs:1182` (`filename_carries_id`) -- the rule itself, and its
  `ends_with(".md")` is what the Open Question is about -- the rule
- `crates/qdev-core/src/write.rs:1290` (`find_file_in_dir_for_id`) and `:1357`
  (`resolve_entity_file`) -- the two public doors onto the resolver; `resolve_entity_file` is where
  "Entity file not found" is raised -- the message to improve
- `crates/qdev-core/src/write.rs:1381`, `:1434` -- the two `Entity file not found for '<id>'`
  sites (`apply_entity_update`, `apply_relation_change`) -- what the actionable refusal replaces
- `crates/qdev-core/src/validate.rs:398-443` (`find_off_convention_entity_files`) -- the warning's
  claim and its `canonical_file_name` rename advice; the finding already computes both halves the
  refusal needs -- reuse, do not re-derive
- `crates/qdev-core/src/write.rs:1995` (`create_story`) -- builds `format!("{}.md", story_id)`
  inline rather than through `canonical_file_name`: a second spelling of the same rule, and the
  first thing an enumeration test should catch -- unify
- `crates/qdev-core/tests/architecture_tests.rs` -- the existing source-scanning tests (the AD-1
  dependency ban) are the precedent for an enumeration test that reads the source -- the mechanism
- `crates/qdev-core/tests/write_tests.rs`, `crates/qdev-cli/tests/update_cli_tests.rs` -- where the
  resolver's behaviour is pinned today -- verification

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/write.rs` -- delete the `.is_file()` short-circuit so every candidate is
  judged by `filename_carries_id`; state in the rustdoc that the filesystem is never asked to
  resolve a spelling -- the rule answers, not the OS
- [x] `crates/qdev-core/src/write.rs` -- match the extension case-insensitively in
  `filename_carries_id` **only**, so every caller inherits it -- one rule, one site
- [x] `crates/qdev-core/src/write.rs` -- when no file resolves but the workspace holds the id under
  a name the rule rejects, refuse with a message naming that file and the canonical rename -- an
  actionable refusal
- [x] `crates/qdev-core/src/validate.rs` -- a `.MD` name now carries its id, so the warning stops
  firing for it; the comment at `:409-413` argues the opposite and must go with the mechanism --
  no claim that is false where it is read
- [x] `crates/qdev-core/src/write.rs:1219` and `docs/architecture.md:266-279` -- the prose that
  states resolution requires a literal `.md`, and the identity-rule section that does not mention
  the extension at all -- keep the prose in step with the mechanism, in the same commit
- [x] `crates/qdev-core/src/write.rs` -- `create_story` builds its path through
  `canonical_file_name` -- one spelling of the rule
- [x] `crates/qdev-core/tests/architecture_tests.rs` -- a test that fails when code outside the
  resolver maps an id to a path or probes the filesystem for one, naming the offending site --
  the enumeration mechanism this story owes
- [x] `crates/qdev-core/tests/write_tests.rs` + `crates/qdev-cli/tests/` -- cover every matrix row,
  each mutation-verified against the change it pins -- verification

**Acceptance Criteria:**
- Given a directory holding exactly one file that carries an id, when any write path resolves that
  id, then it resolves that file — whatever the case of the name and whatever the host filesystem.
- Given two files that both carry one id, when a write path resolves it, then the refusal names two
  distinct files that both exist.
- Given an id held only by a file the rule does not resolve, when a write is attempted, then the
  refusal names that file and the rename that would fix it, and `qdev validate` says the same thing.
- Given a new site that maps an id to a path, when the suite runs, then a named test fails.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo fmt --check` are clean.

## Implementation Notes

Implemented 2026-09-11 against baseline `32c94bb`.

**The rule answers.** `find_file_in_dir`'s `dir.join(canonical_file_name(id)).is_file()`
short-circuit is deleted: every entry the directory listing yields is judged by
`filename_carries_id`, and nothing else is. Two matches are therefore two directory entries, so
every name the "Multiple entity files match" refusal prints is a name `ls` prints. Matches are
sorted, because `read_dir` order is the filesystem's and a refusal has to read the same twice.
The de-duplication the probe needed (`!matches.contains(&path)`) went with it.

**Option B, in one site.** `filename_carries_id` splits the name on its last `.` and compares the
extension with `eq_ignore_ascii_case`, so `E1S7.MD`, `e1s1.md` and `E1S1-buffer.Md` all carry
`E1S1` for every caller — the write path, `validate`'s off-convention check, and `--fix-ids`'s
rename pre-flight. `id_carried_by_filename` (occupancy) was already case-insensitive on the
extension and is unchanged: nothing about which ids a workspace owns moved.

**The actionable refusal.** `resolve_entity_file`'s three not-found exits go through one
`entity_file_not_found`, which asks `validate::off_convention_file_holding_id` whether a file in
the spec or state tree declares or carries the id, and appends the sentence
`entity_file_off_convention` would print for that same file — the two are one function,
`validate::off_convention_message`, so they cannot describe one file two ways. When nothing holds
the id the bare "Entity file not found for 'X'" stands. The walk runs only on a path already
returning an error, so resolution keeps its no-cache, no-walk shape.

**`create_story`** spells its file name with `canonical_file_name` rather than an inline
`format!("{}.md", …)`, which was the second spelling of the rule the enumeration test now bans.

**The enumeration.** `architecture_tests::test_the_identity_rule_has_exactly_one_set_of_sites`
scans `qdev-core/src` and `qdev-cli/src` (source halves only, `#[cfg(test)]` excluded) for lines
that build a markdown name from a value or probe the filesystem at a path built from one, and
fails naming file, line and text unless the site is in `IDENTITY_RULE_SITES` — today one entry,
`canonical_file_name` itself. Its two predicates are pinned by their own test, so neutering them
fails the suite rather than passing quietly.

**Tests changed because they asserted the old rule.** Two, both about `.MD`:
`test_off_convention_check_judges_uppercase_md_extension` (core) is now
`…_by_the_same_rule` — a `.MD` name that carries its id draws no finding, a `.MD` name that
carries none (`notes.MD`) still does — and the CLI's
`test_off_convention_warning_covers_uppercase_md_extension` moved its fixture to `notes.MD`,
with the `E1S9.MD` case becoming the new
`test_uppercase_md_extension_carries_its_id_and_draws_no_warning`.

### Deliberately not done

- The unreadable-directory hole in `find_file_in_dir` (`read_dir` failing still yields "no
  matches") is story 1-28's, per the frozen Never list.
- `--fix-ids`'s own defects (P3-4) stay with story 1-27, which builds on this resolver.

## Spec Change Log

- 2026-09-11 — Implemented. Two existing tests asserted the pre-option-B `.MD` behaviour (the
  off-convention warning firing for a `.MD` name that carries its id) and were rewritten; both are
  named in the Implementation Notes. New coverage: four unit tests in
  `crates/qdev-core/tests/write_tests.rs`, five CLI tests in
  `crates/qdev-cli/tests/update_cli_tests.rs`, one in `crates/qdev-cli/tests/validate_cli_tests.rs`
  and one predicate test in `crates/qdev-core/tests/architecture_tests.rs`. Each was
  mutation-verified against the change it pins: restoring the probe, making the extension literal,
  blinding the refusal's lookup and adding a second name-building site each fail a named test.
- 2026-09-11 — Created from the pass-3 acceptance gate (P3-2, P3-6), first of the seven blocking
  stories. Folds in the filed "Entity file not found for a file that exists" message defect, which
  the gate assigned to this story.

## Review Triage Log

### 2026-09-11 — Review pass (blind-hunter 12, edge-case-hunter 15, verification-gap 3 + 2)

Three layers, 32 findings, grouped to 14 entries. No `intent_gap` and no `bad_spec`, so no
loopback. Twelve patched, one refuted, two deferred. Mutations cited as evidence I re-ran myself.

**Patched** (the implementer's own account of each change is above this line in Implementation
Notes; what follows is the verdict and why it was taken):

- `[high]` **The enumeration mechanism could not see the defect it was written to ban.** All three
  layers found this independently, and the verification-gap layer demonstrated it: the deleted
  probe is two statements, and both per-line predicates missed it, so reintroducing the exact
  deleted code left the test green. A guard the architecture document now cites as the reason the
  rule's sites stay enumerable, which passes on the one shape that actually occurred, is worse than
  no guard — it converts "nobody checked" into "a test says it is fine". The scan is stateful now;
  I re-ran the mutation myself and it fails naming `write.rs:1333`.
- `[med]` **Four real id→path sites escaped the predicates** — `--fix-ids`' rename plan and target
  probe, `create_story`'s path build and clobber check. Now detected and listed with reasons. The
  allow-list *is* the enumeration, so an inert entry is a lie about coverage; the implementer
  dropped one it could not make the scan flag, which is the right instinct.
- `[med]` **`split("#[cfg(test)]").next()` blinded the scan** to everything after the first test
  attribute in a file — no live miss today, and a permanent silent one the first time a
  `#[cfg(test)]` helper appears mid-file.
- `[med]` **`off_convention_message` had no `(true, true)` arm**, so a conventionally named file in
  another kind's standard directory drew a reason that is false on both counts and a "rename" that
  is a move. Unreachable from today's CLI (every caller passes `entity_kind: None`) and reachable
  from the public library API — graded medium because the message is the product here.
- `[med]` **The refusal's lookup was case-sensitive**, so `qdev update e1s9` fell back to the bare
  not-found while `E1S9` got the explanation. The story exists to stop spelling deciding answers;
  reintroducing that one layer up would have been the joke version of this fix.
- `[med]` **Only the first of N holders was named**, so the user renames it and the next write
  fails on the unnamed one. Everywhere else this change insists a refusal names every file.
- `[low]` **Two rustdoc claims went stale**: `find_file_in_dir`'s swallowed `read_dir` failure is
  now strictly worse than before the probe was deleted, and `resolve_entity_file` still advertised
  "no walk" above an error path that walks both trees. Both now say what is true, and the first
  names story 1-28 as the owner.
- `[low]` **Test gaps**: no negative coverage for the widened extension, no test for
  `renamed_file_name`'s documented `.MD` behaviour, two uncovered branches of the holder lookup,
  and a host-dependent test that skipped silently on the platform the work was done on.

**Refuted:**

- `[false]` *"`create story` on a case-sensitive host holding `E1S1.MD` writes a second file
  carrying the id, and every later write then fails as ambiguous."* The premise fails at the first
  step: allocation asks `ids_in_use`, which includes the id carried by `E1S1.MD`, so the CLI never
  offers that id. Verified — with `E1S1.MD` present, `qdev create story E1` allocated `E1S2`. A
  library caller that passes an explicit id could still do it, which is `create_story`'s
  documented contract, not this story's defect.

**Deferred** (filed in `deferred-work.md`):

- The not-found refusal walks both trees and reads every markdown file whole, swallowing IO
  errors, on every failed resolution.
- `matches.sort()`'s stable ordering is asserted only by a comment; both tests are
  order-insensitive. The verification-gap layer disposed it `defer` itself — the names are all
  present either way.

**Noted, not filed:**

- On a case-sensitive host, `E1S1.md` and `E1S1.MD` side by side now refuse every write as
  ambiguous where `.md` alone used to resolve. That is the identity rule working — two files carry
  one id — not a regression to fix.
- `write` now calls into `validate` for the refusal's wording, which is a module cycle. Keeping one
  wording for the warning and the refusal is worth it; where that shared function lives is a
  question for whoever next opens either module.

## Design Notes

- The fix is a deletion, which is the shape to prefer here: the `read_dir` loop already implements
  the rule, and the probe exists only as an optimisation for the common case. One directory listing
  per resolution is the same cost hydration already pays per sweep.
- **Why the enumeration test reads the source.** The invariant is "no second answer", which no type
  signature expresses — a `PathBuf` built anywhere looks like any other. The repo already enforces
  AD-1's dependency ban by scanning source text, so the precedent exists. The test should name the
  offending file and line, and should fail loudly enough that adding a legitimate site requires
  updating the rule rather than silencing the test.
- **`renamed_file_name` preserves the extension it is given** (`E1S7.MD` renumbering to
  `E1S8.MD`), because it preserves the whole suffix after the id — the same way it preserves a
  slug. Under option B that is now a legal name, so this is consistent rather than a gap; say so
  at the site rather than leaving a reader to wonder.
- The refusal's new message needs the file holding the id, which the cache row already carries
  (`source_path`) and which `find_off_convention_entity_files` computes from the same data. Reuse
  that rather than a second scan.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: clean
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: no output
- `cargo fmt --check` -- expected: no output

**Results (2026-09-11):**
- `cargo test --workspace` — 30 suites, 0 failures.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.
- Manual, release binary, three scratch workspaces: a lone `docs/specs/stories/e1s1.md` gives
  `qdev get E1S1` exit 0 and `qdev update E1S1 --status ready` exit 0, reporting
  `docs/specs/stories/e1s1.md` — no phantom duplicate; `E1S1.md` + `E1S1-copy.md` refuse at exit 2
  naming both files, both of which exist; `notes.md` declaring `E1S9` refuses at exit 2 with
  *"'docs/specs/stories/notes.md' holds entity 'E1S9' but its name does not carry that id … rename
  it to 'docs/specs/stories/E1S9.md'"*, the same sentence `qdev validate`'s warning prints.

**Manual checks:**
- In a scratch workspace with only `docs/specs/stories/e1s1.md` declaring `E1S1`:
  `qdev get E1S1` exits 0 and `qdev update E1S1 --status ready` exits 0 — no phantom duplicate.
- With `E1S1.md` and `E1S1-copy.md` both declaring `E1S1`: the refusal names two files that exist.
- With a story declared in `notes.md`: the refusal names `notes.md` and the rename target.
