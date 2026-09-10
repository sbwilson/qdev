use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use qdev_core::store::{SqliteStore, Store};
use qdev_core::{
    acquire_write_lock, apply_entity_update, create_story, patch_frontmatter,
    replace_markdown_section, sha256_digest, upsert_cache_and_mark_dirty,
    upsert_cache_with_relation, write_file_atomic, Author, EntityKind, EntityRecord,
    EntityUpdateOptions, ExitCode, FrontmatterPatchOptions, RelationRowChange, StoryCreateOptions,
};
use tempfile::TempDir;

#[test]
fn test_patch_frontmatter_preserves_comments_and_blank_lines_golden() {
    let original = r#"---
# Top frontmatter comment
# Another top comment
id: E12S4
title: "CoreResponse Buffer Layout" # Inline comment on title

# Comment above status
status: draft

# Comment above version
version: 1

owners:
  - simon
  - team:core-platform

# Comment before created_by
created_by:
  type: human
  id: simon

updated_by:
  type: human
  id: simon

# Bottom frontmatter comment
---

## Acceptance Criteria
- Original criteria line 1.
- Original criteria line 2.

## Implementation Notes
- Keep buffer aligned to 64 bytes.
"#;

    let patch_opts = FrontmatterPatchOptions {
        status: Some("in-progress".to_string()),
        title: None,
        custom_fields: Vec::new(),
        author: Some(Author::new("agent", "claude-code")),
        if_version: Some(1),
    };

    let (patched, new_version) = patch_frontmatter(original, &patch_opts).unwrap();
    assert_eq!(new_version, 2);

    let expected = r#"---
# Top frontmatter comment
# Another top comment
id: E12S4
title: "CoreResponse Buffer Layout" # Inline comment on title

# Comment above status
status: in-progress

# Comment above version
version: 2

owners:
  - simon
  - team:core-platform

# Comment before created_by
created_by:
  type: human
  id: simon

updated_by:
  type: agent
  id: claude-code

# Bottom frontmatter comment
---

## Acceptance Criteria
- Original criteria line 1.
- Original criteria line 2.

## Implementation Notes
- Keep buffer aligned to 64 bytes.
"#;

    assert_eq!(
        patched, expected,
        "Patched frontmatter must match golden fixture byte-for-byte"
    );
}

#[test]
fn test_patch_frontmatter_optimistic_concurrency_match() {
    let content = r#"---
id: E12S4
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#;

    let patch_opts = FrontmatterPatchOptions {
        status: Some("ready".to_string()),
        title: None,
        custom_fields: Vec::new(),
        author: Some(Author::new("human", "simon")),
        if_version: Some(1),
    };

    let (patched, new_ver) = patch_frontmatter(content, &patch_opts).unwrap();
    assert_eq!(new_ver, 2);
    assert!(patched.contains("status: ready\n"));
    assert!(patched.contains("version: 2\n"));
}

#[test]
fn test_patch_frontmatter_optimistic_concurrency_mismatch() {
    let content = r#"---
id: E12S4
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#;

    let patch_opts = FrontmatterPatchOptions {
        status: Some("ready".to_string()),
        title: None,
        custom_fields: Vec::new(),
        author: Some(Author::new("human", "simon")),
        if_version: Some(2), // Mismatch! File is version 1
    };

    let err = patch_frontmatter(content, &patch_opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "version_mismatch");
}

/// One function reads `version:` for the whole write path (`patch_frontmatter`'s fence and
/// `apply_relation_change`'s), so the scalar forms it accepts are worth pinning: a quoted value
/// and a trailing comment both name the same version, and a mismatch against either is still
/// exit 5. Before that reader was shared, `apply_relation_change` went through `serde_yaml` and
/// read a quoted version as no version at all.
#[test]
fn test_patch_frontmatter_reads_a_quoted_or_commented_version_for_the_fence() {
    let content = |version: &str| {
        format!(
            r#"---
id: E12S4
status: draft
version: {version}
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#
        )
    };
    let opts = |if_version: u64| FrontmatterPatchOptions {
        status: Some("ready".to_string()),
        title: None,
        custom_fields: Vec::new(),
        author: Some(Author::new("human", "simon")),
        if_version: Some(if_version),
    };

    for declared in ["\"3\"", "'3'", "3  # bumped by the sweep"] {
        let text = content(declared);
        let (patched, new_ver) = patch_frontmatter(&text, &opts(3))
            .unwrap_or_else(|e| panic!("version {declared} must satisfy --if-version 3: {e:?}"));
        assert_eq!(new_ver, 4, "version {declared} must count as 3");
        assert!(patched.contains("version: 4\n"));

        let err = patch_frontmatter(&text, &opts(99)).unwrap_err();
        assert_eq!(err.code(), "version_mismatch", "version {declared}");
        assert_eq!(err.exit_code(), ExitCode::Conflict);
    }
}

