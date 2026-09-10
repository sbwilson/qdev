use std::collections::HashSet;
use std::fs;
use std::path::Path;

use proptest::prelude::*;
use qdev_core::store::{EntityRecord, SqliteStore, Store};
use qdev_core::write::Author;
use qdev_core::{
    allocate_decision_id, allocate_decision_id_in_with_rng, allocate_decision_id_with_rng,
    allocate_deferred_work_id, allocate_deferred_work_id_in_with_rng,
    allocate_deferred_work_id_with_rng, allocate_next_story_id, ConstraintKind, ConstraintOwner,
    EntityKind, HygieneConfig, IdParseError, Identifier, IdentifierKind, DEFAULT_CITATION_PATTERN,
};
use rand::RngCore;
use tempfile::tempdir;

#[test]
fn test_parse_and_classify_all_canonical_forms() {
    // 1. Epic: E12
    let id: Identifier = "E12".parse().expect("Failed to parse Epic");
    assert_eq!(id, Identifier::Epic { number: 12 });
    assert_eq!(id.kind(), IdentifierKind::Epic);
    assert_eq!(id.to_string(), "E12");

    // 2. Story: E12S4
    let id: Identifier = "E12S4".parse().expect("Failed to parse Story");
    assert_eq!(id, Identifier::Story { epic: 12, story: 4 });
    assert_eq!(id.kind(), IdentifierKind::Story);
    assert_eq!(id.to_string(), "E12S4");

    // 3. ADR: AD-43
    let id: Identifier = "AD-43".parse().expect("Failed to parse ADR");
    assert_eq!(id, Identifier::Adr { number: 43 });
    assert_eq!(id.kind(), IdentifierKind::Adr);
    assert_eq!(id.to_string(), "AD-43");

    // 4. FR: FR-102
    let id: Identifier = "FR-102".parse().expect("Failed to parse FR");
    assert_eq!(id, Identifier::FunctionalRequirement { number: 102 });
    assert_eq!(id.kind(), IdentifierKind::FunctionalRequirement);
    assert_eq!(id.to_string(), "FR-102");

    // 5. NFR: NFR-3
    let id: Identifier = "NFR-3".parse().expect("Failed to parse NFR");
    assert_eq!(id, Identifier::NonFunctionalRequirement { number: 3 });
    assert_eq!(id.kind(), IdentifierKind::NonFunctionalRequirement);
    assert_eq!(id.to_string(), "NFR-3");

    // 6. Hazard: HAZ-14
    let id: Identifier = "HAZ-14".parse().expect("Failed to parse Hazard");
    assert_eq!(id, Identifier::Hazard { number: 14 });
    assert_eq!(id.kind(), IdentifierKind::Hazard);
    assert_eq!(id.to_string(), "HAZ-14");

    // 7. PRD: PRD-1
    let id: Identifier = "PRD-1".parse().expect("Failed to parse PRD");
    assert_eq!(id, Identifier::Prd { number: 1 });
    assert_eq!(id.kind(), IdentifierKind::Prd);
    assert_eq!(id.to_string(), "PRD-1");

    // 8. Deferred Work: DW-7f3a
    let id: Identifier = "DW-7f3a".parse().expect("Failed to parse DW");
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "7f3a".to_string()
        }
    );
    assert_eq!(id.kind(), IdentifierKind::DeferredWork);
    assert_eq!(id.to_string(), "DW-7f3a");

    // 9. Decision: DEC-2b91
    let id: Identifier = "DEC-2b91".parse().expect("Failed to parse DEC");
    assert_eq!(
        id,
        Identifier::Decision {
            hash: "2b91".to_string()
        }
    );
    assert_eq!(id.kind(), IdentifierKind::Decision);
    assert_eq!(id.to_string(), "DEC-2b91");

    // 10. Story Constraint: E12S4/NG-1
    let id: Identifier = "E12S4/NG-1"
        .parse()
        .expect("Failed to parse Story Constraint");
    assert_eq!(
        id,
        Identifier::Constraint {
            owner: ConstraintOwner::Story(12, 4),
            kind: ConstraintKind::NoGo,
            number: 1,
        }
    );
    assert_eq!(id.kind(), IdentifierKind::Constraint);
    assert_eq!(id.to_string(), "E12S4/NG-1");

    // 11. Epic Constraint: E12/RH-2
    let id: Identifier = "E12/RH-2".parse().expect("Failed to parse Epic Constraint");
    assert_eq!(
        id,
        Identifier::Constraint {
            owner: ConstraintOwner::Epic(12),
            kind: ConstraintKind::RabbitHole,
            number: 2,
        }
    );
    assert_eq!(id.kind(), IdentifierKind::Constraint);
    assert_eq!(id.to_string(), "E12/RH-2");
}

#[test]
fn test_sprint_prefixed_rejections() {
    let sprint_prefixed_cases = ["S5E2S4", "S1E2", "S5", "S12E3S4", "S5E2/NG-1"];
    for s in sprint_prefixed_cases {
        let err = s
            .parse::<Identifier>()
            .expect_err("Must reject sprint-prefixed");
        match err {
            IdParseError::SprintPrefixed(rejected) => {
                assert!(
                    rejected.contains(s) || s.contains(&rejected),
                    "Expected sprint-prefixed error for '{}', got '{}'",
                    s,
                    rejected
                );
            }
            other => panic!(
                "Expected IdParseError::SprintPrefixed for '{}', got {:?}",
                s, other
            ),
        }
    }
}

