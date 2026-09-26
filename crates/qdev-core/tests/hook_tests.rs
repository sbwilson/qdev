use std::fs;
use std::path::Path;
use std::process::Command;

use qdev_core::config::Config;
use qdev_core::hook::{
    inspect_hooks, install_hooks, resolve_hooks_dir, run_legacy_hook, run_pre_commit,
    scan_staged_secrets, shim_content, EXPECTED_HOOKS,
};
use tempfile::TempDir;

fn init_git_repo(path: &Path) {
    let status = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .status()
        .expect("git init failed");
    assert!(status.success());

    // Configure user name and email for commits in tests
    let _ = Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .status();
}

#[test]
fn test_shim_content_generation() {
    for hook in EXPECTED_HOOKS {
        let content = shim_content(hook);
        assert!(content.starts_with("#!/bin/sh\n"));
        assert!(content.contains(&format!("exec qdev hook {} \"$@\"\n", hook)));
        assert!(!content.contains('\r'), "Shims must never contain CRLF");
    }
}

#[test]
fn test_install_hooks_clean_and_inspect() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    // Initial inspection: hooks should be missing
    let status = inspect_hooks(root).unwrap();
    assert!(!status.all_installed);
    assert_eq!(status.missing_hooks.len(), 3);
    assert!(status.outdated_hooks.is_empty());

    // Install hooks
    let report = install_hooks(root).unwrap();
    assert_eq!(report.installed_hooks.len(), 3);
    assert!(report.preserved_legacy.is_empty());

    // Inspection after install: all should be installed
    let status2 = inspect_hooks(root).unwrap();
    assert!(status2.all_installed);
    assert!(status2.missing_hooks.is_empty());
    assert!(status2.outdated_hooks.is_empty());

    // Check executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hooks_dir = resolve_hooks_dir(root).unwrap();
        for hook in EXPECTED_HOOKS {
            let meta = fs::metadata(hooks_dir.join(hook)).unwrap();
            assert_ne!(
                meta.permissions().mode() & 0o111,
                0,
                "Hook {} must be executable",
                hook
            );
        }
    }
}

#[test]
fn test_install_hooks_preserves_legacy_and_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();

    // Create custom pre-commit hook
    let custom_script = "#!/bin/sh\necho 'running custom legacy hook'\nexit 0\n";
    let pre_commit_path = hooks_dir.join("pre-commit");
    fs::write(&pre_commit_path, custom_script).unwrap();

    // First install: should preserve pre-commit as pre-commit.legacy
    let report = install_hooks(root).unwrap();
    assert_eq!(report.installed_hooks.len(), 3);
    assert_eq!(report.preserved_legacy, vec!["pre-commit".to_string()]);

    let legacy_path = hooks_dir.join("pre-commit.legacy");
    assert!(legacy_path.exists());
    assert_eq!(fs::read_to_string(&legacy_path).unwrap(), custom_script);

    // Re-installing should be idempotent: does NOT rename qdev shim to .legacy
    let report2 = install_hooks(root).unwrap();
    assert_eq!(report2.installed_hooks.len(), 3);
    assert!(report2.preserved_legacy.is_empty());
    assert_eq!(fs::read_to_string(&legacy_path).unwrap(), custom_script);
}

#[test]
fn test_inspect_hooks_outdated_detection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    install_hooks(root).unwrap();

    // Overwrite one hook with custom content
    let hooks_dir = root.join(".git").join("hooks");
    fs::write(hooks_dir.join("pre-commit"), "#!/bin/sh\necho outdated\n").unwrap();

    let status = inspect_hooks(root).unwrap();
    assert!(!status.all_installed);
    assert_eq!(status.outdated_hooks, vec!["pre-commit".to_string()]);
    assert!(status.missing_hooks.is_empty());
}

#[test]
fn test_inspect_hooks_non_git_repo() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let err = inspect_hooks(root).unwrap_err();
    assert_eq!(err.code(), "not_a_git_repository");
}

#[test]
fn test_run_legacy_hook_exit_code_propagation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();

    // Create a legacy script that exits with code 42
    let script = "#!/bin/sh\nexit 42\n";
    let legacy_file = hooks_dir.join("pre-commit.legacy");
    fs::write(&legacy_file, script).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&legacy_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&legacy_file, perms).unwrap();
    }

    let code = run_legacy_hook(root, &hooks_dir, "pre-commit", &[], false).unwrap();
    assert_eq!(code, Some(42));
}

#[test]
fn test_staged_secret_scanner_nfr_404() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    let mut config = Config::default();
    config.hygiene.secret_patterns = vec![
        r"SECRET_[0-9]+".to_string(),
        r"ghp_[a-zA-Z0-9]{20,}".to_string(),
    ];

    // 1. Unstaged scratchpad file containing a secret: must NOT be flagged
    let scratch_dir = root.join("docs").join("state").join("scratch");
    fs::create_dir_all(&scratch_dir).unwrap();
    let unstaged_file = scratch_dir.join("work.jsonl");
    fs::write(&unstaged_file, "{\"note\": \"SECRET_12345\"}\n").unwrap();

    let violations = scan_staged_secrets(root, &config).unwrap();
    assert!(
        violations.is_empty(),
        "Unstaged files must never trigger secret scan"
    );

    // 2. Stage a normal source file with a secret: must NOT be flagged (only scratchpad/evidence are scanned)
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let src_file = src_dir.join("main.rs");
    fs::write(&src_file, "// Mock SECRET_999\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "src/main.rs"])
        .current_dir(root)
        .status();

    let violations = scan_staged_secrets(root, &config).unwrap();
    assert!(
        violations.is_empty(),
        "Non-scratchpad/non-evidence files should not trigger secret scan"
    );

    // 3. Stage the scratchpad file with secret: MUST be flagged
    let _ = Command::new("git")
        .args(["add", "docs/state/scratch/work.jsonl"])
        .current_dir(root)
        .status();

    let violations = scan_staged_secrets(root, &config).unwrap();
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].file, "docs/state/scratch/work.jsonl");
    assert_eq!(violations[0].pattern, r"SECRET_[0-9]+");

    // 4. Staging an evidence file with a secret token: MUST be flagged
    let evidence_dir = root.join("docs").join("state").join("evidence");
    fs::create_dir_all(&evidence_dir).unwrap();
    let ev_file = evidence_dir.join("test.json");
    fs::write(&ev_file, "{\"token\": \"ghp_abcdefghijklmnopqrstuvwx\"}\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "docs/state/evidence/test.json"])
        .current_dir(root)
        .status();

    let violations = scan_staged_secrets(root, &config).unwrap();
    assert_eq!(violations.len(), 2);
}

#[test]
fn test_run_pre_commit_secret_blocks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_repo(root);

    let mut config = Config::default();
    config.hygiene.secret_patterns = vec![r"SUPER_SECRET".to_string()];

    // Stage scratchpad containing secret
    let scratch_dir = root.join("docs").join("state").join("scratch");
    fs::create_dir_all(&scratch_dir).unwrap();
    let scratch_file = scratch_dir.join("scratch.jsonl");
    fs::write(&scratch_file, "SUPER_SECRET\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "docs/state/scratch/scratch.jsonl"])
        .current_dir(root)
        .status();

    let err = run_pre_commit(root, &config, &[], None).unwrap_err();
    assert_eq!(err.code(), "secret_detected");
}
