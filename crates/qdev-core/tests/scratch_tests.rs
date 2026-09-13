#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;

use qdev_core::scratch::{
    append_scratch_entry, estimate_tokens, filter_scratch_entries_by_budget, read_scratch_entries,
    summarize_scratch_entries, ScratchpadAuthor, ScratchpadEntry,
};
use qdev_core::store::{SqliteStore, Store};
use qdev_core::{ensure_cache, Author, StorageConfig};
use tempfile::TempDir;

fn setup_test_workspace(root: &Path) {
    let qdev_toml = root.join("qdev.toml");
    fs::write(
        qdev_toml,
        r#"
[project]
name = "TestProject"

[identity]
developer_id = "simon"
teams = ["core-platform"]

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"
"#,
    )
    .unwrap();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let scratch_dir = root.join("docs/state/scratch");
    fs::create_dir_all(&scratch_dir).unwrap();

    ensure_cache(root, &StorageConfig::default()).unwrap();
}

fn write_story_file(root: &Path, id: &str, title: &str) -> String {
    let story_content = format!(
        r#"---
id: {id}
title: "{title}"
status: ready
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Spec content for story {id}.
"#
    );
    let path = root.join("docs/specs/stories").join(format!("{}.md", id));
    fs::write(&path, &story_content).unwrap();
    story_content
}

#[test]
fn test_scratch_append_sequencing_and_atomic_file_write() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let author = Author::new("human", "simon");

    // 1. Append first entry with explicit kind
    let entry1 = append_scratch_entry(
        root,
        None,
        "E12S4",
        Some("tradeoff"),
        "AtomicBool over Mutex on frame drop latch",
        &author,
        None,
    )
    .unwrap();

    assert_eq!(entry1.seq, 1);
    assert_eq!(entry1.kind, "tradeoff");
    assert_eq!(entry1.text, "AtomicBool over Mutex on frame drop latch");
    assert_eq!(entry1.author.r#type, "human");
    assert_eq!(entry1.author.id, "simon");

    let scratch_file = root.join("docs/state/scratch/E12S4.jsonl");
    assert!(scratch_file.is_file());

    let lines: Vec<String> = fs::read_to_string(&scratch_file)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    assert_eq!(lines.len(), 1);

    // 2. Append second entry with default kind (None -> "note")
    let entry2 = append_scratch_entry(
        root,
        None,
        "E12S4",
        None,
        "Checked frame latch",
        &author,
        None,
    )
    .unwrap();

    assert_eq!(entry2.seq, 2);
    assert_eq!(entry2.kind, "note");
    assert_eq!(entry2.text, "Checked frame latch");

    let lines: Vec<String> = fs::read_to_string(&scratch_file)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    assert_eq!(lines.len(), 2);

    // 3. Append third entry with kind "decision"
    let entry3 = append_scratch_entry(
        root,
        None,
        "E12S4",
        Some("decision"),
        "Adopted AtomicBool",
        &author,
        None,
    )
    .unwrap();

    assert_eq!(entry3.seq, 3);
    assert_eq!(entry3.kind, "decision");
    assert_eq!(entry3.text, "Adopted AtomicBool");

    let lines: Vec<String> = fs::read_to_string(&scratch_file)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();
    assert_eq!(lines.len(), 3);
}

#[test]
fn test_scratch_append_syncs_sqlite_cache_and_sync_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let author = Author::new("agent", "claude-code");

    let entry = append_scratch_entry(
        root,
        None,
        "E12S4",
        Some("transition"),
        "Transitioned to review",
        &author,
        None,
    )
    .unwrap();

    assert_eq!(entry.seq, 1);

    // Inspect SQLite cache
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();

    let rows = store.get_scratchpad_entries("E12S4").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].story_id, "E12S4");
    assert_eq!(rows[0].seq, 1);
    assert_eq!(rows[0].kind.as_deref(), Some("transition"));
    assert_eq!(rows[0].text.as_deref(), Some("Transitioned to review"));
    assert_eq!(rows[0].author_type.as_deref(), Some("agent"));
    assert_eq!(rows[0].author_id.as_deref(), Some("claude-code"));

    // Inspect sync_state
    let sync_state = store
        .get_sync_state("docs/state/scratch/E12S4.jsonl")
        .unwrap();
    assert!(sync_state.is_some());
    let ss = sync_state.unwrap();
    assert!(ss.mtime > 0);
    assert!(ss.size > 0);
    assert!(ss.content_hash.is_some());
}

#[test]
fn test_story_specification_markdown_immutability() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let original_content = write_story_file(root, "E12S4", "Story 4");
    let story_path = root.join("docs/specs/stories/E12S4.md");

    let meta_before = fs::metadata(&story_path).unwrap();
    let mtime_before = meta_before.modified().unwrap();

    let author = Author::new("human", "simon");

    for i in 1..=5 {
        append_scratch_entry(
            root,
            None,
            "E12S4",
            Some("note"),
            &format!("Note {}", i),
            &author,
            None,
        )
        .unwrap();
    }

    let _entries = read_scratch_entries(root, None, "E12S4").unwrap();

    let content_after = fs::read_to_string(&story_path).unwrap();
    let meta_after = fs::metadata(&story_path).unwrap();
    let mtime_after = meta_after.modified().unwrap();

    assert_eq!(
        original_content, content_after,
        "Story specification markdown must never be modified by scratchpad operations"
    );
    assert_eq!(
        mtime_before, mtime_after,
        "Story specification file mtime must not change"
    );
}