#[test]
fn test_patch_frontmatter_if_version_missing_in_file() {
    let content = r#"---
id: E12S4
status: draft
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#;

    let patch_opts = FrontmatterPatchOptions {
        status: Some("ready".to_string()),
        title: None,
        custom_fields: Vec::new(),
        author: Some(Author::new("human", "simon")),
        if_version: Some(1),
    };

    let err = patch_frontmatter(content, &patch_opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "version_mismatch");
}

#[test]
fn test_patch_frontmatter_adds_new_field_before_closing_delimiter() {
    let content = r#"---
id: E12S4
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#;

    let patch_opts = FrontmatterPatchOptions {
        status: None,
        title: Some("New Title".to_string()),
        custom_fields: vec![(
            "appetite".to_string(),
            serde_yaml::Value::String("medium".to_string()),
        )],
        author: Some(Author::new("human", "simon")),
        if_version: None,
    };

    let (patched, new_ver) = patch_frontmatter(content, &patch_opts).unwrap();
    assert_eq!(new_ver, 2);
    assert!(patched.contains("title: New Title\n") || patched.contains("title: \"New Title\"\n"));
    assert!(patched.contains("appetite: medium\n"));
    assert!(patched.contains("version: 2\n"));
}

#[test]
fn test_replace_markdown_section_happy_path() {
    let doc = r#"---
id: E12S4
---

## Acceptance Criteria
- Old AC 1
- Old AC 2

## Implementation Directives
- Follow zero-copy layout.
"#;

    let new_body = "- New AC 1\n- New AC 2\n\n";
    let updated = replace_markdown_section(doc, "Acceptance Criteria", new_body).unwrap();

    let expected = r#"---
id: E12S4
---

## Acceptance Criteria
- New AC 1
- New AC 2

## Implementation Directives
- Follow zero-copy layout.
"#;

    assert_eq!(updated, expected);
}

#[test]
fn test_replace_markdown_section_with_hashes_in_heading_arg() {
    let doc = r#"## Acceptance Criteria
Old text.

## Other Section
Other text.
"#;

    let updated =
        replace_markdown_section(doc, "## Acceptance Criteria", "New text line.\n").unwrap();
    assert_eq!(
        updated,
        "## Acceptance Criteria\nNew text line.\n## Other Section\nOther text.\n"
    );
}

#[test]
fn test_replace_markdown_section_with_child_subsections() {
    let doc = r#"## Acceptance Criteria
Intro text.

### Sub-criteria A
- detail 1

### Sub-criteria B
- detail 2

## Next Section
- next text
"#;

    let updated =
        replace_markdown_section(doc, "Acceptance Criteria", "Replaced entire body.\n").unwrap();
    assert_eq!(
        updated,
        "## Acceptance Criteria\nReplaced entire body.\n## Next Section\n- next text\n"
    );
}

#[test]
fn test_replace_markdown_section_at_eof() {
    let doc = r#"## Intro
Intro text.

## Acceptance Criteria
Old AC text.
"#;

    let updated = replace_markdown_section(doc, "Acceptance Criteria", "New AC text.\n").unwrap();
    assert_eq!(
        updated,
        "## Intro\nIntro text.\n\n## Acceptance Criteria\nNew AC text.\n"
    );
}

#[test]
fn test_replace_markdown_section_not_found() {
    let doc = r#"## Acceptance Criteria
Text.
"#;

    let err = replace_markdown_section(doc, "NonExistent", "New").unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("not found"));
}

#[test]
fn test_replace_markdown_section_multiple_matches() {
    let doc = r#"## Acceptance Criteria
First instance.

## Acceptance Criteria
Second instance.
"#;

    let err = replace_markdown_section(doc, "Acceptance Criteria", "New").unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("Multiple sections matching"));
}

#[test]
fn test_advisory_lock_acquisition_and_release() {
    let tmp = TempDir::new().unwrap();
    let lock_path = tmp.path().join("test.lock");

    {
        let guard1 = acquire_write_lock(&lock_path, Duration::from_millis(500)).unwrap();
        assert_eq!(guard1.path(), lock_path);
        // Guard is held
    }

    // After drop, lock can be acquired again immediately
    let guard2 = acquire_write_lock(&lock_path, Duration::from_millis(500)).unwrap();
    assert_eq!(guard2.path(), lock_path);
}