#[test]
fn test_invalid_prefix_rejections() {
    let invalid_prefixes = ["X12", "STORY-1", "TASK-42", "FOO-1"];
    for s in invalid_prefixes {
        let err = s
            .parse::<Identifier>()
            .expect_err("Must reject invalid prefix");
        match err {
            IdParseError::InvalidPrefix(rejected) => {
                assert_eq!(rejected, s);
            }
            other => panic!(
                "Expected IdParseError::InvalidPrefix for '{}', got {:?}",
                s, other
            ),
        }
    }
}

#[test]
fn test_zero_number_rejections() {
    let zero_cases = [
        "E0",
        "AD-0",
        "E12S0",
        "E0S4",
        "FR-0",
        "NFR-0",
        "HAZ-0",
        "PRD-0",
        "E12S4/NG-0",
        "E12/RH-0",
    ];
    for s in zero_cases {
        let err = s
            .parse::<Identifier>()
            .expect_err("Must reject zero number");
        match err {
            IdParseError::NonPositiveInteger(rejected) => {
                assert_eq!(rejected, s);
            }
            other => panic!(
                "Expected IdParseError::NonPositiveInteger for '{}', got {:?}",
                s, other
            ),
        }
    }
}

#[test]
fn test_invalid_hex_rejections() {
    let invalid_hex = [
        ("DW-xyz", "character violation"),
        ("DW-12", "length violation"),
        ("DW-7F3A", "uppercase violation"),
        ("DEC-xyz", "character violation"),
        ("DEC-12", "length violation"),
        ("DEC-ABCD", "uppercase violation"),
    ];
    for (s, desc) in invalid_hex {
        let err = s
            .parse::<Identifier>()
            .expect_err(&format!("Must reject invalid hex: {}", desc));
        match err {
            IdParseError::InvalidHex { .. } => {}
            other => panic!(
                "Expected IdParseError::InvalidHex for '{}', got {:?}",
                s, other
            ),
        }
    }
}

#[test]
fn test_invalid_constraint_rejections() {
    let invalid_constraints = [
        "AD-43/NG-1",      // ADR cannot own constraint
        "FR-102/RH-1",     // FR cannot own constraint
        "E12S4/UNKNOWN-1", // Invalid kind
        "E12/FOO-2",       // Invalid kind
        "E12/NG",          // Missing number
    ];
    for s in invalid_constraints {
        assert!(
            s.parse::<Identifier>().is_err(),
            "Expected failure for invalid constraint '{}'",
            s
        );
    }
}

#[test]
fn test_json_serde_round_trip() {
    let id = Identifier::Story { epic: 12, story: 4 };
    let json_str = serde_json::to_string(&id).expect("Serialization failed");
    assert_eq!(json_str, "\"E12S4\"");
    let deserialized: Identifier = serde_json::from_str(&json_str).expect("Deserialization failed");
    assert_eq!(id, deserialized);
}

// Proptest property-based testing of round-trip parsing
prop_compose! {
    fn arb_hex_hash()(chars in prop::collection::vec(prop::sample::select(b"0123456789abcdef".to_vec()), 4..=12)) -> String {
        chars.iter().map(|&c| c as char).collect()
    }
}

prop_compose! {
    fn arb_identifier()(
        kind in 0u8..10,
        n1 in 1u32..10_000,
        n2 in 1u32..10_000,
        hash in arb_hex_hash(),
        is_nogo in any::<bool>(),
        is_owner_story in any::<bool>()
    ) -> Identifier {
        match kind {
            0 => Identifier::Epic { number: n1 },
            1 => Identifier::Story { epic: n1, story: n2 },
            2 => Identifier::Adr { number: n1 },
            3 => Identifier::FunctionalRequirement { number: n1 },
            4 => Identifier::NonFunctionalRequirement { number: n1 },
            5 => Identifier::Hazard { number: n1 },
            6 => Identifier::Prd { number: n1 },
            7 => Identifier::DeferredWork { hash },
            8 => Identifier::Decision { hash },
            _ => {
                let owner = if is_owner_story {
                    ConstraintOwner::Story(n1, n2)
                } else {
                    ConstraintOwner::Epic(n1)
                };
                let kind = if is_nogo {
                    ConstraintKind::NoGo
                } else {
                    ConstraintKind::RabbitHole
                };
                Identifier::Constraint { owner, kind, number: n1 }
            }
        }
    }
}

proptest! {
    #[test]
    fn test_property_identifier_round_trip(id in arb_identifier()) {
        let formatted = id.to_string();
        let parsed = formatted.parse::<Identifier>();
        prop_assert_eq!(parsed, Ok(id.clone()));
        prop_assert_eq!(id.to_string(), formatted);
    }
}

#[test]
fn test_story_allocation_clean_dir() {
    let tmp = tempdir().unwrap();
    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 1 });
}

#[test]
fn test_story_allocation_sequential() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    fs::write(stories_dir.join("E12S1.md"), "").unwrap();
    fs::write(stories_dir.join("E12S2.md"), "").unwrap();

    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 3 });
}

