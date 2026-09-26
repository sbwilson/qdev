use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

fn setup(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Soup",
            "--developer",
            "tester",
            "--team",
            "core",
        ])
        .assert()
        .success();
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
}

fn configure_soup(root: &Path, fields: &str) {
    let mut config = fs::read_to_string(root.join("qdev.toml")).unwrap();
    config.push_str(&format!("\n[soup]\n{fields}"));
    fs::write(root.join("qdev.toml"), config).unwrap();
}

fn release(root: &Path, file: &str) {
    let dir = root.join("docs/state/releases");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(file), "---\nid: 0.1.0\ntitle: Release\nstatus: planned\nversion: 1\ncreated_by:\n  type: human\n  id: tester\nupdated_by:\n  type: human\n  id: tester\n---\n").unwrap();
}

#[test]
fn audit_records_evidence_and_only_cargo_audit_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    release(root, "0.1.0.md");
    let audit = root.join("audit.sh");
    let deny = root.join("deny.sh");
    executable(&audit, "#!/bin/sh\necho '{\"vulnerabilities\":{\"list\":[{\"package\":{\"name\":\"demo\",\"version\":\"1.0\"},\"advisory\":{\"aliases\":[\"CVE-2026-1\"]}}]}}'\n");
    executable(&deny, "#!/bin/sh\necho deny-ok\n");
    configure_soup(
        root,
        &format!(
            "audit_command = \"{}\"\ndeny_command = \"{}\"\n",
            audit.display(),
            deny.display()
        ),
    );
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "audit", "--release", "0.1.0"])
        .assert()
        .success();
    assert!(root.join("docs/state/soup/demo-1.0.md").is_file());
    let record = fs::read_to_string(root.join("docs/state/soup/demo-1.0.md")).unwrap();
    assert!(record.contains("evaluated_for_release: \"0.1.0\""));
    assert!(record.contains("cve_status: \"CVE-2026-1\""));
    let evidence = root.join("docs/state/evidence/_workspace");
    assert!(fs::read_dir(evidence)
        .unwrap()
        .filter_map(Result::ok)
        .any(|f| f.file_name().to_string_lossy().contains("qdev-soup-audit")));
    assert!(fs::read_dir(root.join("docs/state/evidence/_workspace"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|f| f.file_name().to_string_lossy().contains("qdev-soup-deny")));
}

#[test]
fn audit_requires_configuration() {
    let temp = TempDir::new().unwrap();
    setup(temp.path());
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["soup", "audit", "--json"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn deny_failure_preserves_evidence_without_persisting_audit_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    let audit = root.join("audit.sh");
    let deny = root.join("deny.sh");
    executable(&audit, "#!/bin/sh\necho '{\"vulnerabilities\":{\"list\":[{\"package\":{\"name\":\"demo\",\"version\":\"1.0\"}}]}}'\n");
    executable(&deny, "#!/bin/sh\nexit 1\n");
    configure_soup(
        root,
        &format!(
            "audit_command = \"{}\"\ndeny_command = \"{}\"\n",
            audit.display(),
            deny.display()
        ),
    );
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "audit"])
        .assert()
        .failure()
        .code(1);
    assert!(!root.join("docs/state/soup/demo-1.0.md").exists());
    assert_eq!(
        fs::read_dir(root.join("docs/state/evidence/_workspace"))
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        2
    );
}

#[test]
fn failed_or_unparseable_audit_never_creates_soup_records_but_keeps_evidence() {
    for (name, body, expected_code) in [
        ("failed", "#!/bin/sh\necho '{not json}'\nexit 1\n", 1),
        ("malformed", "#!/bin/sh\necho '{not json}'\n", 0),
        ("generic", "#!/bin/sh\necho audit-complete\n", 0),
    ] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        setup(root);
        let audit = root.join("audit.sh");
        executable(&audit, body);
        configure_soup(root, &format!("audit_command = \"{}\"\n", audit.display()));
        Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args(["soup", "audit"])
            .assert()
            .code(expected_code);
        assert!(
            !root.join("docs/state/soup/example-1.0.md").exists(),
            "{name}"
        );
        assert!(
            fs::read_dir(root.join("docs/state/evidence/_workspace"))
                .unwrap()
                .filter_map(Result::ok)
                .any(|f| f.file_name().to_string_lossy().contains("qdev-soup-audit")),
            "{name}"
        );
    }
}

