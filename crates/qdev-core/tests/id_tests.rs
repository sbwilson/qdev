use std::fs;

use proptest::prelude::*;
use qdev_core::{
    allocate_decision_id, allocate_decision_id_with_rng, allocate_deferred_work_id,
    allocate_deferred_work_id_with_rng, allocate_next_story_id, ConstraintKind, ConstraintOwner,
    HygieneConfig, IdParseError, Identifier, IdentifierKind, DEFAULT_CITATION_PATTERN,
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
    let id = allocate_deferred_work_id(tmp.path());
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
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng);
    assert_eq!(
        id,
        Identifier::DeferredWork {
            hash: "123456".to_string()
        }
    );

    // Create collision for 6 chars: "DW-123456.md"
    fs::write(dw_dir.join("DW-123456.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng);
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
    let id = allocate_decision_id(tmp.path());
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
    let id = allocate_decision_id_with_rng(tmp.path(), &mut mock_rng);
    assert_eq!(
        id,
        Identifier::Decision {
            hash: "abcdef".to_string()
        }
    );

    // Create collision for 6 chars: "DEC-abcdef.md"
    fs::write(dec_dir.join("DEC-abcdef.md"), "").unwrap();

    let mut mock_rng = MockRng::new(bytes);
    let id = allocate_decision_id_with_rng(tmp.path(), &mut mock_rng);
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
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng);
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
    let id = allocate_deferred_work_id_with_rng(tmp.path(), &mut mock_rng);
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