/// The allocator is monotonic per epic, not lowest-free: it takes one past the highest number in
/// the shared in-use id set. It reads that set rather than its own private scan, which is what
/// makes `create story` and `--fix-ids` unable to disagree about which ids are *taken* — they
/// still allocate differently, and that difference is pinned in
/// `test_story_allocator_never_returns_an_id_the_shared_set_contains`.
#[test]
fn test_story_allocation_with_gaps() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    fs::write(stories_dir.join("E12S1.md"), "").unwrap();
    fs::write(stories_dir.join("E12S4.md"), "").unwrap();

    // One past the highest, not the lowest free: a gap is left rather than a deleted story's id
    // being handed out again, because an id is a citation target (AD-7).
    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 5 });
}

#[test]
fn test_story_allocation_ignores_other_epics_and_non_story_files() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    fs::write(stories_dir.join("E11S10.md"), "").unwrap();
    fs::write(stories_dir.join("E13S1.md"), "").unwrap();
    fs::write(stories_dir.join("E12S1.txt"), "").unwrap(); // Non-md
    fs::write(stories_dir.join("README.md"), "").unwrap(); // Non-story

    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 1 });
}

#[test]
fn test_story_allocation_rejects_zero_epic() {
    let tmp = tempdir().unwrap();
    let res = allocate_next_story_id(tmp.path(), 0);
    assert!(res.is_err());
}

// Mock RNG to test collision growth deterministically
struct MockRng {
    bytes: Vec<u8>,
    idx: usize,
}

impl MockRng {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, idx: 0 }
    }
}

impl RngCore for MockRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        let b = self.next_byte();
        b as u64
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for b in dest {
            *b = self.next_byte();
        }
    }
}

impl MockRng {
    fn next_byte(&mut self) -> u8 {
        if self.idx < self.bytes.len() {
            let val = self.bytes[self.idx];
            self.idx += 1;
            val
        } else {
            0
        }
    }
}

#[test]
fn test_dw_allocation_collision_growth_4_to_6_to_8() {
    let tmp = tempdir().unwrap();
    let dw_dir = tmp.path().join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();

    // 1. Clean allocation: 4 chars
    let id = allocate_deferred_work_id(tmp.path()).expect("allocation should succeed");
    if let Identifier::DeferredWork { hash } = id {
        assert_eq!(hash.len(), 4);
    } else {
        panic!("Expected DeferredWork identifier");
    }

    // 2. Deterministic collision growth:
    // First 4 hex digits: 0x1, 0x2, 0x3, 0x4 -> "1234"
    // Next 2 hex digits: 0x5, 0x6 -> "56" (making "123456")
    // Next 2 hex digits: 0x7, 0x8 -> "78" (making "12345678")
    let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];

    // Create collision for 4 chars: "DW-1234.md"
    fs::write(dw_dir.join("DW-1234.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes.clone());
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "123456".to_string()
        }
    );

    // Create collision for 6 chars: "DW-123456.md"
    fs::write(dw_dir.join("DW-123456.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "12345678".to_string()
        }
    );
}

#[test]
fn test_dec_allocation_collision_growth_4_to_6_to_8() {
    let tmp = tempdir().unwrap();
    let dec_dir = tmp.path().join("docs/state/decisions");
    fs::create_dir_all(&dec_dir).unwrap();

    // 1. Clean allocation: 4 chars
    let id = allocate_decision_id(tmp.path()).expect("allocation should succeed");
    if let Identifier::Decision { hash } = id {
        assert_eq!(hash.len(), 4);
    } else {
        panic!("Expected Decision identifier");
    }

    // 2. Deterministic collision growth:
    let bytes = vec![0xa, 0xb, 0xc, 0xd, 0xe, 0xf, 1, 2];

    // Create collision for 4 chars: "DEC-abcd.md"
    fs::write(dec_dir.join("DEC-abcd.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes.clone());
    let id = allocate_decision_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::Decision {
            hash: "abcdef".to_string()
        }
    );

    // Create collision for 6 chars: "DEC-abcdef.md"
    fs::write(dec_dir.join("DEC-abcdef.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_decision_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::Decision {
            hash: "abcdef12".to_string()
        }
    );
}

#[test]
fn test_citation_pattern_matching_and_rejection() {
    let hygiene_default = HygieneConfig::default();
    assert!(hygiene_default.citation_pattern.is_some());
    let pattern_str = hygiene_default.citation_pattern.unwrap();
    assert_eq!(pattern_str, DEFAULT_CITATION_PATTERN);

    let re = regex::Regex::new(&pattern_str).expect("Valid regex");

    let happy_cases = [
        "[E12]",
        "[E12S4]",
        "[AD-43]",
        "[FR-102]",
        "[NFR-3]",
        "[HAZ-14]",
        "[PRD-1]",
        "[DW-7f3a]",
        "[DEC-2b91]",
        "[E12S4/NG-1]",
        "[E12/RH-2]",
    ];
    for case in happy_cases {
        assert!(
            re.is_match(case),
            "Expected citation pattern to match '{}'",
            case
        );
    }

    let rejection_cases = [
        "[S5E2S4]",
        "[UNKNOWN-1]",
        "[DW-12]",
        "[DW-zzzz]",
        "[E12S4/UNKNOWN-1]",
        "[E0]",
        "[AD-0]",
        "[E12S0]",
        "[FR-0]",
    ];
    for case in rejection_cases {
        assert!(
            !re.is_match(case),
            "Expected citation pattern to reject '{}'",
            case
        );
    }
}

#[test]
fn test_story_allocation_slugged_and_padded_files() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    fs::write(stories_dir.join("E12S01-init.md"), "").unwrap();
    fs::write(stories_dir.join("E12S2-buffer-layout.md"), "").unwrap();

    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 3 });
}

