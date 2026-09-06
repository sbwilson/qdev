use std::fs;
use std::path::Path;

use qdev_core::{
    extract_frontmatter, extract_frontmatter_str, validate_frontmatter,
    validate_frontmatter_detailed, validate_frontmatter_value, EntityKind, SchemaError,
};

#[test]
fn test_detailed_validation_and_value_validation() {
    let raw = serde_json::json!({
        "id": "E12S4",
        "title": "Story Title",
        "status": "draft",
        "version": 1,
        "created_by": { "type": "human", "id": "simon" },
        "updated_by": { "type": "human", "id": "simon" }
    });
    assert!(validate_frontmatter_value(EntityKind::Story, &raw).is_ok());

    let detailed_res = validate_frontmatter_detailed(EntityKind::Story, "---\nid: E12S4\n---");
    assert!(detailed_res.is_err());
    let errs = detailed_res.unwrap_err();
    assert!(!errs.is_empty());
    assert!(!errs[0].to_string().is_empty());
}

#[test]
fn test_all_13_schemas_embedded_and_valid_json() {
    let all_kinds = EntityKind::all();
    assert_eq!(all_kinds.len(), 13);

    for kind in all_kinds {
        let raw_schema = kind.schema_str();
        assert!(!raw_schema.is_empty(), "Schema for {:?} is empty", kind);

        let json_val = kind.schema_json();
        assert!(
            json_val.is_object(),
            "Schema for {:?} is not an object",
            kind
        );

        // Verify that jsonschema compiles the schema without error
        let validator_res = jsonschema::validator_for(&json_val);
        assert!(
            validator_res.is_ok(),
            "Failed to compile schema for {:?}: {:?}",
            kind,
            validator_res.err()
        );
    }
}

#[test]
fn test_attribution_and_required_fields_contract_on_all_schemas() {
    let kinds_with_title = [
        EntityKind::Prd,
        EntityKind::Requirement,
        EntityKind::Epic,
        EntityKind::Story,
        EntityKind::Adr,
        EntityKind::Hazard,
        EntityKind::Sprint,
        EntityKind::Release,
        EntityKind::DeferredWork,
        EntityKind::Decision,
    ];

    let kinds_without_title = [
        EntityKind::Scratchpad,
        EntityKind::Soup,
        EntityKind::Evidence,
    ];

    for kind in kinds_with_title {
        let json = kind.schema_json();
        let required = json["required"]
            .as_array()
            .expect("required must be an array");
        let req_keys: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        assert!(req_keys.contains(&"id"), "{:?} missing required 'id'", kind);
        assert!(
            req_keys.contains(&"title"),
            "{:?} missing required 'title'",
            kind
        );
        assert!(
            req_keys.contains(&"status"),
            "{:?} missing required 'status'",
            kind
        );
        assert!(
            req_keys.contains(&"version"),
            "{:?} missing required 'version'",
            kind
        );
        assert!(
            req_keys.contains(&"created_by"),
            "{:?} missing required 'created_by'",
            kind
        );
        assert!(
            req_keys.contains(&"updated_by"),
            "{:?} missing required 'updated_by'",
            kind
        );
    }

    for kind in kinds_without_title {
        let json = kind.schema_json();
        let required = json["required"]
            .as_array()
            .expect("required must be an array");
        let req_keys: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

        assert!(req_keys.contains(&"id"), "{:?} missing required 'id'", kind);
        assert!(
            !req_keys.contains(&"title"),
            "{:?} should not require 'title'",
            kind
        );
        assert!(
            req_keys.contains(&"status"),
            "{:?} missing required 'status'",
            kind
        );
        assert!(
            req_keys.contains(&"version"),
            "{:?} missing required 'version'",
            kind
        );
        assert!(
            req_keys.contains(&"created_by"),
            "{:?} missing required 'created_by'",
            kind
        );
        assert!(
            req_keys.contains(&"updated_by"),
            "{:?} missing required 'updated_by'",
            kind
        );
    }
}

