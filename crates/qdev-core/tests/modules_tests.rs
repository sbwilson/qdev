use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

use qdev_core::config::{load_config, Config, ModuleConfig};
use qdev_core::modules::ModuleRegistry;
use qdev_core::validate::find_unmatched_module_globs;

#[test]
fn test_module_registry_from_config_and_lookup() {
    let config = Config {
        modules: vec![
            ModuleConfig {
                id: "core".to_string(),
                paths: vec!["crates/core/**".to_string()],
                layer: Some(1),
                may_depend_on: vec![],
            },
            ModuleConfig {
                id: "cli".to_string(),
                paths: vec!["crates/cli/**".to_string()],
                layer: Some(2),
                may_depend_on: vec!["core".to_string()],
            },
        ],
        ..Default::default()
    };

    let registry = ModuleRegistry::from_config(&config);
    assert_eq!(registry.modules().len(), 2);
    assert!(registry.is_registered("core"));
    assert!(registry.is_registered("cli"));
    assert!(!registry.is_registered("unknown"));

    let core_mod = registry.get("core").expect("core module should exist");
    assert_eq!(core_mod.id, "core");
    assert_eq!(core_mod.layer, Some(1));
    assert!(core_mod.may_depend_on.is_empty());

    let map = registry.to_module_paths_map();
    let mut expected = BTreeMap::new();
    expected.insert("core".to_string(), vec!["crates/core/**".to_string()]);
    expected.insert("cli".to_string(), vec!["crates/cli/**".to_string()]);
    assert_eq!(map, expected);
}

#[test]
fn test_module_registry_resolve_path_single_module() {
    let modules = vec![ModuleConfig {
        id: "qdev-core".to_string(),
        paths: vec!["crates/qdev-core/**".to_string()],
        layer: None,
        may_depend_on: vec![],
    }];
    let registry = ModuleRegistry::new(modules);

    let resolved = registry.resolve_path("crates/qdev-core/src/lib.rs");
    assert_eq!(resolved, vec!["qdev-core"]);
}

#[test]
fn test_module_registry_resolve_path_zero_matches() {
    let modules = vec![ModuleConfig {
        id: "core".to_string(),
        paths: vec!["crates/core/**".to_string()],
        layer: None,
        may_depend_on: vec![],
    }];
    let registry = ModuleRegistry::new(modules);

    let resolved = registry.resolve_path("docs/index.md");
    assert!(resolved.is_empty());
}