#[test]
fn sbom_cli_records_artifact_and_refuses_missing_artifact_without_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    release(root, "0.1.0.md");
    let script = root.join("sbom.sh");
    executable(
        &script,
        "#!/bin/sh\nprintf '{}' > bom.json\necho bom.json\n",
    );
    configure_soup(root, &format!("sbom_command = \"{}\"\n", script.display()));
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .success();
    assert!(
        fs::read_to_string(root.join("docs/state/releases/0.1.0.md"))
            .unwrap()
            .contains("sbom_artifact_path: bom.json")
    );

    let failed_temp = TempDir::new().unwrap();
    let failed_root = failed_temp.path();
    setup(failed_root);
    release(failed_root, "0.1.0.md");
    let failed_script = failed_root.join("failed.sh");
    executable(&failed_script, "#!/bin/sh\nprintf '{}' > bom.json\necho bom.json\nexit 1\n");
    configure_soup(
        failed_root,
        &format!("sbom_command = \"{}\"\n", failed_script.display()),
    );
    let before = fs::read_to_string(failed_root.join("docs/state/releases/0.1.0.md")).unwrap();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(failed_root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .failure()
        .code(1);
    assert_eq!(
        before,
        fs::read_to_string(failed_root.join("docs/state/releases/0.1.0.md")).unwrap()
    );

    // A second [soup] table is invalid TOML, so use a fresh workspace for missing-artifact behavior.
    let missing_temp = TempDir::new().unwrap();
    let missing_root = missing_temp.path();
    setup(missing_root);
    release(missing_root, "0.1.0.md");
    let missing_script = missing_root.join("missing.sh");
    executable(&missing_script, "#!/bin/sh\necho absent.json\n");
    configure_soup(
        missing_root,
        &format!("sbom_command = \"{}\"\n", missing_script.display()),
    );
    let before = fs::read_to_string(missing_root.join("docs/state/releases/0.1.0.md")).unwrap();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(missing_root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .failure()
        .code(2);
    assert_eq!(
        before,
        fs::read_to_string(missing_root.join("docs/state/releases/0.1.0.md")).unwrap()
    );
    assert!(
        fs::read_dir(missing_root.join("docs/state/evidence/_workspace"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|f| f.file_name().to_string_lossy().contains("qdev-soup-sbom"))
    );
}

#[test]
fn sbom_requires_config_and_unambiguous_existing_release() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .failure()
        .code(2);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "sbom"])
        .assert()
        .failure()
        .code(2);

    let no_release = TempDir::new().unwrap();
    let no_release_root = no_release.path();
    setup(no_release_root);
    let no_release_script = no_release_root.join("sbom.sh");
    executable(
        &no_release_script,
        "#!/bin/sh\nprintf '{}' > bom.json\necho bom.json\n",
    );
    configure_soup(
        no_release_root,
        &format!("sbom_command = \"{}\"\n", no_release_script.display()),
    );
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(no_release_root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .failure()
        .code(2);
    assert!(!no_release_root.join("bom.json").exists());

    let second = TempDir::new().unwrap();
    let root = second.path();
    setup(root);
    release(root, "0.1.0.md");
    release(root, "0.1.0-copy.md");
    let script = root.join("sbom.sh");
    executable(
        &script,
        "#!/bin/sh\nprintf '{}' > bom.json\necho bom.json\n",
    );
    configure_soup(root, &format!("sbom_command = \"{}\"\n", script.display()));
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["soup", "sbom", "--release", "0.1.0"])
        .assert()
        .failure()
        .code(2);
    assert!(!root.join("bom.json").exists());
}
