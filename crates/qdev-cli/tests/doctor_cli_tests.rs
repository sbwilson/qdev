//! `qdev doctor` CLI tests: cache section fields present, `--json` shape, and the
//! uninitialized-workspace usage error (spec-1-12); plus the `validation` section reporting the
//! same checks `qdev validate` runs (spec-doctor-sees-computed-findings).

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "TestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();
}

fn write_story(dir: &Path, id: &str, title: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_doctor_json_shape_and_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["schema_version"], "1");

    let sections = val["sections"].as_array().unwrap();
    let cache = sections
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a 'cache' doctor section must be present");

    assert!(cache["cache_schema_version"].is_number());
    assert!(cache["entity_count"].is_number());
    assert!(cache["finding_count"].is_number());
    assert!(cache.get("last_synced_at").is_some());
}

#[test]
fn test_doctor_healthy_cache_reports_entity_and_finding_counts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_story(&stories_dir, "E1S2", "Two");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let cache = sections.iter().find(|s| s["name"] == "cache").unwrap();

    // Boot-time hydration (spec-1-6/1-7) has already parsed both new story files by the time
    // `doctor` reads the cache.
    assert_eq!(cache["entity_count"], 2);
    assert_eq!(cache["finding_count"], 0);
    assert!(
        cache["last_synced_at"].is_string(),
        "a synced cache must report a non-null last_synced_at: {}",
        cache
    );
}

#[test]
fn test_doctor_text_output_contains_cache_section() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["doctor"]).assert().success();

    let output = assert.get_output();
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert!(text.contains("[cache]"), "text output: {}", text);
    assert!(text.contains("schema_version"), "text output: {}", text);
    assert!(text.contains("entity_count"), "text output: {}", text);
    assert!(text.contains("last_synced_at"), "text output: {}", text);
    assert!(text.contains("finding_count"), "text output: {}", text);
}

#[test]
fn test_doctor_uninitialized_workspace_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_doctor_uninitialized_workspace_text_mode_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("usage_error"),
        "text-mode error output should be usage-error-shaped: {}",
        stderr
    );
}

#[test]
fn test_doctor_reports_nonzero_finding_count_for_a_seeded_validation_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // A merge-conflict marker is never parsed and records a `merge_conflict` finding for the
    // file's path (see `hydrate_markdown_file` in qdev-core's sqlite store).
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "One"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

<<<<<<< HEAD
## Acceptance Criteria
- AC.
=======
## Acceptance Criteria
- Other AC.
>>>>>>> branch
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let cache = sections.iter().find(|s| s["name"] == "cache").unwrap();

    assert!(
        cache["finding_count"].as_u64().unwrap_or(0) > 0,
        "expected a non-zero finding_count for a seeded merge-conflict finding: {}",
        cache
    );
}

/// The cache section reports the version the database actually carries alongside the one this
/// binary expects. Echoing the compiled-in constant back — as it did — made the one diagnostic
/// `qdev doctor` ships structurally incapable of detecting the stale or half-migrated cache it
/// exists to find.
#[test]
fn test_doctor_reports_observed_schema_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(&root.join("docs/specs/stories"), "E1S1", "Story one");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let cache = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a cache section");

    assert_eq!(cache["schema_status"], "ok");
    assert_eq!(cache["missing_tables"].as_array().unwrap().len(), 0);
    assert_eq!(
        cache["cache_schema_version"], cache["expected_cache_schema_version"],
        "a healthy cache reports the version it was built for"
    );
    assert!(
        cache["schema_version"].is_null(),
        "the section must not shadow the envelope's string schema_version"
    );
}

/// A cache stamped with an older version is healed by the boot-time `ensure_cache` rebuild
/// before any command's handler runs, so `qdev doctor` reports it as `ok` — the rebuild is what
/// makes that true, and `schema_status` is what would catch a rebuild that failed to stamp.
/// (The store-level observation itself is covered by `sweep_tests`.)
#[test]
fn test_doctor_reports_ok_after_a_stale_cache_is_healed_at_boot() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(&root.join("docs/specs/stories"), "E1S1", "Story one");

    let mut prime = Command::cargo_bin("qdev").unwrap();
    prime
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    {
        let conn =
            qdev_core::rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let cache = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a cache section");

    assert_eq!(
        cache["schema_status"], "ok",
        "the boot rebuild must have restamped the cache before doctor read it"
    );
    assert_eq!(
        cache["cache_schema_version"],
        cache["expected_cache_schema_version"]
    );
    assert_eq!(
        cache["entity_count"], 1,
        "the rebuild must not have lost the workspace's entities"
    );
}