#[test]
fn test_advisory_lock_timeout_exits_5() {
    let tmp = TempDir::new().unwrap();
    let lock_path = tmp.path().join("contended.lock");

    let lock_acquired_flag = Arc::new(AtomicBool::new(false));
    let lock_acquired_clone = lock_acquired_flag.clone();
    let lock_path_clone = lock_path.clone();

    let handle = thread::spawn(move || {
        let _guard = acquire_write_lock(&lock_path_clone, Duration::from_millis(5000)).unwrap();
        lock_acquired_clone.store(true, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(1000));
    });

    // Wait until thread has acquired the lock
    while !lock_acquired_flag.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }

    // Try acquiring lock with a short timeout (100ms) while thread holds it
    let err = acquire_write_lock(&lock_path, Duration::from_millis(100)).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "lock_timeout");

    handle.join().unwrap();
}

#[test]
fn test_write_file_atomic() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("sub/dir/entity.md");

    write_file_atomic(&target, "hello atomic write\n").unwrap();
    let content = fs::read_to_string(&target).unwrap();
    assert_eq!(content, "hello atomic write\n");

    // Overwrite atomically
    write_file_atomic(&target, "updated content\n").unwrap();
    let content2 = fs::read_to_string(&target).unwrap();
    assert_eq!(content2, "updated content\n");
}

#[test]
fn test_upsert_cache_and_mark_dirty() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");

    // Initialize database and pre-populate sync_state to test invalidation
    {
        let parent = db_path.parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS sync_state (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    synced_at TEXT NOT NULL
);
INSERT INTO sync_state (path, content_hash, synced_at)
VALUES ('docs/specs/stories/E12S4.md', 'oldhash', '2026-09-01T00:00:00Z');
"#,
        )
        .unwrap();
    }

    let record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Buffer Layout".to_string()),
        status: Some("in-progress".to_string()),
        owners: Some("[\"simon\"]".to_string()),
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: sha256_digest(b"test content"),
        version: 2,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("agent", "claude-code")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some("[\"bridge\"]".to_string()),
    };

    upsert_cache_and_mark_dirty(&db_path, &record).unwrap();

    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Verify entities table
    let (kind, version, hash): (String, u64, String) = conn
        .query_row(
            "SELECT kind, version, content_hash FROM entities WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(kind, "story");
    assert_eq!(version, 2);
    assert_eq!(hash, record.content_hash);

    // Verify stories detail table
    let (epic_id, seq, appetite): (String, u32, String) = conn
        .query_row(
            "SELECT epic_id, seq, appetite FROM stories WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(epic_id, "E12");
    assert_eq!(seq, 4);
    assert_eq!(appetite, "small");

    // Verify dirty_entities table
    let dirty_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM dirty_entities WHERE id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dirty_count, 1,
        "dirty_entities must contain updated entity ID"
    );

    // Verify sync_state row was invalidated (deleted)
    let sync_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM sync_state WHERE path = 'docs/specs/stories/E12S4.md';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        sync_count, 0,
        "sync_state row must be invalidated when entity is marked dirty"
    );
}

#[test]
fn test_apply_entity_update_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Setup workspace directory structure
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    let story_path = stories_dir.join("E12S4.md");
    let initial_content = r#"---
id: E12S4
title: Initial Title
status: draft
version: 1
owners: ["simon"]
appetite: small
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Initial criterion.
"#;
    fs::write(&story_path, initial_content).unwrap();

    // Prepare section replacement file
    let ac_file = root.join("ac.md");
    fs::write(&ac_file, "- Updated criterion A.\n- Updated criterion B.\n").unwrap();

    let update_opts = EntityUpdateOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: Some(EntityKind::Story),
        entity_id: "E12S4".to_string(),
        status: Some("in-progress".to_string()),
        title: Some("Updated Title".to_string()),
        custom_fields: Vec::new(),
        section: Some("Acceptance Criteria".to_string()),
        section_file: Some(ac_file),
        if_version: Some(1),
        author: Author::new("agent", "claude-code"),
    };

    let res = apply_entity_update(&update_opts).unwrap();
    assert_eq!(res.old_version, 1);
    assert_eq!(res.new_version, 2);
    assert_eq!(res.id, "E12S4");
    assert_eq!(res.kind, EntityKind::Story);

    let updated_file_content = fs::read_to_string(&story_path).unwrap();
    assert!(updated_file_content.contains("status: in-progress\n"));
    assert!(
        updated_file_content.contains("title: Updated Title\n")
            || updated_file_content.contains("title: \"Updated Title\"\n")
    );
    assert!(updated_file_content.contains("version: 2\n"));
    assert!(updated_file_content.contains("  type: agent\n  id: claude-code\n"));
    assert!(updated_file_content.contains("- Updated criterion A.\n"));

    // Verify cache was updated
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let (cached_status, cached_title): (String, String) = conn
        .query_row(
            "SELECT status, title FROM entities WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(cached_status, "in-progress");
    assert_eq!(cached_title, "Updated Title");

    // Verify stories table was populated
    let (cached_epic, cached_seq, cached_appetite): (String, u32, String) = conn
        .query_row(
            "SELECT epic_id, seq, appetite FROM stories WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(cached_epic, "E12");
    assert_eq!(cached_seq, 4);
    assert_eq!(cached_appetite, "small");

    let dirty_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM dirty_entities WHERE id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dirty_count, 1);
}