#[test]
fn test_hex_allocation_collision_with_slugged_files() {
    let tmp = tempdir().unwrap();
    let dw_dir = tmp.path().join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();

    // Create collision with slugged file name: "DW-1234-buffer.md"
    fs::write(dw_dir.join("DW-1234-buffer.md"), "").unwrap();

    let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "123456".to_string()
        }
    );
}

#[test]
fn test_hex_allocation_collision_with_underscore_slugged_files() {
    let tmp = tempdir().unwrap();
    let dw_dir = tmp.path().join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();

    // Create collision with underscore slugged file name: "DW-1234_buffer.md"
    fs::write(dw_dir.join("DW-1234_buffer.md"), "").unwrap();

    let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng)
        .expect("allocation should succeed");
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "123456".to_string()
        }
    );
}

/// A story numbered at `u32::MAX` no longer overflows the allocator: it is one member of the
/// in-use set like any other, so one past it does not exist. The
/// One past the highest is unrepresentable when the highest is `u32::MAX`, so allocation refuses
/// rather than wrapping.
#[test]
fn test_story_allocation_overflow() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    fs::write(stories_dir.join("E12S4294967295.md"), "").unwrap();

    // The highest member of the space is in use, so one past it does not exist.
    let res = allocate_next_story_id(tmp.path(), 12);
    let err = res.expect_err("allocation past the id space must fail");
    assert_eq!(err.code(), "id_space_exhausted");
}

#[test]
fn test_story_allocation_ignores_directory_entries() {
    let tmp = tempdir().unwrap();
    let stories_dir = tmp.path().join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();

    // A directory named E12S99.md must be ignored because it is not a file
    fs::create_dir_all(stories_dir.join("E12S99.md")).unwrap();
    fs::write(stories_dir.join("E12S1.md"), "").unwrap();

    let id = allocate_next_story_id(tmp.path(), 12).expect("Allocation should succeed");
    assert_eq!(id, Identifier::Story { epic: 12, story: 2 });
}

// ---------------------------------------------------------------------------
// One rule for which ids are in use
// ---------------------------------------------------------------------------

/// Writes a minimal entity file declaring `id`, creating parent directories.
fn write_entity(path: &std::path::Path, id: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "---\nid: {id}\ntitle: \"Entity {id}\"\nstatus: draft\nversion: 1\n\
             created_by:\n  type: human\n  id: simon\n\
             updated_by:\n  type: human\n  id: simon\n---\n\n## Acceptance Criteria\n- AC.\n"
        ),
    )
    .unwrap();
}

/// A workspace shaped like every row of the in-use matrix at once, so one assertion covers the
/// whole rule rather than one instance of it.
fn matrix_workspace(root: &std::path::Path) {
    let stories = root.join("docs/specs/stories");
    // Declared, in a subdirectory hydration reads recursively.
    write_entity(&stories.join("sub/E1S1.md"), "E1S1");
    // Declared, in a file whose name differs from its id only in case.
    write_entity(&stories.join("e1s2-buffer.md"), "E1S2");
    // Declared, with an extension hydration matches case-insensitively.
    write_entity(&stories.join("E1S3.MD"), "E1S3");
    // Carried by the name only: the frontmatter will not parse, so nothing is declared.
    fs::write(stories.join("E1S4.md"), "---\nid: E1S4\nbroken: [\n---\n").unwrap();
    // A state-directory id, which is in use exactly like a spec-directory one.
    write_entity(&root.join("docs/state/decisions/DEC-2b91.md"), "DEC-2b91");
    // Not an id at all, and must not become one.
    fs::write(stories.join("README.md"), "notes\n").unwrap();
}

/// The in-use set is the union the rule names: ids declared in frontmatter anywhere under either
/// hydrated tree, plus ids carried by file names. Recursive, extension matched case-insensitively
/// (`E1S3.MD` is read by hydration, so its id is taken), and a file whose frontmatter will not
/// parse still occupies the id its name carries.
#[test]
fn test_ids_in_use_unions_declared_and_carried_ids_across_both_trees() {
    let tmp = tempdir().unwrap();
    matrix_workspace(tmp.path());

    let storage = qdev_core::StorageConfig::default();
    let used = qdev_core::ids_in_use(tmp.path(), &storage, None).expect("scan should succeed");

    for id in ["E1S1", "E1S2", "E1S3", "E1S4", "DEC-2b91"] {
        assert!(used.contains(id), "'{id}' must be in use: {used:?}");
    }
    assert!(
        !used.contains("README"),
        "a file name that carries no id must not mint one: {used:?}"
    );
}