// ---------------------------------------------------------------------------
// The `validation` section (spec-doctor-sees-computed-findings)
//
// The `cache` section's `finding_count` reads the `findings` table, which structurally holds
// only the findings hydration wrote. Four of `qdev validate`'s eight checks are computed fresh
// and never persisted, so `doctor` reported `finding_count: 0` on a workspace `validate` found
// defects in. The `validation` section runs `run_validation` — the same checks `validate` runs.
// ---------------------------------------------------------------------------

/// Writes a second file declaring an id another file already declares — the retrospective's
/// exact reproduction (finding C3): two `duplicate_planning_id` findings from `validate`, zero
/// from `doctor`.
fn write_duplicate_of(dir: &Path, id: &str, dup_name: &str) {
    fs::write(
        dir.join(format!("{}.md", dup_name)),
        fs::read_to_string(dir.join(format!("{}.md", id))).unwrap(),
    )
    .unwrap();
}

fn write_orphan_dw(root: &Path, id: &str) {
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Deferred thing"
status: open
target_module: core
origin_story_id: E9S9
safety_risk: negligible
rationale: "Not urgent."
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Notes
- Origin story E9S9 does not exist.
"#
        ),
    )
    .unwrap();
}

/// Seeds a `schema_violation`: a story frontmatter missing the required `status` field is never
/// hydrated, and hydration records the finding against its path.
fn write_schema_violation(dir: &Path, id: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Missing status"
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

fn doctor_sections(root: &Path) -> Vec<Value> {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        // Reporting findings never changes doctor's exit code: it is a report, not a gate.
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    val["sections"].as_array().unwrap().clone()
}

fn section<'a>(sections: &'a [Value], name: &str) -> &'a Value {
    sections
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("a '{}' doctor section must be present", name))
}

/// `validate --json`'s finding count, whatever its exit code.
fn validate_finding_count(root: &Path) -> u64 {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let output = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .get_output()
        .clone();
    let val: Value = serde_json::from_slice(&output.stdout).unwrap();
    val["findings"].as_array().unwrap().len() as u64
}

#[test]
fn test_doctor_validation_section_matches_validate_on_duplicate_planning_ids() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2", "Two");
    write_duplicate_of(&stories_dir, "E1S2", "E1S2-dup");

    let expected = validate_finding_count(root);
    assert_eq!(
        expected, 2,
        "the retrospective's reproduction is two duplicate_planning_id findings"
    );

    let sections = doctor_sections(root);
    let validation = section(&sections, "validation");
    assert_eq!(validation["status"], "ok");
    assert_eq!(
        validation["finding_count"].as_u64().unwrap(),
        expected,
        "doctor must report the same count validate reports: {}",
        validation
    );
    assert_eq!(validation["findings_by_code"]["duplicate_planning_id"], 2);

    // The half-answer that misled the reader in the first place: the cache section still
    // reports zero, because a duplicate planning id is invisible to the `findings` table.
    assert_eq!(section(&sections, "cache")["finding_count"], 0);
}

#[test]
fn test_doctor_counts_a_hydration_only_finding_exactly_once() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_schema_violation(&root.join("docs/specs/stories"), "E1S1");

    let sections = doctor_sections(root);
    let validation = section(&sections, "validation");

    assert_eq!(validation["status"], "ok");
    assert_eq!(
        validation["finding_count"], 1,
        "a cache-native finding must be counted once, not once per source: {}",
        validation
    );
    assert_eq!(validation["findings_by_code"]["schema_violation"], 1);
    assert_eq!(
        validation["finding_count"].as_u64().unwrap(),
        validate_finding_count(root)
    );
    // The cache section keeps meaning cache-native findings, so this one appears there too.
    assert_eq!(section(&sections, "cache")["finding_count"], 1);
}