#[test]
fn test_patch_frontmatter_rejects_non_whitespace_before_opening_delimiter() {
    let content = "Some non-frontmatter text before\n---\nid: E12S4\nversion: 1\n---\n";
    let patch_opts = FrontmatterPatchOptions {
        status: Some("ready".to_string()),
        ..Default::default()
    };
    let err = patch_frontmatter(content, &patch_opts).unwrap_err();
    assert_eq!(err.code(), "missing_frontmatter");
}

#[test]
fn test_patch_frontmatter_key_block_blank_lines_followed_by_indented_lines() {
    let content = r#"---
id: E12S4
description: |
  first line

  second line
version: 1
---
"#;
    let patch_opts = FrontmatterPatchOptions {
        status: Some("in-progress".to_string()),
        ..Default::default()
    };
    let (patched, new_ver) = patch_frontmatter(content, &patch_opts).unwrap();
    assert_eq!(new_ver, 2);
    assert!(patched.contains("first line\n\n  second line\n"));
    assert!(patched.contains("status: in-progress\n"));
}

#[test]
fn test_patch_frontmatter_single_entry_mapping_custom_field() {
    let content = r#"---
id: E12S4
version: 1
---
"#;
    let mut map = serde_yaml::Mapping::new();
    map.insert(
        serde_yaml::Value::String("sub_key".to_string()),
        serde_yaml::Value::String("sub_val".to_string()),
    );
    let patch_opts = FrontmatterPatchOptions {
        custom_fields: vec![("nested".to_string(), serde_yaml::Value::Mapping(map))],
        ..Default::default()
    };
    let (patched, _) = patch_frontmatter(content, &patch_opts).unwrap();
    assert!(patched.contains("nested:\n  sub_key: sub_val\n"));
}

#[test]
fn test_patch_frontmatter_rejects_managed_fields() {
    let content = r#"---
id: E12S4
version: 1
---
"#;
    for managed_key in &["id", "version", "updated_by", "created_by"] {
        let patch_opts = FrontmatterPatchOptions {
            custom_fields: vec![(
                managed_key.to_string(),
                serde_yaml::Value::String("override".to_string()),
            )],
            ..Default::default()
        };
        let err = patch_frontmatter(content, &patch_opts).unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::UsageError);
    }
}

#[test]
fn test_replace_markdown_section_ignores_headings_in_fenced_code_blocks() {
    let doc = r#"---
id: E12S4
---

## Acceptance Criteria
- AC 1
```rust
## Fake Heading in backticks
fn test() {}
```
- AC 2

## Other Section
Some text
"#;
    let new_body = "- Only new AC\n\n";
    let res = replace_markdown_section(doc, "Acceptance Criteria", new_body).unwrap();
    assert!(res.contains("## Acceptance Criteria\n- Only new AC\n\n## Other Section"));
    assert!(!res.contains("Fake Heading"));
    assert!(!res.contains("AC 2"));
}

#[test]
fn test_replace_markdown_section_ignores_indented_headings() {
    let doc = r#"---
id: E12S4
---

## Acceptance Criteria
- AC 1
    # Indented code block comment with 4 spaces
- AC 2

## Other Section
Text
"#;
    let new_body = "- AC 3\n\n";
    let res = replace_markdown_section(doc, "Acceptance Criteria", new_body).unwrap();
    assert!(res.contains("## Acceptance Criteria\n- AC 3\n\n## Other Section"));
    assert!(!res.contains("Indented code block comment"));
    assert!(!res.contains("AC 2"));
}

#[test]
fn test_replace_markdown_section_heading_without_trailing_newline() {
    let doc = "---
id: E12S4
---

## Acceptance Criteria"; // No trailing newline at EOF
    let res = replace_markdown_section(doc, "Acceptance Criteria", "- New criterion\n").unwrap();
    assert_eq!(
        res,
        "---
id: E12S4
---

## Acceptance Criteria
- New criterion\n"
    );
}

