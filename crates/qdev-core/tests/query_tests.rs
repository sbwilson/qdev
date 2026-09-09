use qdev_core::query::{query_entity, query_list, GetResult, ListQueryOptions, QueryOptions};
use qdev_core::schema::EntityKind;
use qdev_core::store::{
    ConstraintRecord, EntityRecord, RelationRecord, ScratchpadRecord, SprintAssignmentRecord,
    SprintRecord, SqliteStore, Store,
};
use qdev_core::write::Author;
use qdev_core::ExitCode;

fn base_entity(id: &str, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind,
        title: Some(format!("{} title", id)),
        status: Some("draft".to_string()),
        owners: None,
        source_path: format!("docs/specs/{}.md", id),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-09-08T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

fn story_entity(id: &str, epic_id: &str, seq: u32) -> EntityRecord {
    EntityRecord {
        epic_id: Some(epic_id.to_string()),
        seq: Some(seq),
        ..base_entity(id, EntityKind::Story)
    }
}

/// Sets up the exact fixture from the cli-reference.md `get` JSON envelope example: story
/// E12S4 under epic E12, an own no_go constraint, an inherited rabbit_hole constraint from the
/// epic, and depends_on/traces_to/governed_by relations.
fn setup_e12s4(store: &SqliteStore) {
    store
        .upsert_entity(&EntityRecord {
            status: Some("active".to_string()),
            ..base_entity("E12", EntityKind::Epic)
        })
        .unwrap();

    store
        .upsert_entity(&EntityRecord {
            title: Some("CoreResponse Buffer Layout".to_string()),
            status: Some("ready".to_string()),
            owners: Some(r#"["simon","team:core-platform"]"#.to_string()),
            version: 3,
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(r#"["bridge","foundation"]"#.to_string()),
            ..story_entity("E12S4", "E12", 4)
        })
        .unwrap();

    store
        .upsert_entity(&EntityRecord {
            status: Some("done".to_string()),
            ..story_entity("E12S3", "E12", 3)
        })
        .unwrap();

    store
        .upsert_constraint(&ConstraintRecord {
            id: "E12S4/NG-1".to_string(),
            owner_id: "E12S4".to_string(),
            kind: "no_go".to_string(),
            text: "Do not implement Swift decoding".to_string(),
        })
        .unwrap();
    store
        .upsert_constraint(&ConstraintRecord {
            id: "E12/RH-2".to_string(),
            owner_id: "E12".to_string(),
            kind: "rabbit_hole".to_string(),
            text: "len == 0 does not mean empty result".to_string(),
        })
        .unwrap();

    for (relation, target) in [
        ("depends_on", "E12S3"),
        ("traces_to", "FR-102"),
        ("governed_by", "AD-43"),
    ] {
        store
            .upsert_relation(&RelationRecord {
                source_id: "E12S4".to_string(),
                relation: relation.to_string(),
                target_id: target.to_string(),
            })
            .unwrap();
    }
}

#[test]
fn test_get_story_default_projection_matches_reference_shape() {
    let store = SqliteStore::open_in_memory().unwrap();
    setup_e12s4(&store);

    let result = query_entity(
        &store,
        Some(EntityKind::Story),
        "E12S4",
        &QueryOptions::default(),
    )
    .unwrap();

    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };

    assert_eq!(projection.id, "E12S4");
    assert_eq!(projection.kind, EntityKind::Story);
    assert_eq!(projection.epic_id.as_deref(), Some("E12"));
    assert_eq!(
        projection.title.as_deref(),
        Some("CoreResponse Buffer Layout")
    );
    assert_eq!(projection.status.as_deref(), Some("ready"));
    assert!(!projection.blocked, "E12S3 is done, so E12S4 is unblocked");
    assert_eq!(projection.appetite.as_deref(), Some("small"));
    assert_eq!(projection.safety_class.as_deref(), Some("ClassB"));
    assert_eq!(
        projection.owners,
        vec!["simon".to_string(), "team:core-platform".to_string()]
    );
    assert_eq!(
        projection.target_modules,
        Some(vec!["bridge".to_string(), "foundation".to_string()])
    );
    assert_eq!(projection.version, 3);
    assert!(
        projection.scratch.is_none(),
        "scratch must be absent without --expand scratch"
    );

    // Own constraint first, then the epic-inherited one, tagged with inherited_from.
    assert_eq!(projection.constraints.len(), 2);
    assert_eq!(projection.constraints[0].id, "E12S4/NG-1");
    assert_eq!(projection.constraints[0].kind, "no_go");
    assert!(projection.constraints[0].inherited_from.is_none());
    assert_eq!(projection.constraints[1].id, "E12/RH-2");
    assert_eq!(projection.constraints[1].kind, "rabbit_hole");
    assert_eq!(
        projection.constraints[1].inherited_from.as_deref(),
        Some("E12")
    );

    // Relations grouped by relation name in an ordered map.
    assert_eq!(
        projection.relations.get("depends_on"),
        Some(&vec!["E12S3".to_string()])
    );
    assert_eq!(
        projection.relations.get("traces_to"),
        Some(&vec!["FR-102".to_string()])
    );
    assert_eq!(
        projection.relations.get("governed_by"),
        Some(&vec!["AD-43".to_string()])
    );
}

#[test]
fn test_get_json_is_byte_identical_across_repeated_runs() {
    let store = SqliteStore::open_in_memory().unwrap();
    setup_e12s4(&store);

    let opts = QueryOptions::default();
    let first = query_entity(&store, Some(EntityKind::Story), "E12S4", &opts).unwrap();
    let second = query_entity(&store, Some(EntityKind::Story), "E12S4", &opts).unwrap();

    let (GetResult::Entity(p1), GetResult::Entity(p2)) = (first, second) else {
        panic!("expected entity projections");
    };

    let json1 = serde_json::to_string(&p1).unwrap();
    let json2 = serde_json::to_string(&p2).unwrap();
    assert_eq!(json1, json2);
}

#[test]
fn test_expand_scratch_only_adds_scratch_field() {
    let store = SqliteStore::open_in_memory().unwrap();
    setup_e12s4(&store);
    store
        .upsert_scratchpad_entry(&ScratchpadRecord {
            story_id: "E12S4".to_string(),
            seq: 1,
            at: "2026-09-08T00:00:00Z".to_string(),
            author_type: Some("human".to_string()),
            author_id: Some("simon".to_string()),
            kind: Some("note".to_string()),
            text: Some("Investigated the buffer layout".to_string()),
        })
        .unwrap();

    let without_expand = match query_entity(
        &store,
        Some(EntityKind::Story),
        "E12S4",
        &QueryOptions::default(),
    )
    .unwrap()
    {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };

    let with_expand = match query_entity(
        &store,
        Some(EntityKind::Story),
        "E12S4",
        &QueryOptions {
            expand_scratch: true,
        },
    )
    .unwrap()
    {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };

    assert!(without_expand.scratch.is_none());
    let scratch = with_expand
        .scratch
        .clone()
        .expect("scratch must be present");
    assert_eq!(scratch.len(), 1);
    assert_eq!(
        scratch[0].text.as_deref(),
        Some("Investigated the buffer layout")
    );

    // Nothing else changes.
    let mut with_expand_cleared = with_expand;
    with_expand_cleared.scratch = None;
    assert_eq!(without_expand, with_expand_cleared);
}

#[test]
fn test_blocked_true_when_dependency_not_done() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&story_entity("E20S1", "E20", 1))
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            status: Some("in-progress".to_string()),
            ..story_entity("E20S2", "E20", 2)
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E20S1".to_string(),
            relation: "depends_on".to_string(),
            target_id: "E20S2".to_string(),
        })
        .unwrap();

    let result = query_entity(&store, None, "E20S1", &QueryOptions::default()).unwrap();
    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };
    assert!(projection.blocked);
}

