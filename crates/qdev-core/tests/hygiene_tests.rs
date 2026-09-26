use std::path::Path;
use qdev_core::config::{Config, HygieneConfig};
use qdev_core::hygiene::{
    check_hygiene, lint_comments, tokenize_comments, CommentKind, HygieneCheckOptions,
    SupportedLanguage, RULE_FORBID_PATTERNS, RULE_MAX_INLINE_COMMENT_LINES, RULE_REVIEW_ROUND,
    RULE_STORY_BANNER,
};

#[test]
fn test_rust_tokenizer_line_and_doc_comments() {
    let source = r#"
// regular line comment
/// outer doc comment
//! inner doc comment
//// four slashes is regular line comment
fn main() {}
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 4);

    assert_eq!(tokens[0].kind, CommentKind::Line);
    assert_eq!(tokens[0].start_line, 2);
    assert_eq!(tokens[0].lines[0].text, "// regular line comment");

    assert_eq!(tokens[1].kind, CommentKind::DocLine);
    assert_eq!(tokens[1].start_line, 3);
    assert_eq!(tokens[1].lines[0].text, "/// outer doc comment");

    assert_eq!(tokens[2].kind, CommentKind::DocLine);
    assert_eq!(tokens[2].start_line, 4);
    assert_eq!(tokens[2].lines[0].text, "//! inner doc comment");

    assert_eq!(tokens[3].kind, CommentKind::Line);
    assert_eq!(tokens[3].start_line, 5);
}

#[test]
fn test_rust_tokenizer_nested_block_comments() {
    let source = r#"
/* outer comment
   /* inner comment */
   still outer comment */
fn main() {}
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, CommentKind::Block);
    assert_eq!(tokens[0].start_line, 2);
    assert_eq!(tokens[0].end_line, 4);
    assert_eq!(tokens[0].lines.len(), 3);
}

#[test]
fn test_rust_tokenizer_doc_block_comments() {
    let source = r#"
/** doc block comment */
/*! inner doc block comment */
/* regular block comment */
/**/
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0].kind, CommentKind::DocBlock);
    assert_eq!(tokens[1].kind, CommentKind::DocBlock);
    assert_eq!(tokens[2].kind, CommentKind::Block);
    assert_eq!(tokens[3].kind, CommentKind::Block);
}

#[test]
fn test_rust_tokenizer_string_literal_shielding() {
    let source = r###"
fn main() {
    let s = "Hello // not a comment /* neither */";
    let esc = "He said: \"// still not comment\"";
    let raw = r#"Raw // not a comment /* neither */"#;
    let raw_hashes = r##"Multiline raw
// not a comment
/* neither */"##;
    let c = '/';
    let star = '*';
}
"###;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 0, "No comments should be extracted from strings or char literals");
}

#[test]
fn test_rust_tokenizer_lifetimes() {
    let source = r#"
fn foo<'a>(x: &'a str) -> &'a str {
    // legitimate comment
    x
}
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].lines[0].text, "// legitimate comment");
}

#[test]
fn test_swift_tokenizer_comments_and_strings() {
    let source = r###"
// Swift line comment
/// Swift doc line
/* Swift block /* nested */ comment */
/** Swift doc block */
let s = "Hello // not a comment"
let multiline = """
// not a comment inside multiline
"""
let raw = #"Raw // not a comment"#
"###;
    let tokens = tokenize_comments(source, SupportedLanguage::Swift);
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0].kind, CommentKind::Line);
    assert_eq!(tokens[1].kind, CommentKind::DocLine);
    assert_eq!(tokens[2].kind, CommentKind::Block);
    assert_eq!(tokens[3].kind, CommentKind::DocBlock);
}

#[test]
fn test_python_tokenizer_comments_and_docstrings() {
    let source = r###"
# Python line comment
"""Triple double quote docstring
line 2
"""
'''Triple single quote docstring
line 2
'''
s1 = "Hello # not comment"
s2 = 'World # not comment'
"###;
    let tokens = tokenize_comments(source, SupportedLanguage::Python);
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].kind, CommentKind::Line);
    assert_eq!(tokens[1].kind, CommentKind::DocBlock);
    assert_eq!(tokens[2].kind, CommentKind::DocBlock);
}