#[test]
fn test_resolve_entity_file_multiple_matches_errors() {
    let tmp = TempDir::new().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E12S4-login.md"), "---").unwrap();
    fs::write(stories_dir.join("E12S4-logout.md"), "---").unwrap();

    let err = qdev_core::resolve_entity_file(tmp.path(), Some(EntityKind::Story), "E12S4", None)
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("Multiple entity files match"));
}

#[test]
fn test_upsert_cache_preserves_created_by_coalesce() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");
    let parent = db_path.parent().unwrap();
    fs::create_dir_all(parent).unwrap();

    let initial_record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Initial Title".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: sha256_digest(b"initial"),
        version: 1,
        created_by: Some(Author::new("human", "alice")),
        updated_by: Some(Author::new("human", "alice")),
        updated_at: "2026-09-01T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    upsert_cache_and_mark_dirty(&db_path, &initial_record).unwrap();

    // Second update where created_by is None
    let update_record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Updated Title".to_string()),
        status: Some("ready".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: sha256_digest(b"updated"),
        version: 2,
        created_by: None, // No created_by in updated frontmatter
        updated_by: Some(Author::new("agent", "bob")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    upsert_cache_and_mark_dirty(&db_path, &update_record).unwrap();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (c_type, c_id, u_type, u_id): (String, String, String, String) = conn
        .query_row(
            "SELECT created_by_type, created_by_id, updated_by_type, updated_by_id FROM entities WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        c_type, "human",
        "COALESCE must preserve existing created_by_type"
    );
    assert_eq!(
        c_id, "alice",
        "COALESCE must preserve existing created_by_id"
    );
    assert_eq!(u_type, "agent", "updated_by_type must be updated");
    assert_eq!(u_id, "bob", "updated_by_id must be updated");
}

#[test]
fn test_resolve_entity_file_path_traversal_rejected() {
    let tmp = TempDir::new().unwrap();
    let err = qdev_core::resolve_entity_file(tmp.path(), None, "../etc/passwd", None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("cannot contain path separators"));
}

#[test]
fn test_resolve_entity_file_case_insensitive() {
    let tmp = TempDir::new().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E12S4-login.md"), "---").unwrap();

    // Query using lowercase e12s4
    let (kind, id, path) = qdev_core::resolve_entity_file(tmp.path(), None, "e12s4", None).unwrap();
    assert_eq!(kind, EntityKind::Story);
    assert_eq!(id, "e12s4");
    assert!(path.ends_with("E12S4-login.md"));
}

#[test]
fn test_upsert_cache_clears_stale_flag() {
    // An entity flagged stale by a merge conflict or schema violation must lose the flag once
    // the write path repairs it; otherwise staleness is permanently sticky.
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Buffer Layout".to_string()),
        status: Some("in-progress".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: sha256_digest(b"test content"),
        version: 2,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("agent", "claude-code")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };

    upsert_cache_and_mark_dirty(&db_path, &record).unwrap();

    // Flag it stale the way the sweep does for an unparsable file.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute("UPDATE entities SET stale = 1 WHERE id = 'E12S4';", [])
            .unwrap();
        let stale: i64 = conn
            .query_row("SELECT stale FROM entities WHERE id = 'E12S4';", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stale, 1, "precondition: the row is flagged stale");
    }

    let repaired = EntityRecord {
        version: 3,
        content_hash: sha256_digest(b"repaired content"),
        ..record
    };
    upsert_cache_and_mark_dirty(&db_path, &repaired).unwrap();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let stale: i64 = conn
        .query_row("SELECT stale FROM entities WHERE id = 'E12S4';", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stale, 0, "a successful write must clear the stale flag");
}