#[test]
fn test_blocked_true_when_dependency_target_is_dangling() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&story_entity("E20S3", "E20", 3))
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E20S3".to_string(),
            relation: "depends_on".to_string(),
            target_id: "E20S99".to_string(),
        })
        .unwrap();

    let result = query_entity(&store, None, "E20S3", &QueryOptions::default()).unwrap();
    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };
    assert!(
        projection.blocked,
        "a dangling depends_on target has no cached status, which isn't 'done'"
    );
}

#[test]
fn test_bare_id_resolves_via_primary_key_without_kind_grammar() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&EntityRecord {
            status: Some("accepted".to_string()),
            ..base_entity("AD-43", EntityKind::Adr)
        })
        .unwrap();

    let result = query_entity(&store, None, "AD-43", &QueryOptions::default()).unwrap();
    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };
    assert_eq!(projection.id, "AD-43");
    assert_eq!(projection.kind, EntityKind::Adr);
}

#[test]
fn test_kind_mismatch_is_usage_error_exit_2() {
    let store = SqliteStore::open_in_memory().unwrap();
    setup_e12s4(&store);

    let err = query_entity(
        &store,
        Some(EntityKind::Epic),
        "E12S4",
        &QueryOptions::default(),
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_unknown_id_is_entity_not_found_exit_2() {
    let store = SqliteStore::open_in_memory().unwrap();

    let err = query_entity(&store, None, "E99S1", &QueryOptions::default()).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "entity_not_found");
}

#[test]
fn test_slash_id_resolves_as_constraint_regardless_of_kind_hint() {
    let store = SqliteStore::open_in_memory().unwrap();
    setup_e12s4(&store);

    let result = query_entity(&store, None, "E12S4/NG-1", &QueryOptions::default()).unwrap();
    let constraint = match result {
        GetResult::Constraint(c) => c,
        GetResult::Entity(_) => panic!("expected a constraint result"),
    };
    assert_eq!(constraint.owner_id, "E12S4");
    assert_eq!(constraint.kind, "no_go");
    assert_eq!(constraint.text, "Do not implement Swift decoding");
}

#[test]
fn test_unknown_constraint_id_is_entity_not_found() {
    let store = SqliteStore::open_in_memory().unwrap();

    let err = query_entity(&store, None, "E12S4/NG-9", &QueryOptions::default()).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "entity_not_found");
}

