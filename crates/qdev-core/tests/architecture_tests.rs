use std::fs;
use std::path::Path;
use std::process::Command;

const FORBIDDEN_DEPENDENCIES: &[&str] = &[
    "clap",
    "colored",
    "crossterm",
    "console",
    "termcolor",
    "indicatif",
    "inquire",
    "reqwest",
    "hyper",
    "curl",
    "ureq",
];

#[test]
fn test_qdev_core_manifest_has_no_forbidden_dependencies() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let content = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", manifest_path.display(), e));

    for &forbidden in FORBIDDEN_DEPENDENCIES {
        assert!(
            !content.contains(forbidden),
            "crates/qdev-core/Cargo.toml contains forbidden dependency '{}' violating AD-1",
            forbidden
        );
    }
}

#[test]
fn test_qdev_core_cargo_metadata_has_no_forbidden_dependencies() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--manifest-path",
            manifest_path.to_str().unwrap(),
            "--format-version",
            "1",
        ])
        .output()
        .expect("Failed to execute cargo metadata");

    assert!(output.status.success(), "cargo metadata failed");

    let metadata_str = String::from_utf8(output.stdout).expect("cargo metadata output not utf8");
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_str).expect("failed to parse cargo metadata JSON");

    if let Some(packages) = metadata.get("packages").and_then(|p| p.as_array()) {
        for pkg in packages {
            if pkg.get("name").and_then(|n| n.as_str()) == Some("qdev-core") {
                if let Some(deps) = pkg.get("dependencies").and_then(|d| d.as_array()) {
                    for dep in deps {
                        if let Some(dep_name) = dep.get("name").and_then(|n| n.as_str()) {
                            for &forbidden in FORBIDDEN_DEPENDENCIES {
                                assert_ne!(
                                    dep_name, forbidden,
                                    "qdev-core resolved dependency contains '{}' violating AD-1",
                                    forbidden
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

fn collect_rs_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).expect("failed to read directory") {
            let entry = entry.expect("valid entry");
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
}

#[test]
fn test_qdev_core_sources_do_not_perform_direct_terminal_io() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden_patterns = [
        "println!",
        "print!",
        "eprintln!",
        "eprint!",
        "std::io::stdin",
        "std::io::stdout",
        "std::io::stderr",
    ];

    let mut rs_files = Vec::new();
    collect_rs_files(&src_dir, &mut rs_files);

    assert!(!rs_files.is_empty(), "No .rs files found in src/");

    for path in rs_files {
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));

        for &pattern in &forbidden_patterns {
            assert!(
                !content.contains(pattern),
                "Found forbidden I/O pattern '{}' in {} violating AD-1",
                pattern,
                path.display()
            );
        }
    }
}