/// A relation change lands in the `relations` table inside the same transaction as the entity
/// upsert, not at the next process's boot sweep. Every in-process reader of the graph — the
/// `qdev relate` cycle pre-check, `query_entity`, the graph renderer — otherwise sees pre-write
/// state, so two relation operations in one run validate the second against a graph that
/// ignores the first.
#[test]
fn test_upsert_cache_with_relation_writes_the_relation_row() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let record = EntityRecord {
        id: "E1S1".to_string(),
        kind: EntityKind::Story,
        title: Some("Source".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E1S1.md".to_string(),
        content_hash: sha256_digest(b"content"),
        version: 2,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E1".to_string()),
        seq: Some(1),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };

    let add = RelationRowChange {
        source_id: "E1S1".to_string(),
        relation: "depends_on".to_string(),
        target_id: "E1S2".to_string(),
        add: true,
    };
    upsert_cache_with_relation(&db_path, &record, Some(&add)).unwrap();

    let store = SqliteStore::open(&db_path).unwrap();
    let rows = store.get_relations_for_source("E1S1").unwrap();
    assert_eq!(rows.len(), 1, "the edge must be visible without a sweep");
    assert_eq!(rows[0].relation, "depends_on");
    assert_eq!(rows[0].target_id, "E1S2");
    drop(store);

    // Removing it is applied the same way.
    let remove = RelationRowChange {
        add: false,
        ..add.clone()
    };
    upsert_cache_with_relation(&db_path, &record, Some(&remove)).unwrap();

    let store = SqliteStore::open(&db_path).unwrap();
    assert!(
        store.get_relations_for_source("E1S1").unwrap().is_empty(),
        "the edge must be gone without a sweep"
    );
}

/// The document `create_story` generates is a byte-for-byte contract: field order, flow-style
/// sequences, an always-quoted title, and block-mapped attribution. Pinned here in core so the
/// shape survives independently of the CLI's own assertions.
#[test]
fn test_create_story_generated_document_golden() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let opts = StoryCreateOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        story_id: "E12S1".to_string(),
        title: Some("Feature \"quoted\": [special]".to_string()),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: vec!["bridge".to_string(), "foundation".to_string()],
        owners: vec!["simon".to_string(), "team:core-platform".to_string()],
        author: Author::new("agent", "claude-code"),
    };

    let result = create_story(&opts).unwrap();
    assert_eq!(result.id, "E12S1");
    assert_eq!(result.rel_path, "docs/specs/stories/E12S1.md");
    assert_eq!(result.path, root.join("docs/specs/stories/E12S1.md"));

    let expected = r#"---
id: E12S1
title: "Feature \"quoted\": [special]"
status: draft
appetite: small
safety_class: ClassB
target_modules: ["bridge","foundation"]
owners: ["simon","team:core-platform"]
version: 1
created_by:
  type: agent
  id: claude-code
updated_by:
  type: agent
  id: claude-code
---

## Acceptance Criteria
"#;
    assert_eq!(fs::read_to_string(&result.path).unwrap(), expected);

    // The title must survive a round trip through the YAML parser, not merely look escaped.
    let frontmatter = qdev_core::extract_frontmatter(expected).unwrap();
    assert_eq!(frontmatter["title"], "Feature \"quoted\": [special]");
}

/// `write_file_atomic` renames over its destination, which *succeeds* on an existing path — so
/// the exclusivity `OpenOptions::create_new` used to provide is re-established explicitly,
/// inside the lock. Without it a refused create would silently clobber an existing story.
#[test]
fn test_create_story_refuses_to_clobber_an_occupied_path() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E12S1.md"), "PRECIOUS\n").unwrap();

    let opts = StoryCreateOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        story_id: "E12S1".to_string(),
        title: Some("Clobber".to_string()),
        appetite: None,
        safety_class: None,
        target_modules: Vec::new(),
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = create_story(&opts).expect_err("an occupied path must be a conflict");
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "file_exists");
    assert_eq!(
        fs::read_to_string(stories_dir.join("E12S1.md")).unwrap(),
        "PRECIOUS\n",
        "the existing file must be untouched"
    );
}

/// Schema validation runs before the lock is taken and before anything is written, so an
/// invalid field leaves no file at all — not a rejected-but-present one.
#[test]
fn test_create_story_schema_invalid_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let opts = StoryCreateOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        story_id: "E12S1".to_string(),
        title: None,
        appetite: Some("huge".to_string()),
        safety_class: None,
        target_modules: Vec::new(),
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = create_story(&opts).expect_err("an out-of-enum appetite must be refused");
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "schema_validation_failed");
    assert!(
        err.message().contains("appetite"),
        "the error must name the offending field, got: {}",
        err.message()
    );
    assert!(!root.join("docs/specs/stories/E12S1.md").exists());
}