#[test]
fn test_linter_story_banner_detection() {
    let config = HygieneConfig::default();
    let source = r#"
// ⭐ **STORY 2.10 — FIRST import**
// STORY 123: Initial implementation
// STORY E12S4: Buffer layout
// ⭐ STORY
// A comment telling a story about performance
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    let findings = lint_comments("src/main.rs", &tokens, &config).unwrap();

    assert_eq!(findings.len(), 4);
    for finding in &findings {
        assert_eq!(finding.rule_id, RULE_STORY_BANNER);
    }
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[1].line, 3);
    assert_eq!(findings[2].line, 4);
    assert_eq!(findings[3].line, 5);
}

#[test]
fn test_linter_review_round_detection() {
    let config = HygieneConfig::default();
    let source = r#"
// Review round 2 findings: fixed bug
// Wave 1: initial scaffolding
// Review pass 3: verification
// Normal comment discussing review process
// Low-pass 1 filter and render pass 1 should not trigger
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    let findings = lint_comments("src/main.rs", &tokens, &config).unwrap();

    assert_eq!(findings.len(), 3);
    for finding in &findings {
        assert_eq!(finding.rule_id, RULE_REVIEW_ROUND);
    }
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[1].line, 3);
    assert_eq!(findings[2].line, 4);
}

#[test]
fn test_linter_forbid_patterns() {
    let config = HygieneConfig {
        forbid_patterns: vec!["TEMPORARY_HACK".to_string(), "NO_COMMIT".to_string()],
        ..Default::default()
    };

    let source = r#"
// Line 1: normal
// Line 2: TEMPORARY_HACK: needs fix
// Line 3: NO_COMMIT before release
// Line 4: clean
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    let findings = lint_comments("src/lib.rs", &tokens, &config).unwrap();

    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].rule_id, RULE_FORBID_PATTERNS);
    assert_eq!(findings[0].line, 3);
    assert_eq!(findings[1].rule_id, RULE_FORBID_PATTERNS);
    assert_eq!(findings[1].line, 4);
}

#[test]
fn test_linter_max_inline_comment_lines() {
    let config = HygieneConfig {
        max_inline_comment_lines: 6,
        ..Default::default()
    };

    // 6 lines -> clean
    let clean_source = r#"
// line 1
// line 2
// line 3
// line 4
// line 5
// line 6
"#;
    let clean_tokens = tokenize_comments(clean_source, SupportedLanguage::Rust);
    let clean_findings = lint_comments("src/clean.rs", &clean_tokens, &config).unwrap();
    assert_eq!(clean_findings.len(), 0);

    // 7 lines -> violation at block start line
    let violation_source = r#"
// line 1
// line 2
// line 3
// line 4
// line 5
// line 6
// line 7
"#;
    let violation_tokens = tokenize_comments(violation_source, SupportedLanguage::Rust);
    let violation_findings = lint_comments("src/violation.rs", &violation_tokens, &config).unwrap();
    assert_eq!(violation_findings.len(), 1);
    assert_eq!(violation_findings[0].rule_id, RULE_MAX_INLINE_COMMENT_LINES);
    assert_eq!(violation_findings[0].line, 2);
    assert_eq!(violation_findings[0].excerpt, "// line 1");
}

#[test]
fn test_linter_doc_comments_never_flagged_under_max_lines() {
    let config = HygieneConfig {
        max_inline_comment_lines: 6,
        ..Default::default()
    };

    let doc_source = r#"
/// doc line 1
/// doc line 2
/// doc line 3
/// doc line 4
/// doc line 5
/// doc line 6
/// doc line 7
/// doc line 8
fn documented() {}
"#;
    let tokens = tokenize_comments(doc_source, SupportedLanguage::Rust);
    let findings = lint_comments("src/doc.rs", &tokens, &config).unwrap();
    assert_eq!(findings.len(), 0, "Doc comments must never be flagged under max_inline_comment_lines");
}

