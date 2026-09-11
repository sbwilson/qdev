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

/// The identity rule's sites, enumerated.
///
/// "Which file holds entity X" has exactly one answer (`docs/architecture.md`, *Identity: a file
/// is named for the entity it holds*), and no type signature expresses that: a `PathBuf` built
/// from an id in a new function looks like any other `PathBuf`, so a second answer can appear
/// without a compiler or a reviewer noticing — which is how the write path came to resolve
/// `E1S7.MD` on macOS and not on Linux. This test enumerates the sites by reading the source,
/// the way the AD-1 bans above do, and fails naming the file and line when a new one appears.
///
/// Adding a legitimate site means adding it here, with the reason, rather than silencing the
/// test: the allow-list is the enumeration, and it is short on purpose.
const IDENTITY_RULE_SITES: &[(&str, &str, &str)] = &[
    (
        "qdev-core/src/write.rs",
        "format!(\"{}.md\", id)",
        "`canonical_file_name` — the one spelling a writer creates",
    ),
    (
        "qdev-cli/src/main.rs",
        "format!(\"{}/{}\", dir, new_name)",
        "`--fix-ids`' rename plan: the name comes from `renamed_file_name`, the rule's own \
         helper, and only the directory is joined to it",
    ),
    (
        "qdev-core/src/write.rs",
        "let abs_path = options.workspace_root.join(&rel_dir).join(&file_name);",
        "`create_story` joins the directory the rule gives to the name `canonical_file_name` \
         gives — the rule spelled once, not a second answer",
    ),
    (
        "qdev-core/src/write.rs",
        "if abs_path.symlink_metadata().is_ok() {",
        "`create_story`'s refusal to clobber: an occupancy question about the path it is about \
         to write, asked of the path it just built",
    ),
    (
        "qdev-cli/src/main.rs",
        "let new_abs_path = root.join(&new_rel_path);",
        "`--fix-ids` resolves its rename target to an absolute path before writing it",
    ),
    (
        "qdev-cli/src/main.rs",
        "let occupied = new_abs_path.symlink_metadata().is_ok();",
        "`--fix-ids` asks whether the rename target is occupied — an occupancy question, not a \
         resolution one; which file holds the id is asked of `find_file_in_dir_for_id` beside it",
    ),
];

/// The rule's own helpers. A binding assigned from one of them holds an entity file name, so
/// what a later line does with that binding is part of the rule's surface.
const RULE_HELPERS: [&str; 2] = ["canonical_file_name(", "renamed_file_name("];

/// A source line that builds a markdown file name out of a value: a second spelling of the
/// identity rule wherever it is not the rule itself.
fn builds_a_markdown_name(line: &str) -> bool {
    if is_comment(line) {
        return false;
    }
    (line.contains("format!") || line.contains(".join("))
        && line.contains(".md\"")
        && line.contains("{}")
}