/// `create_story` must take the advisory lock *itself*. The CLI-level lock test cannot prove
/// this: in an initialized workspace every command boots through `ensure_cache`, whose sweep
/// takes the same lock, so the command fails at boot and exits 5 whether or not `create_story`
/// ever locks anything. Deleting the lock from `create_story` left the whole suite green.
/// Calling core directly bypasses boot, so the contention observed here is the one under test.
#[test]
fn test_create_story_waits_on_the_advisory_lock_then_succeeds_once_released() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // The lock is only taken when the cache directory exists — that is what marks an
    // initialized workspace for `create_story`.
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let lock_path = cache_dir.join("write.lock");

    let opts = StoryCreateOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        story_id: "E12S1".to_string(),
        title: Some("Contended".to_string()),
        appetite: None,
        safety_class: None,
        target_modules: Vec::new(),
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let release = Arc::new(AtomicBool::new(false));
    let holder_release = Arc::clone(&release);
    let holder_path = lock_path.clone();
    let holder = thread::spawn(move || {
        let _guard = acquire_write_lock(&holder_path, Duration::from_millis(1000))
            .expect("the holder must get the lock first");
        // Held past the 5 s timeout the writer allows, then released unconditionally so an
        // assertion failure in the main thread cannot leave this thread holding it.
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while !holder_release.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
    });

    thread::sleep(Duration::from_millis(200));

    let err = create_story(&opts).expect_err("a held write lock must refuse the create");
    assert_eq!(err.code(), "lock_timeout");
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "nothing may be written while another writer holds the lock"
    );

    release.store(true, Ordering::SeqCst);
    holder.join().unwrap();

    // Recovery: the same create succeeds once the lock is free, and the id was not consumed.
    let res = create_story(&opts).expect("the create must succeed once the lock is released");
    assert_eq!(res.id, "E12S1");
    assert!(root.join("docs/specs/stories/E12S1.md").is_file());
}

// ---------------------------------------------------------------------------
// The one identity rule: a file is named for the entity it holds
// ---------------------------------------------------------------------------

/// `--fix-ids` renames as it renumbers, so the renamed file still carries its id and stays
/// resolvable by the write path. A `-slug`/`_slug` suffix is part of the convention and survives.
#[test]
fn test_renamed_file_name_replaces_the_id_and_keeps_the_slug() {
    assert_eq!(
        qdev_core::renamed_file_name("E1S1.md", "E1S1", "E1S2"),
        "E1S2.md"
    );
    assert_eq!(
        qdev_core::renamed_file_name("E1S1-buffer-layout.md", "E1S1", "E1S2"),
        "E1S2-buffer-layout.md"
    );
    assert_eq!(
        qdev_core::renamed_file_name("E1S1_buffer.md", "E1S1", "E1S2"),
        "E1S2_buffer.md"
    );
    // The id part is rewritten canonically cased even from a differently cased file name.
    assert_eq!(
        qdev_core::renamed_file_name("e1s1-dup.md", "E1S1", "E1S2"),
        "E1S2-dup.md"
    );
    // A name that never carried the old id has no identifiable suffix: it becomes canonical.
    assert_eq!(
        qdev_core::renamed_file_name("login-flow.md", "E1S1", "E1S2"),
        "E1S2.md"
    );
}

/// A renamed file is exactly what `resolve_entity_file` then finds — the round trip that makes a
/// renumbered entity writable with no manual `mv`.
#[test]
fn test_renamed_file_is_resolvable_under_the_new_id() {
    let tmp = TempDir::new().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let new_name = qdev_core::renamed_file_name("E1S1-dup.md", "E1S1", "E1S2");
    fs::write(stories_dir.join(&new_name), "---").unwrap();

    let (kind, _, path) = qdev_core::resolve_entity_file(tmp.path(), None, "E1S2", None).unwrap();
    assert_eq!(kind, EntityKind::Story);
    assert!(path.ends_with("E1S2-dup.md"));
    // And the old id no longer resolves to it.
    assert!(qdev_core::resolve_entity_file(tmp.path(), None, "E1S1", None).is_err());
}

/// Kind is hydration's answer, not the directory's: frontmatter `kind:` wins, so a write
/// validates against the schema the next sweep will validate against.
#[test]
fn test_kind_for_write_prefers_frontmatter_kind_over_the_directory() {
    let adr_path = std::path::Path::new("docs/specs/adrs/AD-9.md");
    let declares_story = "---\nid: AD-9\nkind: story\n---\n";
    assert_eq!(
        qdev_core::kind_for_write(adr_path, declares_story, EntityKind::Adr),
        EntityKind::Story
    );

    // With no `kind:`, the directory still decides, so conventional files are unaffected.
    let no_kind = "---\nid: AD-9\n---\n";
    assert_eq!(
        qdev_core::kind_for_write(adr_path, no_kind, EntityKind::Adr),
        EntityKind::Adr
    );

    // Content with no extractable frontmatter falls back to the resolved kind; the caller
    // reports the parse failure on its own terms.
    assert_eq!(
        qdev_core::kind_for_write(adr_path, "no frontmatter here", EntityKind::Adr),
        EntityKind::Adr
    );
}