#[test]
fn test_linter_citation_exemption() {
    let config = HygieneConfig {
        max_inline_comment_lines: 6,
        ..Default::default()
    };

    // Comments with compact citations are exempt from story banner and forbid checks
    let source = r#"
// [E12S10] Rust-driven mount (see AD-43)
// [DEC-2b91] Extended attributes table
// [HAZ-14] Safety PIP latch
// [DW-7f3a] Temporary zero-copy bypass
// [E12S4/NG-2] Frame buffers untouched
// [E12S4] STORY 2.10 implementation
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    let findings = lint_comments("src/citations.rs", &tokens, &config).unwrap();
    assert_eq!(findings.len(), 0, "Compact citations must be completely exempt");

    // Lines with citations do not count toward max_inline_comment_lines
    let block_with_citation = r#"
// line 1
// line 2
// line 3
// [E12S4] line with citation does not count
// line 4
// line 5
// line 6
"#;
    let tokens2 = tokenize_comments(block_with_citation, SupportedLanguage::Rust);
    let findings2 = lint_comments("src/block.rs", &tokens2, &config).unwrap();
    assert_eq!(findings2.len(), 0, "Block with 6 regular lines and 1 citation line must pass");

    // 7 regular lines + 1 citation line -> 7 non-citation lines > 6 -> violation
    let block_over_limit = r#"
// line 1
// line 2
// line 3
// line 4
// [E12S4] line with citation does not count
// line 5
// line 6
// line 7
"#;
    let tokens3 = tokenize_comments(block_over_limit, SupportedLanguage::Rust);
    let findings3 = lint_comments("src/block.rs", &tokens3, &config).unwrap();
    assert_eq!(findings3.len(), 1);
    assert_eq!(findings3[0].rule_id, RULE_MAX_INLINE_COMMENT_LINES);
}

#[test]
fn test_check_hygiene_disabled() {
    let config = Config {
        hygiene: HygieneConfig {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let options = HygieneCheckOptions::default();
    let outcome = check_hygiene(Path::new("."), &config, &options).unwrap();
    assert_eq!(outcome.status, "pass");
    assert_eq!(outcome.findings.len(), 0);
}

#[test]
fn test_rust_tokenizer_escaped_quote_char() {
    let source = "let c = '\\''; // comment after escaped quote\nlet x = 1;\n";
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].lines[0].text, "// comment after escaped quote");
    assert_eq!(tokens[0].start_line, 1);
}

#[test]
fn test_swift_tokenizer_interpolation_with_quotes() {
    let source = "let msg = \"Hello \\(name == \"world\" ? 1 : 2)\"; // comment after interpolation\n";
    let tokens = tokenize_comments(source, SupportedLanguage::Swift);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].lines[0].text, "// comment after interpolation");
}

#[test]
fn test_trailing_comments_not_grouped_into_max_inline_lines() {
    let config = HygieneConfig {
        max_inline_comment_lines: 6,
        ..Default::default()
    };
    let source = r#"
let a = 1; // comment 1
let b = 2; // comment 2
let c = 3; // comment 3
let d = 4; // comment 4
let e = 5; // comment 5
let f = 6; // comment 6
let g = 7; // comment 7
"#;
    let tokens = tokenize_comments(source, SupportedLanguage::Rust);
    let findings = lint_comments("src/statements.rs", &tokens, &config).unwrap();
    assert_eq!(findings.len(), 0, "Trailing statement comments must not be grouped into inline block");
}

#[test]
fn test_check_hygiene_bare_workspace_traversal_multi_language() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();

    let rust_file = root.join("src/lib.rs");
    std::fs::create_dir_all(rust_file.parent().unwrap()).unwrap();
    std::fs::write(&rust_file, "// Clean rust code\npub fn hello() {}\n").unwrap();

    let swift_file = root.join("Sources/App.swift");
    std::fs::create_dir_all(swift_file.parent().unwrap()).unwrap();
    std::fs::write(&swift_file, "// ⭐ EPIC 3 - core setup\nfunc run() {}\n").unwrap();

    let py_file = root.join("scripts/build.py");
    std::fs::create_dir_all(py_file.parent().unwrap()).unwrap();
    std::fs::write(&py_file, "# Review round 1 findings: fix script\nprint('ok')\n").unwrap();

    let config = Config {
        hygiene: HygieneConfig {
            enabled: true,
            languages: vec!["rs".to_string(), "swift".to_string(), "py".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };

    let outcome = check_hygiene(root, &config, &HygieneCheckOptions::default()).unwrap();
    assert_eq!(outcome.findings.len(), 2);
    assert!(outcome.findings.iter().any(|f| f.file.ends_with("App.swift") && f.rule_id == RULE_STORY_BANNER));
    assert!(outcome.findings.iter().any(|f| f.file.ends_with("build.py") && f.rule_id == RULE_REVIEW_ROUND));
}