#[test]
fn test_module_registry_resolve_path_multiple_modules_declaration_order() {
    let modules = vec![
        ModuleConfig {
            id: "mod-a".to_string(),
            paths: vec!["shared/**".to_string(), "crates/common/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        },
        ModuleConfig {
            id: "mod-b".to_string(),
            paths: vec!["crates/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        },
    ];
    let registry = ModuleRegistry::new(modules);

    let resolved = registry.resolve_path("crates/common/src/lib.rs");
    assert_eq!(resolved, vec!["mod-a", "mod-b"]);
}

#[test]
fn test_module_registry_resolve_path_normalization() {
    let modules = vec![ModuleConfig {
        id: "core".to_string(),
        paths: vec!["crates/core/**".to_string()],
        layer: None,
        may_depend_on: vec![],
    }];
    let registry = ModuleRegistry::new(modules);

    // Backslashes and leading ./
    assert_eq!(
        registry.resolve_path(r".\crates\core\src\lib.rs"),
        vec!["core"]
    );
    assert_eq!(
        registry.resolve_path("./crates/core/src/lib.rs"),
        vec!["core"]
    );
    assert_eq!(
        registry.resolve_path("/crates/core/src/lib.rs"),
        vec!["core"]
    );
    assert_eq!(
        registry.resolve_path(r"./crates\core/src/lib.rs"),
        vec!["core"]
    );
    assert_eq!(
        registry.resolve_path("/./crates/core/src/lib.rs"),
        vec!["core"]
    );
    assert_eq!(
        registry.resolve_path("//./crates/core/src/lib.rs"),
        vec!["core"]
    );
}

#[test]
fn test_module_registry_resolve_path_deduplicates_overlapping_globs_within_same_module() {
    let modules = vec![ModuleConfig {
        id: "core".to_string(),
        paths: vec![
            "crates/core/**".to_string(),
            "crates/core/src/**".to_string(),
            "crates/core/src/lib.rs".to_string(),
        ],
        layer: None,
        may_depend_on: vec![],
    }];
    let registry = ModuleRegistry::new(modules);

    let resolved = registry.resolve_path("crates/core/src/lib.rs");
    assert_eq!(resolved, vec!["core"]);
}

#[test]
fn test_module_registry_validate_target_modules() {
    let modules = vec![
        ModuleConfig {
            id: "core".to_string(),
            paths: vec!["crates/core/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        },
        ModuleConfig {
            id: "cli".to_string(),
            paths: vec!["crates/cli/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        },
    ];
    let registry = ModuleRegistry::new(modules);

    let valid_targets = vec!["core".to_string(), "cli".to_string()];
    assert!(registry.validate_target_modules(&valid_targets).is_ok());

    let invalid_targets = vec![
        "core".to_string(),
        "ghost".to_string(),
        "phantom".to_string(),
        "ghost".to_string(),
    ];
    let err = registry
        .validate_target_modules(&invalid_targets)
        .expect_err("should return unregistered modules");
    assert_eq!(err, vec!["ghost", "phantom"]);
}

#[test]
fn test_config_load_valid_modules() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml = r#"
[project]
name = "ModProject"

[[modules]]
id = "core"
paths = ["crates/core/**"]
layer = 1
may_depend_on = ["utils"]

[[modules]]
id = "utils"
paths = ["crates/utils/**"]
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    let loaded = load_config(root).expect("Configuration with valid modules must load");
    assert_eq!(loaded.config.modules.len(), 2);
    assert_eq!(loaded.config.modules[0].id, "core");
    assert_eq!(loaded.config.modules[0].layer, Some(1));
    assert_eq!(loaded.config.modules[0].may_depend_on, vec!["utils"]);
    assert_eq!(loaded.config.modules[1].id, "utils");

    assert_eq!(
        loaded.QDEV_MODULE_PATHS.get("core").unwrap(),
        &vec!["crates/core/**".to_string()]
    );
    assert_eq!(
        loaded.qdev_module_paths.get("utils").unwrap(),
        &vec!["crates/utils/**".to_string()]
    );
}

#[test]
fn test_config_load_duplicate_module_id_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml = r#"
[project]
name = "DupProject"

[[modules]]
id = "core"
paths = ["crates/core1/**"]

[[modules]]
id = "core"
paths = ["crates/core2/**"]
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    let err = load_config(root).expect_err("Duplicate module ID must be rejected");
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code().as_i32(), 2);
    let details = err.details().expect("details must be present");
    assert_eq!(details["file"], "qdev.toml");
    assert_eq!(details["key"], "modules.id");
}

#[test]
fn test_config_load_empty_module_id_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml_empty = r#"
[project]
name = "EmptyIdProject"

[[modules]]
id = ""
paths = ["crates/core/**"]
"#;
    fs::write(root.join("qdev.toml"), toml_empty).unwrap();

    let err = load_config(root).expect_err("Empty module ID must be rejected");
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code().as_i32(), 2);
    let details = err.details().expect("details must be present");
    assert_eq!(details["file"], "qdev.toml");
    assert_eq!(details["key"], "modules.id");

    let toml_ws = r#"
[project]
name = "WhitespaceIdProject"

[[modules]]
id = "   "
paths = ["crates/core/**"]
"#;
    fs::write(root.join("qdev.toml"), toml_ws).unwrap();

    let err_ws = load_config(root).expect_err("Whitespace module ID must be rejected");
    assert_eq!(err_ws.code(), "usage_error");
    assert_eq!(err_ws.exit_code().as_i32(), 2);
    let details_ws = err_ws.details().expect("details must be present");
    assert_eq!(details_ws["file"], "qdev.toml");
    assert_eq!(details_ws["key"], "modules.id");

    let toml_untrimmed = r#"
[project]
name = "UntrimmedIdProject"

[[modules]]
id = " core "
paths = ["crates/core/**"]
"#;
    fs::write(root.join("qdev.toml"), toml_untrimmed).unwrap();

    let err_untrimmed = load_config(root).expect_err("Untrimmed module ID must be rejected");
    assert_eq!(err_untrimmed.code(), "usage_error");
    assert_eq!(err_untrimmed.exit_code().as_i32(), 2);
    let details_untrimmed = err_untrimmed.details().expect("details must be present");
    assert_eq!(details_untrimmed["file"], "qdev.toml");
    assert_eq!(details_untrimmed["key"], "modules.id");
}

#[test]
fn test_config_load_empty_paths_array_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml = r#"
[project]
name = "EmptyPathsProject"

[[modules]]
id = "core"
paths = []
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    let err = load_config(root).expect_err("Empty paths array must be rejected");
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code().as_i32(), 2);
    let details = err.details().expect("details must be present");
    assert_eq!(details["file"], "qdev.toml");
    assert_eq!(details["key"], "modules.paths");
}

#[test]
fn test_config_load_empty_glob_string_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml = r#"
[project]
name = "EmptyGlobProject"

[[modules]]
id = "core"
paths = [""]
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    let err = load_config(root).expect_err("Empty glob string must be rejected");
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code().as_i32(), 2);
    let details = err.details().expect("details must be present");
    assert_eq!(details["file"], "qdev.toml");
    assert_eq!(details["key"], "modules.paths");
}

#[test]
fn test_find_unmatched_module_globs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create a workspace structure
    fs::create_dir_all(root.join("crates/core/src")).unwrap();
    fs::write(root.join("crates/core/src/lib.rs"), "// core").unwrap();

    let config = Config {
        modules: vec![
            ModuleConfig {
                id: "core".to_string(),
                paths: vec!["crates/core/**".to_string()],
                layer: None,
                may_depend_on: vec![],
            },
            ModuleConfig {
                id: "nonexistent".to_string(),
                paths: vec!["nonexistent/**".to_string()],
                layer: None,
                may_depend_on: vec![],
            },
        ],
        ..Default::default()
    };

    let findings = find_unmatched_module_globs(root, &config).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "module_glob_unmatched");
    assert_eq!(findings[0].severity, "warning");
    assert_eq!(findings[0].path, "qdev.toml");
    let msg = findings[0].message.as_deref().unwrap();
    assert!(msg.contains("nonexistent"));
}