/// Two `entities` rows must never point at one file. `ON CONFLICT(id)` cannot see that
/// collision, so the upsert purges the row claiming the same `source_path` under another id, as
/// hydration does — but without hydration's relation cascade, since `--fix-ids` redirects the
/// edges through the relate write path afterwards.
#[test]
fn test_upsert_cache_purges_a_row_claiming_the_same_path_under_another_id() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let mut record = EntityRecord {
        id: "E1S1".to_string(),
        kind: EntityKind::Story,
        title: Some("Story".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E1S1.md".to_string(),
        content_hash: sha256_digest(b"one"),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E1".to_string()),
        seq: Some(1),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    upsert_cache_and_mark_dirty(&db_path, &record).unwrap();

    let store = SqliteStore::open(&db_path).unwrap();
    store
        .upsert_relation(&qdev_core::store::RelationRecord {
            source_id: "E1S5".to_string(),
            relation: "depends_on".to_string(),
            target_id: "E1S1".to_string(),
        })
        .unwrap();

    // The same file now declares a different id (an in-place id edit).
    record.id = "E1S2".to_string();
    record.seq = Some(2);
    record.content_hash = sha256_digest(b"two");
    upsert_cache_and_mark_dirty(&db_path, &record).unwrap();

    assert!(store.get_entity("E1S1").unwrap().is_none());
    assert!(store.get_story_details("E1S1").unwrap().is_none());
    assert_eq!(
        store.get_entity("E1S2").unwrap().unwrap().source_path,
        "docs/specs/stories/E1S1.md"
    );
    // The inbound edge survives: it is redirected by a later write, not deleted here.
    assert_eq!(store.list_relations().unwrap().len(), 1);
}

/// The renamed-away row is dropped only when it still claims the path that moved. In the
/// duplicate case the old id belongs to the keeper file, which is on disk and untouched.
#[test]
fn test_purge_entity_row_for_moved_file_respects_the_source_path_guard() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join(".qdev/cache/cache.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();

    let record = EntityRecord {
        id: "E1S1".to_string(),
        kind: EntityKind::Story,
        title: Some("Story".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E1S1-dup.md".to_string(),
        content_hash: sha256_digest(b"keeper"),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E1".to_string()),
        seq: Some(1),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    upsert_cache_and_mark_dirty(&db_path, &record).unwrap();
    let store = SqliteStore::open(&db_path).unwrap();

    // The row belongs to the keeper file, not to the file that was renamed away: left alone.
    let purged =
        qdev_core::purge_entity_row_for_moved_file(&db_path, "E1S1", "docs/specs/stories/E1S1.md")
            .unwrap();
    assert!(!purged);
    assert!(store.get_entity("E1S1").unwrap().is_some());

    // When it does claim the moved path, it goes.
    let purged =
        qdev_core::purge_entity_row_for_moved_file(&db_path, "E1S1", &record.source_path).unwrap();
    assert!(purged);
    assert!(store.get_entity("E1S1").unwrap().is_none());
    assert!(store.get_story_details("E1S1").unwrap().is_none());
}

/// The fence must accept every YAML integer form hydration accepts. A decimal-only scan made
/// `version: 0x03` — schema-valid, reported as `3` by `qdev get` — read as "no version", so
/// `--if-version 3` refused the version the entity declared *and* an unfenced write reset the
/// counter to 1, rolling it backwards.
#[test]
fn test_frontmatter_version_accepts_every_yaml_integer_form() {
    for (literal, expected) in [
        ("3", 3u64),
        ("\"3\"", 3),
        ("'3'", 3),
        ("3  # bumped by the sweep", 3),
        ("0x03", 3),
        ("0o3", 3),
        ("+3", 3),
    ] {
        let content = format!(
            "---\nid: E1S1\ntitle: One\nstatus: draft\nversion: {literal}\ncreated_by:\n  \
             type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\n## AC\n- one\n"
        );

        // The fence is satisfied by the declared value...
        let opts = FrontmatterPatchOptions {
            status: Some("ready".to_string()),
            title: None,
            custom_fields: Vec::new(),
            author: Some(Author::new("human", "simon")),
            if_version: Some(expected),
        };
        let (patched, new_version) = patch_frontmatter(&content, &opts).unwrap_or_else(|e| {
            panic!("version literal {literal:?} must satisfy the fence: {e:?}")
        });

        // ...and the counter moves forward from it, never back to 1.
        assert_eq!(
            new_version,
            expected + 1,
            "version literal {literal:?} must bump from {expected}"
        );
        assert!(
            patched.contains(&format!("version: {}", expected + 1)),
            "version literal {literal:?} produced: {patched}"
        );
    }
}
