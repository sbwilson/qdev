use std::path::Path;

use qdev_core::gate::adapter::cargo::{normalize_location, relativize_path};
use qdev_core::gate::adapter::{parse_with_adapter, validate_adapter_name, VALID_ADAPTERS};
use qdev_core::gate::GateStatus;

#[test]
fn test_adapter_name_validation() {
    assert!(validate_adapter_name("json").is_ok());
    assert!(validate_adapter_name("cargo").is_ok());
    assert!(validate_adapter_name("xcodebuild").is_ok());

    let err = validate_adapter_name("unsupported").unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert!(err.to_string().contains("Unknown output adapter 'unsupported'"));
    assert_eq!(VALID_ADAPTERS, &["json", "cargo", "xcodebuild"]);
}

#[test]
fn test_json_adapter_disallowed_in_parse_with_adapter() {
    let err = parse_with_adapter("json", "{}", "", 0, None).unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert!(err.to_string().contains("cannot be run as stream adapter"));
}

#[test]
fn test_cargo_adapter_failure_parsing() {
    let fixture = include_str!("fixtures/cargo_test_failure.txt");
    let workspace = Path::new("/workspace");

    let parsed = parse_with_adapter("cargo", fixture, "", 101, Some(workspace)).unwrap();
    assert_eq!(parsed.status, GateStatus::Fail);
    assert_eq!(parsed.summary, "1 of 18 tests failed");
    assert_eq!(parsed.failures.len(), 1);

    let f = &parsed.failures[0];
    assert_eq!(f.location, "crates/bridge/tests/c_abi_round_trip.rs:142");
    assert!(f.message.contains("assertion failed: left == right"));
    assert!(f.message.contains("left: EffectsUnavailable"));
    assert!(f.message.contains("right: EventNotUnderstood"));
}

#[test]
fn test_cargo_adapter_success_parsing() {
    let stdout = r#"
running 18 tests
test test_1 ... ok
test test_18 ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
"#;
    let parsed = parse_with_adapter("cargo", stdout, "", 0, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Pass);
    assert_eq!(parsed.summary, "18 tests passed");
    assert!(parsed.failures.is_empty());
}

#[test]
fn test_cargo_adapter_windows_paths_and_apostrophes() {
    // Windows drive letter path with line and column: C:\dev\project\tests\c_abi.rs:142:9
    let loc = normalize_location(r#"C:\dev\project\tests\c_abi.rs:142:9"#, None);
    assert_eq!(loc, r#"C:\dev\project\tests\c_abi.rs:142"#);

    // Message with internal apostrophe
    let output = r#"
running 1 test
test test_fail ... FAILED

failures:

---- test_fail stdout ----
thread 'test_fail' panicked at 'can\'t find user\'s profile', tests/profile.rs:25:5

failures:
    test_fail

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
    let parsed = parse_with_adapter("cargo", output, "", 101, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Fail);
    assert_eq!(parsed.failures.len(), 1);
    assert_eq!(parsed.failures[0].location, "tests/profile.rs:25");
    assert!(parsed.failures[0].message.contains("can\\'t find user\\'s profile"));
}

#[test]
fn test_cargo_adapter_should_panic_ignored_outside_failures() {
    let output = r#"
running 2 tests
test test_expected_panic ... thread 'test_expected_panic' panicked at tests/foo.rs:10:5:
expected panic happened!
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
ok
test test_normal ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
    let parsed = parse_with_adapter("cargo", output, "", 0, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Pass);
    assert_eq!(parsed.summary, "2 tests passed");
    assert!(parsed.failures.is_empty());
}

#[test]
fn test_cargo_adapter_exit_code_failure_with_no_failed_tests() {
    let output = r#"
running 2 tests
test test_1 ... ok
test test_2 ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
"#;
    // Process exited with non-zero (e.g. 101) despite all tests passing (e.g. panic in benchmark/post-test hook)
    let parsed = parse_with_adapter("cargo", output, "", 101, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Fail);
    assert_eq!(parsed.summary, "cargo test exited with code 101");
}

#[test]
fn test_relativize_path_boundary_check() {
    let root = Path::new("/app");
    // /app_lib/src.rs should NOT be relativized against /app because it is not a boundary match
    assert_eq!(relativize_path("/app_lib/src.rs", Some(root)), "/app_lib/src.rs");
    // /app/src.rs SHOULD be relativized to src.rs
    assert_eq!(relativize_path("/app/src.rs", Some(root)), "src.rs");
}

#[test]
fn test_xcodebuild_adapter_failure_parsing() {
    let fixture = include_str!("fixtures/xcodebuild_test_failure.txt");
    let workspace = Path::new("/Users/developer/project");

    let parsed = parse_with_adapter("xcodebuild", fixture, "", 1, Some(workspace)).unwrap();
    assert_eq!(parsed.status, GateStatus::Fail);
    assert_eq!(parsed.summary, "1 of 18 tests failed");
    assert_eq!(parsed.failures.len(), 1);

    let f = &parsed.failures[0];
    assert_eq!(f.location, "Tests/AppTests/MyTests.swift:42");
    assert_eq!(
        f.message,
        r#"XCTAssertEqual failed: ("EffectsUnavailable") is not equal to ("EventNotUnderstood")"#
    );
}

#[test]
fn test_xcodebuild_adapter_success_parsing() {
    let stdout = r#"
Test Suite 'All tests' started at 2026-09-22 10:00:00.000
	 Executed 18 tests, with 0 failures (0 unexpected) in 1.234 (1.234) seconds
** TEST SUCCEEDED **
"#;
    let parsed = parse_with_adapter("xcodebuild", stdout, "", 0, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Pass);
    assert_eq!(parsed.summary, "18 tests passed");
    assert!(parsed.failures.is_empty());
}

#[test]
fn test_xcodebuild_adapter_no_double_counting_across_nested_suites() {
    let output = r#"
Test Suite 'All tests' started at 2026-09-22 10:00:00.000
Test Suite 'Nested1' started.
	 Executed 5 tests, with 1 failure in 0.1s
Test Suite 'Nested2' started.
	 Executed 5 tests, with 0 failures in 0.1s
Test Suite 'All tests' failed.
	 Executed 10 tests, with 1 failure in 0.2s
** TEST FAILED **
"#;
    let parsed = parse_with_adapter("xcodebuild", output, "", 1, None).unwrap();
    assert_eq!(parsed.status, GateStatus::Fail);
    // Should take the final summary line (10 tests, 1 failure) rather than summing (20 tests, 2 failures)
    assert_eq!(parsed.summary, "1 of 10 tests failed");
}