#[test]
fn test_doctor_validation_section_reports_the_sum_of_both_finding_kinds() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_schema_violation(&root.join("docs/specs/stories"), "E1S1");
    write_orphan_dw(root, "DW-a1b2");

    let sections = doctor_sections(root);
    let validation = section(&sections, "validation");

    assert_eq!(validation["findings_by_code"]["schema_violation"], 1);
    assert_eq!(validation["findings_by_code"]["orphan_deferred_work"], 1);
    assert_eq!(
        validation["finding_count"], 2,
        "the total is the sum of cache-native and computed findings: {}",
        validation
    );
    assert_eq!(
        validation["finding_count"].as_u64().unwrap(),
        validate_finding_count(root)
    );
    // Only the schema_violation lives in the findings table.
    assert_eq!(section(&sections, "cache")["finding_count"], 1);
}

#[test]
fn test_doctor_clean_workspace_reports_zero_validation_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(&root.join("docs/specs/stories"), "E1S1", "One");

    let sections = doctor_sections(root);
    let validation = section(&sections, "validation");

    assert_eq!(validation["status"], "ok");
    assert_eq!(validation["finding_count"], 0);
    assert_eq!(
        validation["findings_by_code"],
        serde_json::json!({}),
        "no defects means an empty breakdown, not absent fields: {}",
        validation
    );
    assert_eq!(validate_finding_count(root), 0);
}

#[test]
fn test_doctor_section_order_is_cache_then_validation_with_no_duplicate_keys() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Determinism: compare two runs to *each other*, not to a literal.
    let first = doctor_sections(root);
    let second = doctor_sections(root);
    let names: Vec<&str> = first.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        vec!["cache", "validation", "leases"],
        "cache must be reported before validation and validation before leases"
    );
    assert_eq!(
        first.iter().map(|s| s["name"].clone()).collect::<Vec<_>>(),
        second.iter().map(|s| s["name"].clone()).collect::<Vec<_>>(),
        "section order must not vary between runs"
    );

    // Serde would have silently collapsed a duplicated key on the way in, so the raw text is
    // the only place a collision within a section is observable.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let raw = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let cache_at = raw.find("\"cache\"").unwrap();
    let validation_at = raw.find("\"validation\"").unwrap();
    let leases_at = raw.find("\"leases\"").unwrap();
    assert!(
        cache_at < validation_at && validation_at < leases_at,
        "sections must be serialized cache-first then validation then leases: {}",
        raw
    );
    // Asserted per section rather than as a global count, so a third section added by a later
    // epic — which `default_doctor_sections` exists to allow — does not fail this test. The raw
    // text of each section object is recovered by brace matching, because serde silently
    // collapses a duplicated key on the way in.
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    let sections_at = raw.find("\"sections\"").unwrap();
    let array_start = raw[sections_at..].find('[').unwrap() + sections_at;
    let mut object_texts: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut current_start = 0usize;
    for (idx, ch) in raw[array_start..].char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    current_start = idx;
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    object_texts
                        .push(raw[array_start + current_start..=array_start + idx].to_string());
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }
    assert_eq!(
        object_texts.len(),
        parsed["sections"].as_array().unwrap().len(),
        "failed to recover each section's raw text from {}",
        raw
    );
    for (section_value, section_text) in parsed["sections"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&object_texts)
    {
        let name = section_value["name"].as_str().unwrap();
        for key in section_value.as_object().unwrap().keys() {
            assert_eq!(
                section_text.matches(&format!("\"{}\":", key)).count(),
                1,
                "key {} appears more than once inside section {}: {}",
                key,
                name,
                section_text
            );
        }
    }
}