#[test]
fn test_golden_fixtures_validation() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures_dir = manifest_dir.join("tests").join("fixtures");

    for kind in EntityKind::all() {
        let kind_dir = fixtures_dir.join(kind.as_str());
        assert!(
            kind_dir.is_dir(),
            "Missing fixture directory for kind {:?}: {}",
            kind,
            kind_dir.display()
        );

        // 1. Valid fixture must pass with 0 errors
        let valid_file = kind_dir.join("valid.md");
        assert!(
            valid_file.is_file(),
            "Missing valid.md in {}",
            kind_dir.display()
        );
        let valid_content = fs::read_to_string(&valid_file).unwrap();
        let valid_result = validate_frontmatter(*kind, &valid_content);
        assert!(
            valid_result.is_ok(),
            "valid.md failed validation for kind {:?}: {:?}",
            kind,
            valid_result.err()
        );

        // 2. Invalid missing fields fixture must fail
        let invalid_missing_file = kind_dir.join("invalid_missing_fields.md");
        assert!(
            invalid_missing_file.is_file(),
            "Missing invalid_missing_fields.md in {}",
            kind_dir.display()
        );
        let invalid_missing_content = fs::read_to_string(&invalid_missing_file).unwrap();
        let missing_result = validate_frontmatter(*kind, &invalid_missing_content);
        assert!(
            missing_result.is_err(),
            "invalid_missing_fields.md unexpectedly passed for kind {:?}",
            kind
        );
        let errors = missing_result.unwrap_err();
        assert!(
            errors.iter().any(|e| e.contains("required property")),
            "Expected 'required property' error in {:?}, got: {:?}",
            kind,
            errors
        );

        // 3. Invalid attribution fixture must fail
        let invalid_attr_file = kind_dir.join("invalid_attribution.md");
        assert!(
            invalid_attr_file.is_file(),
            "Missing invalid_attribution.md in {}",
            kind_dir.display()
        );
        let invalid_attr_content = fs::read_to_string(&invalid_attr_file).unwrap();
        let attr_result = validate_frontmatter(*kind, &invalid_attr_content);
        assert!(
            attr_result.is_err(),
            "invalid_attribution.md unexpectedly passed for kind {:?}",
            kind
        );
        let attr_errors = attr_result.unwrap_err();
        assert!(
            !attr_errors.is_empty(),
            "Expected validation errors for attribution in {:?}",
            kind
        );
    }
}

#[test]
fn test_missing_attribution_validation_error() {
    let content = r#"---
id: E12S4
title: Test Story
status: draft
version: 1
---

## Body
"#;
    let res = validate_frontmatter(EntityKind::Story, content);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("created_by")),
        "Expected error mentioning created_by, got: {:?}",
        errs
    );
    assert!(
        errs.iter().any(|e| e.contains("updated_by")),
        "Expected error mentioning updated_by, got: {:?}",
        errs
    );
}

#[test]
fn test_invalid_author_type_validation_error() {
    let content = r#"---
id: E12S4
title: Test Story
status: draft
version: 1
created_by: { type: "system", id: "bot" }
updated_by: { type: "human", id: "simon" }
---

## Body
"#;
    let res = validate_frontmatter(EntityKind::Story, content);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("system") || e.contains("human") || e.contains("agent")),
        "Expected error indicating type must be human or agent, got: {:?}",
        errs
    );
}

#[test]
fn test_invalid_relation_structure_error() {
    let content = r#"---
id: E12S4
title: Test Story
status: draft
version: 1
created_by: { type: "human", id: "simon" }
updated_by: { type: "agent", id: "bot" }
relations:
  depends_on: "E12S3"
---

## Body
"#;
    let res = validate_frontmatter(EntityKind::Story, content);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("array") || e.contains("depends_on")),
        "Expected error indicating depends_on must be an array, got: {:?}",
        errs
    );
}

#[test]
fn test_invalid_constraint_kind_error() {
    let content = r#"---
id: E12S4
title: Test Story
status: draft
version: 1
created_by: { type: "human", id: "simon" }
updated_by: { type: "agent", id: "bot" }
constraints:
  - id: NG-1
    kind: forbidden_zone
    text: Cannot enter here
---

## Body
"#;
    let res = validate_frontmatter(EntityKind::Story, content);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("forbidden_zone") || e.contains("no_go")),
        "Expected error indicating kind must be one of no_go, rabbit_hole, appetite, got: {:?}",
        errs
    );
}