/// A source line that asks the filesystem whether a path exists.
fn probes_the_filesystem(line: &str) -> bool {
    !is_comment(line)
        && (line.contains(".is_file()")
            || line.contains(".exists()")
            || line.contains(".symlink_metadata()"))
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// The binding a `let` introduces, if the line is one.
fn binding_of(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let name = rest.split(['=', ':', ';', ' ']).next()?.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(name)
}

fn mentions_binding(line: &str, binding: &str) -> bool {
    line.match_indices(binding).any(|(idx, _)| {
        let before = line[..idx].chars().next_back();
        let after = line[idx + binding.len()..].chars().next();
        let boundary = |c: Option<char>| !matches!(c, Some(c) if c.is_alphanumeric() || c == '_');
        boundary(before) && boundary(after)
    })
}

/// Every line in one source file that answers "which file holds entity X" — reported as
/// `(line number, why)`.
///
/// The scan is stateful because the sites are: the probe this story deleted was written as
/// `let direct = dir.join(canonical_file_name(id));` and `if direct.is_file() {` on the *next*
/// line, so a per-line predicate reintroduces it silently. A binding whose value came from one
/// of [`RULE_HELPERS`] — or from a line already flagged as a site — is therefore tracked, and a
/// later line that probes the filesystem for it, or builds a path out of it, is the site,
/// however far away it sits. A path built from a *literal* name (`qdev.toml`, the cache
/// directory) maps no id and is tracked by nothing.
fn identity_rule_sites_in(content: &str) -> Vec<(usize, &'static str)> {
    let mut sites = Vec::new();
    let mut tracked: Vec<String> = Vec::new();
    let mut pending: Option<(String, bool)> = None;
    // `#[cfg(test)]` module bodies write fixture files by name; they are not the write path. Only
    // the body is skipped — the file continues to be scanned after it, because cutting at the
    // first occurrence would leave everything below unscanned forever.
    let mut in_test_mod = false;
    let mut pending_test_mod = false;
    let mut depth: i32 = 0;

    for (idx, line) in content.lines().enumerate() {
        if in_test_mod {
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if depth <= 0 {
                in_test_mod = false;
            }
            continue;
        }
        if pending_test_mod && line.contains('{') {
            pending_test_mod = false;
            in_test_mod = true;
            depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
            continue;
        }
        if line.contains("#[cfg(test)]") {
            pending_test_mod = true;
            continue;
        }

        let mentions_tracked = tracked.iter().any(|b| mentions_binding(line, b));
        // A path is joined (`.join(`) or spelled with a separator (`format!("{}/{}", …)`); a
        // `format!` that merely *reports* a path in a message builds nothing.
        let builds_a_path =
            !is_comment(line) && (line.contains(".join(") || line.contains("\"{}/"));

        let why = if builds_a_markdown_name(line) {
            Some("builds an entity file name from a value")
        } else if builds_a_path && mentions_tracked {
            Some("builds a path from a name the identity rule produced")
        } else if probes_the_filesystem(line)
            && ((line.contains(".join(") && !line.contains('"')) || mentions_tracked)
        {
            Some("probes the filesystem for a path built from a value")
        } else {
            None
        };
        if let Some(why) = why {
            sites.push((idx + 1, why));
        }

        // A binding takes the rule with it: from a helper call, or from a line that is itself a
        // site. A probe is not carried, because it yields a `bool`, not a path.
        let carries = RULE_HELPERS.iter().any(|helper| line.contains(helper))
            || matches!(
                why,
                Some("builds an entity file name from a value")
                    | Some("builds a path from a name the identity rule produced")
            );
        // A `let` whose initialiser spans lines (`let p = match … { … };`) is still one binding,
        // so it stays open until its `;` and takes the rule with it if any line inside it had
        // the rule. `--fix-ids` writes its rename target that way.
        if let Some((binding, saw)) = pending.as_mut() {
            *saw = *saw || carries;
            if line.trim_end().ends_with(';') {
                if *saw {
                    tracked.push(binding.clone());
                }
                pending = None;
            }
        } else if let Some(binding) = binding_of(line) {
            if line.trim_end().ends_with(';') {
                if carries {
                    tracked.push(binding.to_string());
                }
            } else {
                pending = Some((binding.to_string(), carries));
            }
        }
    }
    sites
}

#[test]
fn test_the_identity_rule_has_exactly_one_set_of_sites() {
    let crate_roots = [
        (
            "qdev-core",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        ),
        (
            "qdev-cli",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../qdev-cli/src")
                .canonicalize()
                .expect("qdev-cli/src must exist"),
        ),
    ];

    let mut offenders = Vec::new();
    for (crate_name, src_dir) in &crate_roots {
        let mut rs_files = Vec::new();
        collect_rs_files(src_dir, &mut rs_files);
        assert!(!rs_files.is_empty(), "No .rs files found in {crate_name}");
        rs_files.sort();

        for path in rs_files {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
            let rel = format!(
                "{}/src/{}",
                crate_name,
                path.strip_prefix(src_dir).unwrap().display()
            )
            .replace('\\', "/");

            let lines: Vec<&str> = content.lines().collect();
            for (line_no, why) in identity_rule_sites_in(&content) {
                let line = lines[line_no - 1];
                let allowed = IDENTITY_RULE_SITES
                    .iter()
                    .any(|(file, snippet, _)| *file == rel && line.contains(snippet));
                if !allowed {
                    offenders.push(format!("{}:{}: {} -- {}", rel, line_no, why, line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "A second answer to \"which file holds entity X\" appeared. The identity rule has one \
         implementation -- `write::filename_carries_id` for the name, `write::directory_for_kind` \
         for the directory, `write::canonical_file_name` for the spelling writers create -- and \
         `write::resolve_entity_file` is the only door onto it. Route these sites through it, or, \
         if one genuinely belongs to the rule, add it to IDENTITY_RULE_SITES in this file with \
         the reason:\n  {}",
        offenders.join("\n  ")
    );
}

/// The enumeration is only worth having if it actually fires, so its shapes are pinned directly:
/// a change that neuters the scan — and so lets a second answer in silently — fails here rather
/// than passing quietly forever.
#[test]
fn test_the_identity_rule_scan_recognises_a_second_answer() {
    let found = |src: &str| !identity_rule_sites_in(src).is_empty();

    assert!(found("    let path = dir.join(format!(\"{}.md\", id));\n"));
    assert!(found(
        "        let rel = format!(\"{}/{}.md\", dir, story_id);\n"
    ));
    assert!(found(
        "    if dir.join(canonical_file_name(id)).is_file() {\n"
    ));
    assert!(found("    let present = root.join(rel_path).exists();\n"));

    // The exact two-statement shape this story deleted: build on one line, probe on the next.
    // A per-line predicate misses it, which would let the defect back in unnoticed.
    assert!(found(
        "    let direct = dir.join(canonical_file_name(id));\n    if direct.is_file() {\n"
    ));
    // And the same shape with the probe further away, since nothing makes it adjacent.
    assert!(found(
        "    let direct = dir.join(canonical_file_name(id));\n    let x = 1;\n    if direct.symlink_metadata().is_ok() {\n"
    ));
    // A name the rule produced, then joined into a path somewhere else.
    assert!(found(
        "    let new_name = renamed_file_name(old, a, b);\n    let rel = format!(\"{}/{}\", dir, new_name);\n"
    ));

    // Prose is not a site.
    assert!(!found(
        "    /// e.g. format!(\"{}.md\", id) -- prose, not a site\n"
    ));
    assert!(!found(
        "    // dir.join(name).is_file() is what this deleted\n"
    ));
    // A literal path maps no id, so probing one is not this rule's business.
    assert!(!found("    let path = dir.join(\"qdev.toml\");\n"));
    assert!(!found("    if root.join(\"qdev.toml\").exists() {\n"));
    // A binding that never touched the rule is not tracked.
    assert!(!found("    let n = 1;\n    if n.exists() {\n"));

    // A `#[cfg(test)]` module body is skipped, and the scan resumes after it.
    let with_test_mod = "#[cfg(test)]\nmod tests {\n    let path = dir.join(format!(\"{}.md\", id));\n}\nlet after = dir.join(format!(\"{}.md\", id));\n";
    let sites = identity_rule_sites_in(with_test_mod);
    assert_eq!(sites.len(), 1, "{sites:?}");
    assert_eq!(sites[0].0, 5, "the scan must resume after the test module");
}