/// The invariant, not the instances: `qdev create story`'s allocator keeps no scan of its own, so
/// whatever the shared in-use set contains, the allocator never returns a member of it. Asserted
/// against the shared function directly rather than inferred from the two commands' outputs.
///
/// The fixture is deliberately **gapped**. A contiguous one (`E1S1..E1S4`) makes monotonic and
/// lowest-free allocation give the same answer, so it cannot tell the invariant from a
/// coincidence — which is exactly how the first version of this test passed while asserting
/// something the allocator does not do.
#[test]
fn test_story_allocator_never_returns_an_id_the_shared_set_contains() {
    let tmp = tempdir().unwrap();
    matrix_workspace(tmp.path());
    let stories = tmp.path().join("docs/specs/stories");
    // A gap at E1S5/E1S6 with E1S7 taken, so the two sequences diverge.
    write_entity(&stories.join("E1S7.md"), "E1S7");

    let storage = qdev_core::StorageConfig::default();
    let used = qdev_core::ids_in_use(tmp.path(), &storage, None).expect("scan should succeed");
    let allocated = qdev_core::allocate_next_story_id_in(tmp.path(), &storage, 1, None)
        .expect("Allocation should succeed");

    // The invariant.
    assert!(
        !used.contains(&allocated.to_string()),
        "the allocator returned {} which the shared set contains: {:?}",
        allocated,
        used
    );

    // And the sequence, pinned separately so the two are never conflated again: `create story`
    // is monotonic per epic (one past the highest), while `next_available_id` — which `--fix-ids`
    // uses, because a renumber must land somewhere free — is lowest-free. On this gapped
    // fixture they differ, which is the point of asserting them apart.
    assert_eq!(allocated, Identifier::Story { epic: 1, story: 8 });
    let lowest_free =
        qdev_core::next_available_id(&Identifier::Story { epic: 1, story: 1 }, &used).unwrap();
    assert_eq!(lowest_free, Identifier::Story { epic: 1, story: 5 });
    assert_ne!(
        allocated, lowest_free,
        "a gapped fixture must make the two sequences differ, or this test proves nothing"
    );
}

/// The allocator refuses rather than colliding when it cannot allocate: `next_available_id`
/// rejects a kind whose ids are not sequential, and every reachable story number being taken is
/// `id_space_exhausted`, never a returned duplicate.
#[test]
fn test_shared_set_never_yields_an_id_it_contains() {
    let tmp = tempdir().unwrap();
    let stories = tmp.path().join("docs/specs/stories");
    // Two spellings of one id, neither of which the old scan saw.
    write_entity(&stories.join("nested/deeper/E9S1.md"), "E9S1");
    write_entity(&stories.join("e9s2_slug.md"), "E9S2");

    let storage = qdev_core::StorageConfig::default();
    let used = qdev_core::ids_in_use(tmp.path(), &storage, None).unwrap();
    let allocated = qdev_core::allocate_next_story_id_in(tmp.path(), &storage, 9, None).unwrap();
    assert_eq!(allocated, Identifier::Story { epic: 9, story: 3 });
    assert!(!used.contains(&allocated.to_string()));
}

/// Allocation works in a bare directory: no workspace, no cache, filesystem alone.
#[test]
fn test_allocation_works_without_a_workspace_or_cache() {
    let tmp = tempdir().unwrap();
    let id = qdev_core::allocate_next_story_id_in(
        tmp.path(),
        &qdev_core::StorageConfig::default(),
        4,
        None,
    )
    .expect("Allocation should succeed with no workspace at all");
    assert_eq!(id, Identifier::Story { epic: 4, story: 1 });
}

/// The filename half of the rule, spelled out: an id may itself contain `-`, a slug may follow it
/// after `-` or `_`, a zero-padded or lower-cased name still carries the id it denotes, and a name
/// that carries no id yields none.
#[test]
fn test_id_carried_by_filename() {
    use qdev_core::id_carried_by_filename as carried;
    assert_eq!(carried("E1S1.md").as_deref(), Some("E1S1"));
    assert_eq!(carried("E1S1-buffer-layout.md").as_deref(), Some("E1S1"));
    assert_eq!(carried("E1S1_buffer.md").as_deref(), Some("E1S1"));
    assert_eq!(carried("e1s1-buffer.md").as_deref(), Some("E1S1"));
    assert_eq!(carried("E12S01.md").as_deref(), Some("E12S1"));
    assert_eq!(carried("AD-7.md").as_deref(), Some("AD-7"));
    assert_eq!(
        carried("AD-7-context-and-decision.md").as_deref(),
        Some("AD-7")
    );
    assert_eq!(carried("FR-101.md").as_deref(), Some("FR-101"));
    assert_eq!(carried("DW-7f3a.md").as_deref(), Some("DW-7f3a"));
    assert_eq!(carried("E1.md").as_deref(), Some("E1"));
    assert_eq!(carried("README.md"), None);
    assert_eq!(carried("sprint-5.md"), None);
    assert_eq!(carried("login-flow.md"), None);
    // The extension is matched case-insensitively, unlike the *resolution* rule: this function
    // answers "does this name occupy an id?", and an `E1S1.MD` file occupies it whether or not
    // the write path would resolve the name. Requiring lowercase left a `.MD` file with
    // unparseable frontmatter in neither half of the union.
    assert_eq!(carried("E1S1.MD").as_deref(), Some("E1S1"));
    assert_eq!(carried("E1S1.Md").as_deref(), Some("E1S1"));
    // Mixed-case grammar: planning ids are uppercase, the hex suffix of a `DW-`/`DEC-` id is
    // lowercase and only that spelling parses.
    assert_eq!(carried("dw-7f3a.md").as_deref(), Some("DW-7f3a"));
    assert_eq!(carried("DW-7f3a.md").as_deref(), Some("DW-7f3a"));
    assert_eq!(carried("dec-0a1b-note.md").as_deref(), Some("DEC-0a1b"));
    // Still not an id, whatever the case.
    assert_eq!(carried("notes.MD"), None);
}