#[test]
fn test_scratch_read_all_entries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let author = Author::new("human", "simon");

    append_scratch_entry(root, None, "E12S4", Some("note"), "First", &author, None).unwrap();
    append_scratch_entry(
        root,
        None,
        "E12S4",
        Some("decision"),
        "Second",
        &author,
        None,
    )
    .unwrap();
    append_scratch_entry(
        root,
        None,
        "E12S4",
        Some("tradeoff"),
        "Third",
        &author,
        None,
    )
    .unwrap();

    let entries = read_scratch_entries(root, None, "E12S4").unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].seq, 1);
    assert_eq!(entries[0].text, "First");
    assert_eq!(entries[1].seq, 2);
    assert_eq!(entries[1].text, "Second");
    assert_eq!(entries[2].seq, 3);
    assert_eq!(entries[2].text, "Third");
}

#[test]
fn test_scratch_read_empty_scratchpad() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let entries = read_scratch_entries(root, None, "E12S4").unwrap();
    assert!(entries.is_empty());
}

#[test]
fn test_scratch_read_nonexistent_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let err = read_scratch_entries(root, None, "E99S99").unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("Entity file not found"));
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
}

#[test]
fn test_scratch_append_nonexistent_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let author = Author::new("human", "simon");
    let err = append_scratch_entry(root, None, "E99S99", Some("note"), "Note", &author, None)
        .unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("Entity file not found"));
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
}

#[test]
fn test_scratch_append_empty_text_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let author = Author::new("human", "simon");
    let err =
        append_scratch_entry(root, None, "E12S4", Some("note"), "   ", &author, None).unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
}

#[test]
fn test_scratch_append_invalid_kind_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story_file(root, "E12S4", "Story 4");

    let author = Author::new("human", "simon");
    let err = append_scratch_entry(root, None, "E12S4", Some("bogus"), "Text", &author, None)
        .unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert!(err
        .message()
        .contains("note, decision, tradeoff, transition"));
}

#[test]
fn test_summarize_scratch_entries_selection_and_deduplication() {
    let author = ScratchpadAuthor {
        r#type: "human".to_string(),
        id: "simon".to_string(),
    };

    let mut entries = Vec::new();
    for i in 1..=20 {
        let kind = match i {
            2 | 18 => "decision",
            10 => "transition",
            5 => "tradeoff",
            _ => "note",
        };
        entries.push(ScratchpadEntry {
            seq: i,
            at: "2026-09-13T12:00:00Z".to_string(),
            author: author.clone(),
            kind: kind.to_string(),
            text: format!("Entry {}", i),
        });
    }

    // With last_n = 5, we expect:
    // - Decisions: seq 2, 18
    // - Transitions: seq 10
    // - Last 5: seq 16, 17, 18, 19, 20
    // Deduplicated: 2, 10, 16, 17, 18, 19, 20 (7 entries total)
    let summary = summarize_scratch_entries(&entries, 5, None);
    let seqs: Vec<u32> = summary.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![2, 10, 16, 17, 18, 19, 20]);
}

#[test]
fn test_token_estimator_and_budget_truncation() {
    // 1. Token estimator: (chars + 3) / 4
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("a"), 1);
    assert_eq!(estimate_tokens("ab"), 1);
    assert_eq!(estimate_tokens("abc"), 1);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
    assert_eq!(estimate_tokens(&"x".repeat(100)), 25);
    assert_eq!(estimate_tokens(&"x".repeat(400)), 100);

    // 2. Budget truncation prioritizing key decisions and transitions
    let author = ScratchpadAuthor {
        r#type: "human".to_string(),
        id: "simon".to_string(),
    };

    let entries = vec![
        ScratchpadEntry {
            seq: 1,
            at: "2026-09-13T12:00:00Z".to_string(),
            author: author.clone(),
            kind: "decision".to_string(),
            text: "x".repeat(40), // 10 tokens
        },
        ScratchpadEntry {
            seq: 2,
            at: "2026-09-13T12:00:00Z".to_string(),
            author: author.clone(),
            kind: "note".to_string(),
            text: "x".repeat(40), // 10 tokens
        },
        ScratchpadEntry {
            seq: 3,
            at: "2026-09-13T12:00:00Z".to_string(),
            author: author.clone(),
            kind: "transition".to_string(),
            text: "x".repeat(40), // 10 tokens
        },
        ScratchpadEntry {
            seq: 4,
            at: "2026-09-13T12:00:00Z".to_string(),
            author: author.clone(),
            kind: "note".to_string(),
            text: "x".repeat(40), // 10 tokens
        },
    ];

    // Total = 40 tokens.
    // With budget 30 tokens: the lowest-priority non-key entry with smallest seq (seq 2 note) is dropped.
    // Remaining: seq 1 (decision), seq 3 (transition), seq 4 (note). Total = 30 tokens.
    let bounded = filter_scratch_entries_by_budget(&entries, 30);
    let seqs: Vec<u32> = bounded.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![1, 3, 4]);

    // With budget 20 tokens: seq 2 note and seq 4 note are both dropped.
    // Remaining: seq 1 (decision), seq 3 (transition). Total = 20 tokens.
    let bounded2 = filter_scratch_entries_by_budget(&entries, 20);
    let seqs2: Vec<u32> = bounded2.iter().map(|e| e.seq).collect();
    assert_eq!(seqs2, vec![1, 3]);

    // With budget 10 tokens: older key entry (seq 1 decision) dropped before newer key entry (seq 3 transition).
    // Remaining: seq 3 (transition). Total = 10 tokens.
    let bounded3 = filter_scratch_entries_by_budget(&entries, 10);
    let seqs3: Vec<u32> = bounded3.iter().map(|e| e.seq).collect();
    assert_eq!(seqs3, vec![3]);
}