#[test]
fn test_list_filters_apply_as_and_and_order_by_id() {
    let store = SqliteStore::open_in_memory().unwrap();

    store
        .upsert_entity(&EntityRecord {
            status: Some("ready".to_string()),
            owners: Some(r#"["simon"]"#.to_string()),
            target_modules: Some(r#"["bridge"]"#.to_string()),
            ..story_entity("E12S9", "E12", 9)
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            status: Some("ready".to_string()),
            owners: Some(r#"["simon"]"#.to_string()),
            target_modules: Some(r#"["bridge"]"#.to_string()),
            ..story_entity("E12S4", "E12", 4)
        })
        .unwrap();
    // Different epic: excluded by --epic E12.
    store
        .upsert_entity(&EntityRecord {
            status: Some("ready".to_string()),
            owners: Some(r#"["simon"]"#.to_string()),
            target_modules: Some(r#"["bridge"]"#.to_string()),
            ..story_entity("E13S1", "E13", 1)
        })
        .unwrap();
    // Different status: excluded by --status ready.
    store
        .upsert_entity(&EntityRecord {
            status: Some("draft".to_string()),
            owners: Some(r#"["simon"]"#.to_string()),
            target_modules: Some(r#"["bridge"]"#.to_string()),
            ..story_entity("E12S5", "E12", 5)
        })
        .unwrap();

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.epic_id = Some("E12".to_string());
    opts.status = Some("ready".to_string());
    opts.owner = Some("simon".to_string());
    opts.module = Some("bridge".to_string());

    let rows = query_list(&store, &opts).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    // Ordered by id ascending: "E12S4" < "E12S9" lexicographically.
    assert_eq!(ids, vec!["E12S4", "E12S9"]);
}

#[test]
fn test_list_owner_filter_avoids_partial_name_collision() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&EntityRecord {
            owners: Some(r#"["sim"]"#.to_string()),
            ..story_entity("E14S1", "E14", 1)
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            owners: Some(r#"["simon"]"#.to_string()),
            ..story_entity("E14S2", "E14", 2)
        })
        .unwrap();

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.owner = Some("sim".to_string());

    let rows = query_list(&store, &opts).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["E14S1"],
        "'sim' must not match the unrelated owner 'simon'"
    );
}

#[test]
fn test_list_owner_filter_treats_wildcard_characters_literally() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&EntityRecord {
            owners: Some(r#"["j_smith@corp.com"]"#.to_string()),
            ..story_entity("E14S3", "E14", 3)
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            // Differs from the filter value only by the character SQL LIKE would treat as a
            // single-char wildcard ('_' matches any one character, including 'X').
            owners: Some(r#"["jXsmith@corp.com"]"#.to_string()),
            ..story_entity("E14S4", "E14", 4)
        })
        .unwrap();

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.owner = Some("j_smith@corp.com".to_string());

    let rows = query_list(&store, &opts).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["E14S3"],
        "'_' in the filter value must match literally, not as a SQL LIKE wildcard"
    );
}

#[test]
fn test_list_owner_filter_is_case_sensitive() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&EntityRecord {
            owners: Some(r#"["sally"]"#.to_string()),
            ..story_entity("E14S5", "E14", 5)
        })
        .unwrap();

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.owner = Some("Sally".to_string());

    let rows = query_list(&store, &opts).unwrap();
    assert!(
        rows.is_empty(),
        "'Sally' must not match the differently-cased stored owner 'sally'"
    );
}

#[test]
fn test_get_and_list_surface_stale_flag() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&EntityRecord {
            stale: true,
            ..story_entity("E17S1", "E17", 1)
        })
        .unwrap();

    let result = query_entity(&store, None, "E17S1", &QueryOptions::default()).unwrap();
    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };
    assert!(projection.stale);

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.epic_id = Some("E17".to_string());
    let rows = query_list(&store, &opts).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].stale);
}