// ---------------------------------------------------------------------------
// The hex allocators ask the one id-in-use rule
// ---------------------------------------------------------------------------
//
// Every test below is a way a workspace can already own a `DW-`/`DEC-` hash that the deleted
// private scan — one `read_dir` of one flat directory, `starts_with` on the exact case — could
// not see. They are the same holes `spec-one-id-in-use-rule.md` closed for `create story`, one
// allocator later.

/// A minimal frontmatter declaration: the *declared* half of the in-use union. The file's own
/// name deliberately carries no id, so each test exercises one membership at a time.
fn declare_id(path: &Path, id: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "---\nid: {id}\ntitle: \"Holder of {id}\"\nstatus: draft\nversion: 1\n---\n\nBody.\n"
        ),
    )
    .unwrap();
}

fn hex_entity(id: &str, kind: EntityKind, source_path: &str) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind,
        title: Some(format!("{} title", id)),
        status: Some("draft".to_string()),
        owners: None,
        source_path: source_path.to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

/// An rng that yields `7f3a` for the first four hex digits and `01` for each pair after, so a
/// collision on `7f3a` shows up as the 6-character `7f3a01` and nothing else can produce it.
fn rng_yielding_7f3a_then_01() -> MockRng {
    MockRng::new(vec![0x7, 0xf, 0x3, 0xa, 0x0, 0x1, 0x0, 0x1])
}

fn dw_hash(id: Identifier) -> String {
    match id {
        Identifier::DeferredWork { hash } => hash,
        other => panic!("expected a DW- identifier, got {other}"),
    }
}

fn dec_hash(id: Identifier) -> String {
    match id {
        Identifier::Decision { hash } => hash,
        other => panic!("expected a DEC- identifier, got {other}"),
    }
}

/// Matrix: *hex id declared by a nested file*. The private scan read one directory level, so a
/// file in `<state_dir>/dw/archive/` was invisible to it while hydration reads it.
#[test]
fn test_hex_allocation_skips_an_id_declared_by_a_nested_file() {
    let tmp = tempdir().unwrap();
    declare_id(&tmp.path().join("docs/state/dw/archive/old.md"), "DW-7f3a");

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut rng).unwrap();
    assert_eq!(
        dw_hash(id),
        "7f3a01",
        "a nested declaration owns the hash, so allocation must grow past it"
    );
}

/// Matrix: *hex id carried by a `.MD` name*. Hydration matches the extension
/// case-insensitively, so it reads this file; an allocator that does not is handing out an id
/// qdev can already see.
#[test]
fn test_hex_allocation_skips_an_id_carried_by_an_uppercase_extension() {
    let tmp = tempdir().unwrap();
    let dw_dir = tmp.path().join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(dw_dir.join("DW-7f3a.MD"), "").unwrap();

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut rng).unwrap();
    assert_eq!(dw_hash(id), "7f3a01");
}

/// Matrix: *hex id in an unparseable file's name*. The declared half skips a file whose
/// frontmatter will not parse; the carried half is what keeps its id owned.
#[test]
fn test_hex_allocation_skips_an_id_carried_by_an_unparseable_file() {
    let tmp = tempdir().unwrap();
    let dw_dir = tmp.path().join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join("DW-7f3a-notes.md"),
        "no frontmatter here, just prose\n",
    )
    .unwrap();

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut rng).unwrap();
    assert_eq!(dw_hash(id), "7f3a01");
}

/// Matrix: *hex id only in the cache*. The cache is a union member, so an id survives its file
/// being deleted — and `store: None` still answers from the filesystem halves alone, which is
/// what an uninitialised workspace can offer.
#[test]
fn test_hex_allocation_skips_a_cache_only_id() {
    let tmp = tempdir().unwrap();
    let storage = qdev_core::StorageConfig::default();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&hex_entity(
            "DEC-1234",
            EntityKind::Decision,
            "docs/state/decisions/DEC-1234.md",
        ))
        .unwrap();

    // Nothing on disk: without the cache the filesystem halves see the hash as free.
    let mut rng = MockRng::new(vec![0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]);
    let free = allocate_decision_id_in_with_rng(tmp.path(), &storage, &mut rng, None).unwrap();
    assert_eq!(dec_hash(free), "1234");

    let mut rng = MockRng::new(vec![0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]);
    let taken =
        allocate_decision_id_in_with_rng(tmp.path(), &storage, &mut rng, Some(&store)).unwrap();
    assert_eq!(
        dec_hash(taken),
        "123456",
        "an id the cache holds is taken even with no file on disk"
    );
}

/// Matrix: *hex id declared mis-cased*. The union's members disagree on case by construction —
/// the carried and cached halves are canonical (a hex hash is lowercase), the declared half is
/// whatever the frontmatter says — so the comparison is case-insensitive on both sides.
#[test]
fn test_hex_allocation_skips_a_mis_cased_declaration() {
    let tmp = tempdir().unwrap();
    declare_id(&tmp.path().join("docs/state/dw/holder.md"), "DW-7F3A");

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut rng).unwrap();
    assert_eq!(
        dw_hash(id),
        "7f3a01",
        "`id: DW-7F3A` must take `dw-7f3a`'s space"
    );
}

