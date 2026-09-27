use std::fs;

use qdev_core::{
    parse_cargo_audit_json, persist_soup_records, record_sbom_artifact, Config, SoupAuditFinding,
};
use tempfile::TempDir;

#[test]
fn cargo_audit_parser_keeps_only_explicit_dependency_facts() {
    let findings = parse_cargo_audit_json(
        r#"{
      "vulnerabilities": {"list": [{
        "package": {"name": "example", "version": "1.2.3", "license": "MIT"},
        "advisory": {"id": "RUSTSEC-2026-0001", "aliases": ["CVE-2026-1234"]}
      }]}
    }"#,
    );
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].name, "example");
    assert_eq!(findings[0].version, "1.2.3");
    assert_eq!(findings[0].license.as_deref(), Some("MIT"));
    assert_eq!(findings[0].cve_status.as_deref(), Some("CVE-2026-1234"));
}

#[test]
fn malformed_or_generic_output_creates_no_dependency_records() {
    assert!(parse_cargo_audit_json("audit completed successfully").is_empty());
    assert!(parse_cargo_audit_json("{}").is_empty());
}

#[test]
fn parser_keeps_every_explicit_cve_alias() {
    let findings = parse_cargo_audit_json(
        r#"{"vulnerabilities":{"list":[{"package":{"name":"example","version":"1.2.3"},"advisory":{"aliases":["CVE-2026-2","CVE-2026-1"]}}]}}"#,
    );
    assert_eq!(findings[0].cve_status.as_deref(), Some("CVE-2026-1, CVE-2026-2"));
}

#[test]
fn persistence_rejects_colliding_normalized_ids_before_writing() {
    let temp = TempDir::new().unwrap();
    let findings = vec![
        SoupAuditFinding { name: "a/b".into(), version: "1".into(), cve_status: None, license: None },
        SoupAuditFinding { name: "a?b".into(), version: "1".into(), cve_status: None, license: None },
    ];
    assert!(persist_soup_records(temp.path(), &Config::default(), &findings, None).is_err());
    assert!(!temp.path().join("docs/state/soup/a-b-1.md").exists());
}

#[test]
fn sbom_artifact_refuses_paths_outside_the_workspace() {
    let temp = TempDir::new().unwrap();
    assert!(record_sbom_artifact(temp.path(), &Config::default(), "0.1.0", "../bom.json").is_err());
}

#[test]
fn sbom_artifact_is_persisted_only_on_a_resolved_release() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let release_dir = root.join("docs/state/releases");
    fs::create_dir_all(&release_dir).unwrap();
    fs::write(
        release_dir.join("0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: planned\nversion: 1\ncreated_by:\n  type: human\n  id: tester\nupdated_by:\n  type: human\n  id: tester\n---\n",
    )
    .unwrap();
    fs::write(root.join("bom.json"), "{}").unwrap();
    record_sbom_artifact(root, &Config::default(), "0.1.0", "bom.json").unwrap();
    assert!(fs::read_to_string(release_dir.join("0.1.0.md"))
        .unwrap()
        .contains("sbom_artifact_path: bom.json"));
}

#[test]
fn reported_parser_warns_on_uninterpretable_output_but_never_hides_findings() {
    use qdev_core::parse_cargo_audit_json_reported;

    // Non-JSON output (wrong command, wrapped output).
    let p = parse_cargo_audit_json_reported("audit completed successfully");
    assert!(p.findings.is_empty());
    assert!(
        p.warning.as_deref().unwrap().contains("not valid JSON"),
        "non-JSON output must be reported, got: {:?}",
        p.warning
    );

    // JSON without the v2 list section (legacy / unexpected format).
    let p = parse_cargo_audit_json_reported(r#"{"vulnerabilities": {}}"#);
    assert!(p.findings.is_empty());
    assert!(
        p.warning.as_deref().unwrap().contains("vulnerabilities/list"),
        "list-less JSON must be reported, got: {:?}",
        p.warning
    );

    // Valid v2 document with an empty list: a genuinely clean audit, no warning.
    let p = parse_cargo_audit_json_reported(r#"{"vulnerabilities": {"list": []}}"#);
    assert!(p.findings.is_empty());
    assert!(p.warning.is_none(), "clean audit must not warn: {:?}", p.warning);

    // Valid v2 document with findings: no warning.
    let p = parse_cargo_audit_json_reported(
        r#"{"vulnerabilities":{"list":[{"package":{"name":"example","version":"1.2.3"}}]}}"#,
    );
    assert_eq!(p.findings.len(), 1);
    assert!(p.warning.is_none());

    // Empty output: neither shape, no warning.
    let p = parse_cargo_audit_json_reported("");
    assert!(p.findings.is_empty());
    assert!(p.warning.is_none());
}