#[test]
fn test_invalid_version_zero_or_negative_error() {
    let content = r#"---
id: E12S4
title: Test Story
status: draft
version: 0
created_by: { type: "human", id: "simon" }
updated_by: { type: "agent", id: "bot" }
---

## Body
"#;
    let res = validate_frontmatter(EntityKind::Story, content);
    assert!(res.is_err());
    let errs = res.unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("minimum") || e.contains("1") || e.contains("0")),
        "Expected error indicating version must be >= 1, got: {:?}",
        errs
    );
}

#[test]
fn test_aliases_resolution() {
    assert_eq!(
        EntityKind::from_str_loose("dw").unwrap(),
        EntityKind::DeferredWork
    );
    assert_eq!(
        EntityKind::from_str_loose("deferred_work").unwrap(),
        EntityKind::DeferredWork
    );
    assert_eq!(
        EntityKind::from_str_loose("deferred-work").unwrap(),
        EntityKind::DeferredWork
    );
    assert_eq!(EntityKind::from_str_loose("adrs").unwrap(), EntityKind::Adr);
    assert_eq!(EntityKind::from_str_loose("ADR").unwrap(), EntityKind::Adr);
    assert_eq!(
        EntityKind::from_str_loose("story").unwrap(),
        EntityKind::Story
    );
    assert_eq!(
        EntityKind::from_str_loose("stories").unwrap(),
        EntityKind::Story
    );
    assert_eq!(EntityKind::from_str_loose("prd").unwrap(), EntityKind::Prd);
    assert_eq!(EntityKind::from_str_loose("prds").unwrap(), EntityKind::Prd);
    assert_eq!(
        EntityKind::from_str_loose("gate_run").unwrap(),
        EntityKind::Evidence
    );
    assert_eq!(
        EntityKind::from_str_loose("scratchpad_entry").unwrap(),
        EntityKind::Scratchpad
    );

    let err = EntityKind::from_str_loose("invalid_kind").unwrap_err();
    assert!(err.message().contains("Unknown schema kind 'invalid_kind'"));
    assert!(err.message().contains("Valid schema kinds"));
}

#[test]
fn test_frontmatter_extraction_helpers() {
    let markdown = r#"---
id: E12S4
title: Layout
status: ready
version: 1
created_by: { type: human, id: simon }
updated_by: { type: human, id: simon }
---

# Extra Body Header
This is body content that should be ignored during schema validation.
"#;
    let (fm_str, body) = extract_frontmatter_str(markdown).unwrap();
    assert!(fm_str.contains("id: E12S4"));
    assert!(body.contains("Extra Body Header"));

    let json_val = extract_frontmatter(markdown).unwrap();
    assert_eq!(json_val["id"], "E12S4");
    assert_eq!(json_val["title"], "Layout");
    assert_eq!(json_val["version"], 1);

    // Test missing delimiter
    let no_fm = "Just markdown content without frontmatter";
    assert_eq!(
        extract_frontmatter_str(no_fm).unwrap_err(),
        SchemaError::MissingDelimiters
    );

    // Test unclosed frontmatter
    let unclosed = "---\nid: E12S4\ntitle: test\n";
    assert_eq!(
        extract_frontmatter_str(unclosed).unwrap_err(),
        SchemaError::UnclosedFrontmatter
    );

    // Test non-delimiter leading line like `--- foo\n`
    let non_delimiter = "--- foo\nid: E12S4\ntitle: test\n---\n";
    assert_eq!(
        extract_frontmatter_str(non_delimiter).unwrap_err(),
        SchemaError::MissingDelimiters
    );
}

#[test]
fn test_entity_kind_deferred_work_serde() {
    let serialized = serde_json::to_string(&EntityKind::DeferredWork).unwrap();
    assert_eq!(serialized, "\"dw\"");

    let deserialized_dw: EntityKind = serde_json::from_str("\"dw\"").unwrap();
    assert_eq!(deserialized_dw, EntityKind::DeferredWork);

    let deserialized_alias: EntityKind = serde_json::from_str("\"deferred_work\"").unwrap();
    assert_eq!(deserialized_alias, EntityKind::DeferredWork);
}