#[test]
fn test_list_sprint_filter() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&story_entity("E15S1", "E15", 1))
        .unwrap();
    store
        .upsert_entity(&story_entity("E15S2", "E15", 2))
        .unwrap();
    store
        .upsert_sprint(&SprintRecord {
            id: 5,
            status: Some("active".to_string()),
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_sprint_assignment(&SprintAssignmentRecord {
            sprint_id: 5,
            story_id: "E15S1".to_string(),
            assigned_at: "2026-09-08T00:00:00Z".to_string(),
            carried_from: None,
        })
        .unwrap();

    let mut opts = ListQueryOptions::new(EntityKind::Story);
    opts.sprint = Some(5);

    let rows = query_list(&store, &opts).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["E15S1"]);
}

/// `blocked` is a story-level notion, and `payload-story.json` documents it as such. Hydration
/// keeps an out-of-band `depends_on` row on another kind (it records an `invalid_relation_kind`
/// finding rather than dropping the row), so without an explicit kind gate a non-story carrying
/// one would report `blocked: true` and contradict the schema every consumer reads.
#[test]
fn test_blocked_stays_false_for_a_non_story_with_a_depends_on_row() {
    let store = SqliteStore::open_in_memory().unwrap();

    let epic = base_entity("E12", EntityKind::Epic);
    store.upsert_entity(&epic).unwrap();

    // A blocking target: present, and not `done`.
    let mut target = base_entity("E12S1", EntityKind::Story);
    target.status = Some("draft".to_string());
    store.upsert_entity(&target).unwrap();

    // The out-of-band edge hydration would have kept while flagging it.
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12".to_string(),
            relation: "depends_on".to_string(),
            target_id: "E12S1".to_string(),
        })
        .unwrap();

    let result = query_entity(
        &store,
        Some(EntityKind::Epic),
        "E12",
        &QueryOptions::default(),
    )
    .unwrap();
    let projection = match result {
        GetResult::Entity(p) => *p,
        GetResult::Constraint(_) => panic!("expected an entity projection"),
    };

    assert_eq!(projection.kind, EntityKind::Epic);
    assert!(
        !projection.blocked,
        "blocked must stay false for a non-story, whatever relations it carries"
    );
    assert!(
        projection.relations.contains_key("depends_on"),
        "the row itself is still reported; only `blocked` is gated"
    );
}