/// Matrix: *hex id outside its own directory*. Ownership is workspace-wide: an id is what a
/// citation names, and the directory it happens to live in does not narrow that. The private
/// scan looked only under `<state_dir>/dw/`.
#[test]
fn test_hex_allocation_skips_an_id_declared_outside_its_own_directory() {
    let tmp = tempdir().unwrap();
    declare_id(
        &tmp.path().join("docs/specs/stories/stray-holder.md"),
        "DW-7f3a",
    );

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut rng).unwrap();
    assert_eq!(
        dw_hash(id),
        "7f3a01",
        "a `DW-` id declared under specs_dir is still taken"
    );
}

/// Matrix: *length growth still works*, now driven by the shared id set rather than by files in
/// one directory — the strategy is what this story keeps. Every hash the rigged rng can yield is
/// owned, so the allocator grows 4 -> 6 -> 8 and then returns its last 8-character candidate
/// rather than looping forever.
#[test]
fn test_hex_allocation_growth_and_bounded_retries_over_the_shared_set() {
    let tmp = tempdir().unwrap();
    let storage = qdev_core::StorageConfig::default();

    // The rng yields nothing but `0` nibbles, so the only candidates it can ever produce are
    // `0000`, `000000` and `00000000`.
    declare_id(&tmp.path().join("docs/state/dw/a.md"), "DW-0000");
    let mut rng = MockRng::new(vec![]);
    let id = allocate_deferred_work_id_in_with_rng(tmp.path(), &storage, &mut rng, None).unwrap();
    assert_eq!(dw_hash(id), "000000", "4 grows to 6");

    declare_id(&tmp.path().join("docs/state/dw/b.md"), "DW-000000");
    let mut rng = MockRng::new(vec![]);
    let id = allocate_deferred_work_id_in_with_rng(tmp.path(), &storage, &mut rng, None).unwrap();
    assert_eq!(dw_hash(id), "00000000", "6 grows to 8");

    declare_id(&tmp.path().join("docs/state/dw/c.md"), "DW-00000000");
    let mut rng = MockRng::new(vec![]);
    let id = allocate_deferred_work_id_in_with_rng(tmp.path(), &storage, &mut rng, None).unwrap();
    assert_eq!(
        dw_hash(id),
        "00000000",
        "exhausted retries return the last candidate rather than hanging"
    );
}

/// The invariant itself, rather than one of its instances: whatever way the workspace owns a
/// hash, the allocator's answer is not a member of `ids_in_use`. Asserted for both prefixes over
/// one workspace that holds every membership at once, with the rng rigged to *demand* each owned
/// hash in turn — a test that let the rng wander would pass by coincidence, since a random
/// 4-character hash almost never lands on one of five fixture ids.
#[test]
fn test_hex_allocators_never_return_a_member_of_the_shared_set() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let storage = qdev_core::StorageConfig::default();

    // Each of the five memberships, once: declared nested, declared mis-cased under `specs_dir`
    // (so also outside its own directory), carried by a `.MD` name, carried by an unparseable
    // file's name, and held only by the cache.
    declare_id(&root.join("docs/state/dw/archive/old.md"), "DW-7f3a");
    declare_id(&root.join("docs/specs/stories/stray.md"), "DEC-7F3A");
    fs::write(root.join("docs/state/dw/DW-abcd.MD"), "").unwrap();
    fs::create_dir_all(root.join("docs/state/decisions")).unwrap();
    fs::write(
        root.join("docs/state/decisions/DEC-abcd-notes.md"),
        "unparseable\n",
    )
    .unwrap();

    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&hex_entity(
            "DW-1234",
            EntityKind::DeferredWork,
            "docs/state/dw/DW-1234.md",
        ))
        .unwrap();

    let used = qdev_core::ids_in_use(root, &storage, Some(&store)).unwrap();
    let used_lower: HashSet<String> = used.iter().map(|id| id.to_ascii_lowercase()).collect();
    assert_eq!(
        used_lower.len(),
        5,
        "fixture must contribute all five memberships: {used_lower:?}"
    );

    // The rng yields the demanded hash's four nibbles, then `01` pairs for each growth step, so
    // the only way to avoid the demanded hash is to grow past it.
    for demanded in ["7f3a", "abcd", "1234", "0000"] {
        let mut bytes: Vec<u8> = demanded
            .chars()
            .map(|c| c.to_digit(16).unwrap() as u8)
            .collect();
        bytes.extend([0x0, 0x1, 0x0, 0x1]);

        let mut rng = MockRng::new(bytes.clone());
        let dw = allocate_deferred_work_id_in_with_rng(root, &storage, &mut rng, Some(&store))
            .unwrap()
            .to_string();
        assert!(
            !used_lower.contains(&dw.to_ascii_lowercase()),
            "asked for {demanded}, allocated {dw}, which the workspace already owns"
        );

        let mut rng = MockRng::new(bytes);
        let dec = allocate_decision_id_in_with_rng(root, &storage, &mut rng, Some(&store))
            .unwrap()
            .to_string();
        assert!(
            !used_lower.contains(&dec.to_ascii_lowercase()),
            "asked for {demanded}, allocated {dec}, which the workspace already owns"
        );
    }
}