/// The per-code breakdown is documented as code-sorted and stable across runs, which needs at
/// least two distinct codes to mean anything — with one code, any order is sorted.
#[test]
fn test_doctor_findings_by_code_is_code_sorted_and_stable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_duplicate_of(&stories_dir, "E1S1", "E1S1-dup");
    write_schema_violation(&stories_dir, "E1S9");
    write_orphan_dw(root, "DW-a1b2");

    let raw_of = |root: &Path| -> String {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd
            .current_dir(root)
            .args(["doctor", "--json"])
            .assert()
            .success();
        String::from_utf8(assert.get_output().stdout.clone()).unwrap()
    };

    let raw = raw_of(root);
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    let validation = section(parsed["sections"].as_array().unwrap(), "validation");
    let codes: Vec<&str> = validation["findings_by_code"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert!(
        codes.len() >= 2,
        "the fixture must produce at least two codes, got {:?}",
        codes
    );

    // Key order in the serialized text, not just in the parsed map.
    let breakdown_at = raw.find("\"findings_by_code\"").unwrap();
    let breakdown = &raw[breakdown_at..];
    let mut positions: Vec<usize> = Vec::new();
    for code in &codes {
        positions.push(breakdown.find(&format!("\"{}\"", code)).unwrap());
    }
    let mut sorted_positions = positions.clone();
    sorted_positions.sort_unstable();
    assert_eq!(
        positions, sorted_positions,
        "codes must be emitted in sorted order: {:?} at {:?}",
        codes, positions
    );

    // Determinism is asserted on the breakdown, not on the whole payload: the `cache` section
    // carries `last_synced_at`, which moves whenever the two runs straddle a second boundary —
    // comparing the entire document made this test flaky rather than strict.
    let second_raw = raw_of(root);
    let second: Value = serde_json::from_str(&second_raw).unwrap();
    let second_validation = section(second["sections"].as_array().unwrap(), "validation");
    let second_codes: Vec<&str> = second_validation["findings_by_code"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(
        codes, second_codes,
        "the breakdown's key order must not vary between runs"
    );
    assert_eq!(
        validation["finding_count"], second_validation["finding_count"],
        "the count must not vary between runs"
    );
}

#[test]
fn test_doctor_text_output_contains_validation_section() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2", "Two");
    write_duplicate_of(&stories_dir, "E1S2", "E1S2-dup");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["doctor"]).assert().success();
    let text = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(text.contains("[validation]"), "text output: {}", text);
    assert!(text.contains("finding_count = 2"), "text output: {}", text);
    assert!(
        text.find("[cache]").unwrap() < text.find("[validation]").unwrap(),
        "text output: {}",
        text
    );
}

/// The `validation` section is only as good as the workspace root and `[storage]` layout it is
/// constructed with. Replacing either with a default keeps every other test green, because their
/// fixtures use the default layout and run from the root — and on a configured workspace the
/// duplicate-id scan would then walk a directory that does not exist, returning zero findings
/// with `status: ok`: exactly the confident `0` this change exists to remove.
#[test]
fn test_doctor_validation_honours_configured_specs_dir_and_runs_from_a_subdirectory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str("\n[storage]\nspecs_dir = \"planning/specs\"\n");
    fs::write(&toml_path, toml).unwrap();

    let stories_dir = root.join("planning/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_duplicate_of(&stories_dir, "E1S1", "E1S1-dup");

    let expected = validate_finding_count(root);
    assert_eq!(
        expected, 2,
        "the fixture must produce the two duplicate-id findings validate reports"
    );

    let sections = doctor_sections(root);
    let validation = section(&sections, "validation");
    assert_eq!(validation["status"], "ok");
    assert_eq!(
        validation["finding_count"].as_u64().unwrap(),
        expected,
        "doctor must count the configured layout's findings, not the default layout's"
    );
    assert_eq!(validation["findings_by_code"]["duplicate_planning_id"], 2);

    // Run from a nested subdirectory: the section's root must be the workspace root, not the cwd.
    let nested = root.join("planning/specs/stories");
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(&nested)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let from_subdir = section(val["sections"].as_array().unwrap(), "validation");
    assert_eq!(
        from_subdir["finding_count"].as_u64().unwrap(),
        expected,
        "invoking from a subdirectory must not change the count"
    );
}

/// The payload schema is only exercised against a clean workspace elsewhere, so the two shapes
/// most likely to violate it — a non-empty breakdown, and the all-null `unavailable` report —
/// were never validated. This pins the defective-workspace shape against the printed schema.
#[test]
fn test_doctor_defective_workspace_output_matches_its_printed_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_duplicate_of(&stories_dir, "E1S1", "E1S1-dup");
    write_schema_violation(&stories_dir, "E1S9");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let schema_assert = cmd
        .current_dir(root)
        .args(["schema", "payload", "doctor"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let doctor_assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let payload: Value = serde_json::from_slice(&doctor_assert.get_output().stdout).unwrap();

    let validator = jsonschema::validator_for(&schema).expect("the printed schema must compile");
    let errors: Vec<String> = validator
        .iter_errors(&payload)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "defective-workspace doctor output must validate against its own schema: {errors:?}"
    );

    let validation = section(payload["sections"].as_array().unwrap(), "validation");
    assert!(
        validation["findings_by_code"].as_object().unwrap().len() >= 2,
        "the fixture must exercise a non-empty, multi-code breakdown"
    );
}