/// The `storage` argument is the one that is used. This is the *original* defect's shape — an
/// allocator asking about the wrong tree — and the review layer demonstrated that replacing
/// `storage` with `StorageConfig::default()` in both allocator bodies left the whole suite green,
/// because every other hex fixture is built under the default layout.
#[test]
fn test_hex_allocation_resolves_the_configured_storage_layout() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let storage = qdev_core::StorageConfig {
        specs_dir: "planning/specs".to_string(),
        state_dir: "planning/state".to_string(),
        cache_dir: ".qdev/cache".to_string(),
    };

    // Owned under the *configured* tree, and a decoy owned under the default one.
    declare_id(&root.join("planning/state/dw/holder.md"), "DW-7f3a");
    declare_id(&root.join("docs/state/dw/decoy.md"), "DW-abcd");

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_in_with_rng(root, &storage, &mut rng, None).unwrap();
    assert_eq!(
        dw_hash(id),
        "7f3a01",
        "the configured state_dir owns this hash, so allocation must grow past it"
    );

    let mut rng = MockRng::new(vec![0xa, 0xb, 0xc, 0xd, 0x0, 0x1, 0x0, 0x1]);
    let id = allocate_deferred_work_id_in_with_rng(root, &storage, &mut rng, None).unwrap();
    assert_eq!(
        dw_hash(id),
        "abcd",
        "the default-layout tree is not this workspace's; a hash owned only there is free"
    );
}

/// The retry branch's own oracle. The growth test above cannot see it: with an all-zero rng the
/// step-3 candidate and every retry candidate are the same string, so deleting the membership
/// test on the retry path leaves it green. Here each retry differs, and the second one is free.
#[test]
fn test_hex_allocation_retry_path_tests_each_fresh_candidate() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let storage = qdev_core::StorageConfig::default();

    // Everything the rng can produce before the second retry is owned.
    for id in ["DW-0000", "DW-000000", "DW-00000000", "DW-11111111"] {
        declare_id(&root.join(format!("docs/state/dw/{id}.md")), id);
    }

    // 4 nibbles -> `0000`; +2 -> `000000`; +2 -> `00000000`; retry 1 -> `11111111`;
    // retry 2 -> `22222222`, the first candidate the workspace does not own.
    let mut bytes = vec![0x0; 8];
    bytes.extend([0x1; 8]);
    bytes.extend([0x2; 8]);

    let mut rng = MockRng::new(bytes);
    let id = allocate_deferred_work_id_in_with_rng(root, &storage, &mut rng, None).unwrap();
    assert_eq!(
        dw_hash(id),
        "22222222",
        "a retry candidate that is owned must be rejected like any other"
    );
}

/// The two id spaces are separate spaces. Membership is tested on `{prefix}-{hash}`, so `DW-7f3a`
/// being owned must leave `DEC-7f3a` allocatable — an oracle that compared bare hashes would pass
/// every other test here while silently halving both spaces.
#[test]
fn test_hex_allocation_does_not_confuse_the_two_prefixes() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let storage = qdev_core::StorageConfig::default();
    declare_id(&root.join("docs/state/dw/holder.md"), "DW-7f3a");

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_decision_id_in_with_rng(root, &storage, &mut rng, None).unwrap();
    assert_eq!(
        dec_hash(id),
        "7f3a",
        "`DW-7f3a` says nothing about whether `DEC-7f3a` is taken"
    );
}

/// A name occupies an id when the identity rule *resolves* it — `<id>.md`, `<id>-<slug>.md`,
/// `<id>_<slug>.md` — and not otherwise. `DW-7f3a.notes.md` and a `DW-abcd.txt` sidecar resolve
/// to nothing, so they own nothing.
///
/// This is **narrower than the deleted scan**, which refused any name in `<state_dir>/dw/`
/// merely starting with `DW-7f3a.`, and the narrowing is the intended answer rather than a
/// regression: `create dw` writes `DW-7f3a.md`, which collides with neither file, and occupancy
/// that disagreed with resolution is the divergence this whole invariant exists to remove. The
/// test is here so the answer is a decision on the record instead of a side effect.
#[test]
fn test_hex_allocation_ignores_names_the_identity_rule_does_not_resolve() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let storage = qdev_core::StorageConfig::default();
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join("DW-7f3a.notes.md"),
        "loose notes, no frontmatter\n",
    )
    .unwrap();
    fs::write(dw_dir.join("DW-abcd.txt"), "not markdown at all\n").unwrap();

    let used = qdev_core::ids_in_use(root, &storage, None).unwrap();
    assert!(
        used.is_empty(),
        "neither name resolves to an id, so neither is in use: {used:?}"
    );

    let mut rng = rng_yielding_7f3a_then_01();
    let id = allocate_deferred_work_id_in_with_rng(root, &storage, &mut rng, None).unwrap();
    assert_eq!(dw_hash(id), "7f3a");
}

/// The `Result` these entry points gained is not decoration: a failing store surfaces as `Err`
/// rather than as a silently narrower id space. Every other test here ends in `.unwrap()`, so
/// without this one the error arm the signature change was made for is never executed.
#[test]
fn test_hex_allocation_propagates_a_store_failure() {
    let tmp = tempdir().unwrap();
    let storage = qdev_core::StorageConfig::default();

    let store = SqliteStore::open_in_memory().unwrap();
    store
        .with_conn(|conn| {
            conn.execute_batch("DROP TABLE entities;").unwrap();
            Ok(())
        })
        .unwrap();

    let mut rng = rng_yielding_7f3a_then_01();
    let err = allocate_deferred_work_id_in_with_rng(tmp.path(), &storage, &mut rng, Some(&store))
        .expect_err("a store that cannot list entities cannot answer what is in use");
    assert_eq!(err.code(), "sqlite_error");
}
