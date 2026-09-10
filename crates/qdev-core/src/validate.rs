//! `qdev validate` (spec-1-11): aggregates hydration-recorded findings with five freshly
//! computed integrity checks, `--changed` filtering against a git merge-base diff, and the
//! pure logic backing the guided `--fix-ids` duplicate-planning-id renumber.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use crate::config::Config;
use crate::errors::QdevError;
use crate::id::Identifier;
use crate::schema::{extract_frontmatter, EntityKind};
use crate::store::sqlite::collect_markdown_files;
use crate::store::{EntityFilter, EntityPresence, FindingRecord, Store};
use crate::write::current_iso8601;

const ERROR_SEVERITY: &str = "error";
/// Severity of `entity_file_off_convention`. Deliberately not `error`: a workspace that was
/// legal before the identity-seam story shipped must not start failing `qdev validate`, and
/// `has_error_finding` gates the exit code on `error` alone.
const WARNING_SEVERITY: &str = "warning";

/// The result of scanning every directory hydration reads for frontmatter `id` collisions: every
/// planning id that is declared by two or more files (sorted paths), plus the full set of ids in
/// use (needed by `--fix-ids` to allocate a renumbered id that collides with nothing else in the
/// workspace).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DuplicateIdScan {
    /// `(id, sorted paths declaring it)`, sorted by id, for every id **declared** 2+ times.
    /// Only frontmatter declarations count as duplicates: two files can carry one id in their
    /// names without either declaring it, and that is an off-convention name, not a collision.
    pub groups: Vec<(String, Vec<String>)>,
    /// The filesystem half of the in-use id set: every id any scanned file *declares* in its
    /// frontmatter, unioned with every id a scanned file *name* carries. Both are ways a
    /// workspace already owns an id — and a file whose frontmatter will not parse still
    /// occupies its name. Use [`ids_in_use`] rather than this field directly unless the cache
    /// is genuinely unavailable.
    pub all_ids: HashSet<String>,
}

/// Scans every markdown file in every directory hydration reads — `specs_dir` **and**
/// `state_dir`, recursively, the same set `rebuild_from_workspace` and `sweep_workspace` walk —
/// reading each file's frontmatter `id` directly from disk (never from the cache): the
/// `entities.id` PRIMARY KEY means a second file declaring an already-hydrated id leaves no
/// trace in the cache to detect against, so this check must re-read the files.
///
/// Scanning only `specs_dir` made every collision among sprints, deferred work, decisions,
/// releases and SOUP invisible — and an invisible collision is what turns the natural response
/// to a duplicate (delete the copy) into a purge of the surviving entity.
pub fn scan_duplicate_planning_ids(
    workspace_root: &Path,
    storage: &crate::config::StorageConfig,
) -> Result<DuplicateIdScan, QdevError> {
    let mut files = Vec::new();
    collect_markdown_files(&workspace_root.join(&storage.specs_dir), &mut files);
    collect_markdown_files(&workspace_root.join(&storage.state_dir), &mut files);
    // One directory configured inside the other would otherwise report every file in the
    // overlap as a duplicate of itself.
    files.sort();
    files.dedup();

    let mut by_id: HashMap<String, Vec<String>> = HashMap::new();
    // Ids carried by file names, collected alongside the declared ones. A name is scanned even
    // when the file cannot be read or its frontmatter cannot be parsed, which is exactly the
    // case the declared half misses: the file still occupies its name, so allocating that id
    // would produce a `file_exists` refusal for an id the user never chose.
    let mut carried_ids: HashSet<String> = HashSet::new();
    for file in &files {
        if let Some(name) = file.file_name().and_then(|n| n.to_str()) {
            if let Some(id) = crate::write::id_carried_by_filename(name) {
                carried_ids.insert(id);
            }
        }
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let frontmatter = match extract_frontmatter(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(id) = frontmatter.get("id").and_then(|v| v.as_str()) {
            let rel = file
                .strip_prefix(workspace_root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            by_id.entry(id.to_string()).or_default().push(rel);
        }
    }

    let mut all_ids: HashSet<String> = by_id.keys().cloned().collect();
    all_ids.extend(carried_ids);
    let mut groups: Vec<(String, Vec<String>)> = by_id
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(id, mut paths)| {
            paths.sort();
            (id, paths)
        })
        .collect();
    groups.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(DuplicateIdScan { groups, all_ids })
}

/// `duplicate_planning_id`: 2+ files under `storage.specs_dir` or `storage.state_dir` declare the
/// same frontmatter `id`. One finding per participating path, naming the other path(s) in its
/// message.
pub fn find_duplicate_planning_ids(
    workspace_root: &Path,
    storage: &crate::config::StorageConfig,
) -> Result<Vec<FindingRecord>, QdevError> {
    let scan = scan_duplicate_planning_ids(workspace_root, storage)?;
    let found_at = current_iso8601();
    let mut findings = Vec::new();
    for (id, paths) in &scan.groups {
        for path in paths {
            let others: Vec<&str> = paths
                .iter()
                .filter(|p| *p != path)
                .map(|s| s.as_str())
                .collect();
            findings.push(FindingRecord {
                path: path.clone(),
                code: "duplicate_planning_id".to_string(),
                severity: ERROR_SEVERITY.to_string(),
                message: Some(format!(
                    "Duplicate planning id '{}' is also declared in {}",
                    id,
                    others.join(", ")
                )),
                found_at: found_at.clone(),
            });
        }
    }
    Ok(findings)
}

/// **The one authority on which ids a workspace already owns.** Every caller that allocates an
/// id asks this for the id space: `qdev create story`
/// ([`crate::id::allocate_next_story_id_in`]), `qdev validate --fix-ids`, and the `DW-`/`DEC-`
/// hex allocators ([`crate::id::allocate_deferred_work_id_in_with_rng`],
/// [`crate::id::allocate_decision_id_in_with_rng`]). They then allocate differently, and
/// deliberately: `--fix-ids` takes the lowest free number via [`next_available_id`], and the hex
/// pair stay random with length growth. **None of them keeps a scan of its own** — the hex pair
/// did until `spec-hex-allocators-and-invariant-coverage.md`, and it disagreed with this set on
/// five separate axes. Do not add a fourth answer here.
///
/// Membership is compared verbatim by the `HashSet`. The hex allocators lowercase both sides
/// before testing, because a hex hash is canonically lowercase while the declared half below
/// contributes whatever the frontmatter says. The story allocator instead parses each member as
/// an [`crate::id::Identifier`], which is case-sensitive, so a mis-cased planning id (`id: e1s7`)
/// is in this set but not in its numbering — such a file is a schema violation in its own right.
///
/// The answer is a union of three memberships, because each is a way a workspace can already own
/// an id:
///
/// 1. **Declared** — the frontmatter `id` of any file under `specs_dir` or `state_dir`,
///    recursively, matching the `.md` extension case-insensitively: exactly the set hydration
///    reads (`collect_markdown_files`), so an id qdev can read is an id qdev considers taken.
/// 2. **Carried** — the id any of those file *names* carries ([`crate::write::id_carried_by_filename`]).
///    A file whose frontmatter will not parse declares nothing but still occupies its name, and
///    `create_story`'s own occupancy check would refuse the create with `file_exists` for an id
///    the user never chose.
/// 3. **Cached** — every id the store holds for a hydrated entity, so an id survives its file
///    becoming unreadable. The cache is a union *member*, not the source: `store` is `None`
///    outside an initialised workspace, where `qdev create story` must still work from the
///    filesystem alone.
///
/// Four components used to answer "does this id already exist?" and the allocator answered
/// differently — one flat directory, case-sensitively, filenames only — so `create story` handed
/// out ids other files already declared. This is that one answer.
///
/// It answers *ownership*, not reservation: two allocations that resolve this set before either
/// writes its file get the same id, and **there is no reservation protocol**. An earlier version
/// of this comment claimed none was needed "while every allocator's caller writes under the
/// advisory write lock"; the pass-3 acceptance gate showed that is false. The callers *write*
/// under the lock but *allocate* outside it — `handle_create_story` allocates before
/// `create_story` takes the lock, and `--fix-ids` plans every renumber before its write phase,
/// deliberately, so the lock is not held across the confirmation prompt. What bounds the damage
/// is that the loser's occupancy check runs inside the lock, so the expected outcome is a
/// spurious `file_exists` rather than two entities sharing an id — expected, not demonstrated.
/// Filed in `deferred-work.md`; do not rebuild a safety argument here.
pub fn ids_in_use(
    workspace_root: &Path,
    storage: &crate::config::StorageConfig,
    store: Option<&dyn Store>,
) -> Result<HashSet<String>, QdevError> {
    let scan = scan_duplicate_planning_ids(workspace_root, storage)?;
    ids_in_use_from_scan(&scan, store)
}

/// [`ids_in_use`] for a caller that has already run [`scan_duplicate_planning_ids`] — the same
/// answer, without walking both trees a second time. `--fix-ids` needs the scan's `groups` as
/// well as its ids, so it uses this rather than paying for the walk twice.
pub fn ids_in_use_from_scan(
    scan: &DuplicateIdScan,
    store: Option<&dyn Store>,
) -> Result<HashSet<String>, QdevError> {
    let mut used = scan.all_ids.clone();
    if let Some(store) = store {
        // Stale rows count, so this deliberately uses the unfiltered `list_entities` rather
        // than the derivation helper: a stale row is a *hydrated* entity whose last parse
        // failed, and its id is precisely the one the filesystem half can no longer see. This
        // asks about *id ownership*, not about whether a finding may be derived — a stale row's
        // id is still taken, and filtering here would hand it out to a second entity.
        used.extend(
            store
                .list_entities(&EntityFilter::default())?
                .into_iter()
                .map(|entity| entity.id),
        );
    }
    Ok(used)
}

/// The source path to report a deferred-work finding against.
///
/// Uses the unfiltered [`Store::get_entity`] deliberately: this asks *where to report*, not
/// whether the entity counts as present, and a row's `source_path` is the right path whatever its
/// staleness. In practice both callers skip a stale DW row before reaching here, so this only
/// ever sees `Live` or a missing row — the unfiltered read matters for the missing case, where a
/// `deferred_work` row without an `entities` row is itself a broken cache and the finding must
/// still be reported against a named placeholder rather than dropped.
fn deferred_work_path(store: &dyn Store, dw_id: &str) -> Result<String, QdevError> {
    Ok(match store.get_entity(dw_id)? {
        Some(entity) => entity.source_path,
        None => format!("(unknown path for {})", dw_id),
    })
}

/// True when the entity row backing a `deferred_work` row is retained *stale* — its file failed
/// its last parse, so every cached field describes content the file may no longer have. The two
/// deferred-work checks below skip such rows: the file already carries the `merge_conflict`,
/// `schema_violation` or `read_error` finding that names the actionable problem, and a second
/// finding derived from pre-edit content sends the reader looking for something they have
/// already deleted. A full rebuild has no stale rows at all, so this is also what makes the two
/// hydration paths agree on what `qdev validate` reports.
///
/// A row the cache has no entity for at all is *not* skipped: that is a broken cache rather than
/// a known-unparseable file, and dropping the finding would let `qdev validate` exit 0 on it.
/// That is the whole reason this asks [`Store::entity_presence_for_derivation`] for the
/// three-state answer instead of the boolean: `Stale` and `Absent` mean different things here,
/// and the rule that a stale row is absent still lives in one place.
fn deferred_work_is_stale(store: &dyn Store, dw_id: &str) -> Result<bool, QdevError> {
    Ok(store.entity_presence_for_derivation(dw_id)? == EntityPresence::Stale)
}

/// `orphan_deferred_work`: DW's `origin_story_id` is set but no entity exists for it — asked of
/// [`Store::entity_exists_for_derivation`], so a *stale* origin story counts as absent and the
/// finding is reported. It has to: a rebuild of the same tree has no row for that story at all
/// and reports the orphan, so a sweep that treated the retained row as present would make
/// `qdev validate`'s answer depend on hydration history.
pub fn find_orphan_deferred_work(store: &dyn Store) -> Result<Vec<FindingRecord>, QdevError> {
    let found_at = current_iso8601();
    let mut findings = Vec::new();
    for dw in store.list_deferred_work()? {
        if deferred_work_is_stale(store, &dw.id)? {
            continue;
        }
        let origin = match dw.origin_story_id.as_deref().map(str::trim) {
            Some(o) if !o.is_empty() => o,
            _ => continue,
        };
        if store.entity_exists_for_derivation(origin)? {
            continue;
        }
        // One message for both a stale and an absent origin, deliberately. Naming the difference
        // would read better — a stale story *is* on disk, and the parse failure reported against
        // it is the actionable finding — but the message is part of the finding, and a sweep
        // (which has the stale row) would then describe the same workspace differently from a
        // rebuild (which has no row at all). Convergence is the stronger property, and the
        // convergence test caught the attempt.
        findings.push(FindingRecord {
            path: deferred_work_path(store, &dw.id)?,
            code: "orphan_deferred_work".to_string(),
            severity: ERROR_SEVERITY.to_string(),
            message: Some(format!(
                "Deferred work '{}' references origin_story_id '{}', which does not exist",
                dw.id, origin
            )),
            found_at: found_at.clone(),
        });
    }
    Ok(findings)
}

/// `dw_missing_rationale`: DW `safety_risk` in `{acceptable_with_mitigation, unacceptable}` and
/// `rationale` is null or empty.
pub fn find_dw_missing_rationale(store: &dyn Store) -> Result<Vec<FindingRecord>, QdevError> {
    let found_at = current_iso8601();
    let mut findings = Vec::new();
    for dw in store.list_deferred_work()? {
        if deferred_work_is_stale(store, &dw.id)? {
            continue;
        }
        let risk = dw.safety_risk.as_deref().unwrap_or("");
        if risk != "acceptable_with_mitigation" && risk != "unacceptable" {
            continue;
        }
        let has_rationale = dw
            .rationale
            .as_deref()
            .map(|r| !r.trim().is_empty())
            .unwrap_or(false);
        if has_rationale {
            continue;
        }
        {
            findings.push(FindingRecord {
                path: deferred_work_path(store, &dw.id)?,
                code: "dw_missing_rationale".to_string(),
                severity: ERROR_SEVERITY.to_string(),
                message: Some(format!(
                    "Deferred work '{}' has safety_risk '{}' but no rationale",
                    dw.id, risk
                )),
                found_at: found_at.clone(),
            });
        }
    }
    Ok(findings)
}

/// `target_module_not_registered`: a story's `target_modules` entry is absent from
/// `config.modules[].id`. One finding per (story, unregistered module) pair.
pub fn find_unregistered_target_modules(
    store: &dyn Store,
    config: &Config,
) -> Result<Vec<FindingRecord>, QdevError> {
    let registered: HashSet<&str> = config.modules.iter().map(|m| m.id.as_str()).collect();
    let found_at = current_iso8601();
    let mut findings = Vec::new();

    let filter = EntityFilter {
        kind: Some(EntityKind::Story),
        ..Default::default()
    };
    for story in store.list_entities(&filter)? {
        // A stale row's `target_modules` is pre-edit content: the file's own parse failure is
        // already reported, and a rebuild of the same tree has no such row to derive from. The
        // rule is the store's (`EntityPresence`), asked of the row already in hand rather than
        // via `entity_exists_for_derivation`, which would cost one query per listed row.
        if !story.exists_for_derivation() {
            continue;
        }
        let modules: Vec<String> = story
            .target_modules
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default();
        for module in modules {
            if registered.contains(module.as_str()) {
                continue;
            }
            findings.push(FindingRecord {
                path: story.source_path.clone(),
                code: "target_module_not_registered".to_string(),
                severity: ERROR_SEVERITY.to_string(),
                message: Some(format!(
                    "Story '{}' targets module '{}', which is not registered in config.modules",
                    story.id, module
                )),
                found_at: found_at.clone(),
            });
        }
    }
    Ok(findings)
}

/// `entity_file_off_convention`: a hydrated entity's file breaks the identity convention every
/// writer resolves by (see `write::resolve_entity_file`) — its name does not carry its
/// frontmatter `id`, or it lives outside every standard entity directory. Such a file is
/// readable (hydration walks the spec and state trees recursively) but the write path cannot
/// resolve it, which is how an entity `qdev get` returns comes back "not found" from
/// `qdev update`. Reporting it makes the convention enforceable rather than assumed.
///
/// `warning` severity by design: files like this were legal until the convention was stated, so
/// the finding must not flip `qdev validate` to exit 1 on a workspace that was previously clean.
///
/// The *directory* half is checked against the whole standard set rather than the one directory
/// for the entity's kind, because that is exactly what the write path accepts: its cross-kind
/// fallback resolves an id in any standard directory, so a `kind:` that disagrees with its
/// directory is not itself an identity problem. The expected name reported names the directory
/// for the entity's kind, which is where a new file of that kind belongs.
pub fn find_off_convention_entity_files(
    store: &dyn Store,
    storage: &crate::config::StorageConfig,
) -> Result<Vec<FindingRecord>, QdevError> {
    let standard_dirs: HashSet<String> = EntityKind::all()
        .iter()
        .map(|kind| normalize_rel(&crate::write::directory_for_kind(Some(storage), *kind)))
        .collect();
    let found_at = current_iso8601();
    let mut findings = Vec::new();

    for entity in store.list_entities(&EntityFilter::default())? {
        // Stale rows are excluded for the same reason as the other computed checks: the id this
        // convention is checked against comes from a parse that is known to be out of date.
        // Same store-owned rule as above, asked of the row in hand.
        if !entity.exists_for_derivation() {
            continue;
        }
        let rel = entity.source_path.replace('\\', "/");
        // The convention covers markdown entity files; evidence JSON and scratchpad JSONL are
        // named for their run and their story, not for an entity id.
        //
        // The extension is matched case-insensitively, exactly as `collect_markdown_files` does:
        // a `.MD` file is hydrated like any other, so judging only `.md` names silenced the one
        // signal that would have warned about it — and a `.MD` name is *not* one the write path
        // resolves (`filename_carries_id` requires `.md`), so it is the very case the warning
        // exists for.
        if !Path::new(&rel)
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            continue;
        }
        let (dir, file_name) = match rel.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), rel.clone()),
        };

        let name_carries_id = crate::write::filename_carries_id(&file_name, &entity.id);
        let dir_is_standard = standard_dirs.contains(&dir);
        if name_carries_id && dir_is_standard {
            continue;
        }

        let expected = format!(
            "{}/{}",
            normalize_rel(&crate::write::directory_for_kind(
                Some(storage),
                entity.kind
            )),
            crate::write::canonical_file_name(&entity.id)
        );
        let reason = match (name_carries_id, dir_is_standard) {
            (false, true) => "its name does not carry that id",
            (true, false) => "it is outside every standard entity directory",
            _ => {
                "its name does not carry that id and it is outside every standard entity directory"
            }
        };
        findings.push(FindingRecord {
            path: rel.clone(),
            code: "entity_file_off_convention".to_string(),
            severity: WARNING_SEVERITY.to_string(),
            message: Some(format!(
                "'{}' holds entity '{}' but {}; the write path resolves entities by file name, so {} it to '{}' (a '-slug' or '_slug' suffix after the id is allowed)",
                rel,
                entity.id,
                reason,
                if dir_is_standard { "rename" } else { "move" },
                expected
            )),
            found_at: found_at.clone(),
        });
    }
    Ok(findings)
}

/// A workspace-relative path as a `/`-separated string, with any trailing separator removed, so
/// configured storage directories and cached `source_path` values compare as written.
fn normalize_rel(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

/// Runs every hydration-derived finding (`Store::list_findings`, reused verbatim) plus the five
/// freshly computed checks above, merged into one flat list. Cache-native findings are never
/// duplicated: this only ever reads from `list_findings`, never writes back to the `findings`
/// table.
///
/// The four checks that read cached rows treat a *stale* row as absent, so no finding is ever
/// derived from content a file no longer has — and none derived from an entity a stale row
/// merely *refers to* either, since the existence probes go through
/// [`Store::entity_exists_for_derivation`] rather than `get_entity`. One rule, one helper: a
/// sweep and a full rebuild of the same tree report the same list. Reads are deliberately
/// untouched: `qdev get` and `qdev list` still return a stale entity with its `stale` flag set,
/// which is what the retention exists for.
pub fn run_validation(
    store: &dyn Store,
    workspace_root: &Path,
    config: &Config,
) -> Result<Vec<FindingRecord>, QdevError> {
    let mut findings = store.list_findings()?;
    findings.extend(find_duplicate_planning_ids(
        workspace_root,
        &config.storage,
    )?);
    findings.extend(find_orphan_deferred_work(store)?);
    findings.extend(find_dw_missing_rationale(store)?);
    findings.extend(find_unregistered_target_modules(store, config)?);
    findings.extend(find_off_convention_entity_files(store, &config.storage)?);
    Ok(findings)
}

/// Shells out to `git merge-base HEAD <integration_branch>` then `git diff --name-only
/// <merge-base>`, mirroring `resolve_git_email`'s `std::process::Command` usage. Unlike
/// `resolve_git_email`, failures are surfaced (not swallowed): a missing git binary or a
/// directory that isn't a git repository is an `InfrastructureFailure`, per spec.
pub fn git_changed_files(
    workspace_root: &Path,
    integration_branch: &str,
) -> Result<HashSet<String>, QdevError> {
    let merge_base_output = Command::new("git")
        .args(["merge-base", "HEAD", integration_branch])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git merge-base': {}", e),
            )
        })?;

    if !merge_base_output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_merge_base_failed",
            format!(
                "'git merge-base HEAD {}' failed: {}",
                integration_branch,
                String::from_utf8_lossy(&merge_base_output.stderr).trim()
            ),
        ));
    }

    let merge_base = String::from_utf8_lossy(&merge_base_output.stdout)
        .trim()
        .to_string();
    if merge_base.is_empty() {
        return Err(QdevError::infrastructure_failure(
            "git_merge_base_failed",
            "'git merge-base' returned no commit",
        ));
    }

    // `--relative` makes git report paths relative to the working directory (the qdev
    // workspace root) rather than to the repository root, and restricts the diff to that
    // subtree — without it, a workspace living in a subdirectory of its repository gets paths
    // that can never match a finding's workspace-relative `path`, and `--changed` silently
    // reports nothing. `core.quotePath=false` stops git octal-escaping non-ASCII paths, which
    // would likewise never match.
    let diff_output = Command::new("git")
        .args([
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-only",
            "--relative",
            &merge_base,
        ])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git diff': {}", e),
            )
        })?;

    if !diff_output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_diff_failed",
            format!(
                "'git diff --name-only --relative {}' failed: {}",
                merge_base,
                String::from_utf8_lossy(&diff_output.stderr).trim()
            ),
        ));
    }

    let mut paths: HashSet<String> = String::from_utf8_lossy(&diff_output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    // `git diff --name-only` only reports tracked-file changes, so a file that was just
    // created and not yet `git add`ed would otherwise never appear in `--changed` — the most
    // common real trigger for a fresh `duplicate_planning_id`.
    // `git ls-files` already reports paths relative to the working directory, so this only
    // needs the quoting disabled to line up with the diff output above.
    let untracked_output = Command::new("git")
        .args([
            "-c",
            "core.quotePath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git ls-files': {}", e),
            )
        })?;

    if !untracked_output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_ls_files_failed",
            format!(
                "'git ls-files --others --exclude-standard' failed: {}",
                String::from_utf8_lossy(&untracked_output.stderr).trim()
            ),
        ));
    }

    paths.extend(
        String::from_utf8_lossy(&untracked_output.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string),
    );

    Ok(paths)
}

/// Keeps only findings whose `path` is in `changed_paths`, except `dependency_cycle` findings:
/// hydration writes one row per participant path but all rows for the same cycle share an
/// identical message (`"depends_on cycle: A -> B -> ... -> A"`), so those rows are grouped by
/// message and kept or dropped as a whole group if *any* participant path is in the changed
/// set — never showing a partial cycle.
pub fn filter_by_changed(
    findings: Vec<FindingRecord>,
    changed_paths: &HashSet<String>,
) -> Vec<FindingRecord> {
    let mut cycle_groups: HashMap<String, Vec<FindingRecord>> = HashMap::new();
    let mut kept: Vec<FindingRecord> = Vec::new();

    for finding in findings {
        if finding.code == "dependency_cycle" {
            // A message-less cycle row cannot be grouped with any other row, so it gets a key
            // unique to its own path: grouping every message-less row together would merge two
            // unrelated cycles and report an untouched one as changed.
            let key = match finding.message.clone() {
                Some(message) => message,
                None => format!("\u{0}nomsg\u{0}{}", finding.path),
            };
            cycle_groups.entry(key).or_default().push(finding);
        } else if changed_paths.contains(&finding.path) {
            kept.push(finding);
        }
    }

    for (_, group) in cycle_groups {
        if group.iter().any(|f| changed_paths.contains(&f.path)) {
            kept.extend(group);
        }
    }

    sort_findings(&mut kept);
    kept
}

/// Total order over findings, so `qdev validate --json` is byte-identical across runs even when
/// one path holds several findings of the same code. `(path, code)` alone is not a total order:
/// since cache schema v3 a path can carry two `dangling_relation` rows, and the merge of cached
/// and freshly computed findings can also produce same-key pairs. Every field of the record is
/// in the key, so only genuinely identical findings compare equal.
pub fn sort_findings(findings: &mut [FindingRecord]) {
    findings.sort_by(|a, b| {
        (
            a.path.as_str(),
            a.code.as_str(),
            a.message.as_deref().unwrap_or(""),
            a.severity.as_str(),
            a.found_at.as_str(),
        )
            .cmp(&(
                b.path.as_str(),
                b.code.as_str(),
                b.message.as_deref().unwrap_or(""),
                b.severity.as_str(),
                b.found_at.as_str(),
            ))
    });
}

/// Returns `true` iff any finding is `error`-severity, the exit-code decision rule shared by
/// `qdev validate`'s plain and `--changed`-filtered runs.
pub fn has_error_finding(findings: &[FindingRecord]) -> bool {
    findings.iter().any(|f| f.severity == ERROR_SEVERITY)
}

/// Computes the next available identifier of the same kind (and, for a `Story`, the same epic)
/// as `old`, skipping every id already present in `used_ids`. Only the sequentially numbered
/// planning kinds that live under `specs_dir` are supported (Epic, Story, ADR, FR, NFR, Hazard,
/// PRD); a collision among the state kinds the scan now also covers is reported by
/// `duplicate_planning_id` and rejected here, so `--fix-ids` records it as skipped rather than
/// renumbering a kind whose ids are not sequential.
pub fn next_available_id(
    old: &Identifier,
    used_ids: &HashSet<String>,
) -> Result<Identifier, QdevError> {
    let mut n: u32 = 1;
    loop {
        let candidate = match old {
            Identifier::Epic { .. } => Identifier::Epic { number: n },
            Identifier::Story { epic, .. } => Identifier::Story {
                epic: *epic,
                story: n,
            },
            Identifier::Adr { .. } => Identifier::Adr { number: n },
            Identifier::FunctionalRequirement { .. } => {
                Identifier::FunctionalRequirement { number: n }
            }
            Identifier::NonFunctionalRequirement { .. } => {
                Identifier::NonFunctionalRequirement { number: n }
            }
            Identifier::Hazard { .. } => Identifier::Hazard { number: n },
            Identifier::Prd { .. } => Identifier::Prd { number: n },
            other => {
                return Err(QdevError::logical_failure(
                    "unsupported_renumber",
                    format!(
                        "'--fix-ids' does not support renumbering identifier kind '{}'",
                        other.kind()
                    ),
                ))
            }
        };
        if !used_ids.contains(&candidate.to_string()) {
            return Ok(candidate);
        }
        n = n.checked_add(1).ok_or_else(|| {
            QdevError::logical_failure("id_space_exhausted", "Ran out of ids to allocate")
        })?;
    }
}

/// Line-based rewrite of a frontmatter document's top-level `id:` value only. `id` is a managed
/// field everywhere else in the write path (`patch_frontmatter` rejects it via custom fields),
/// so `--fix-ids` needs this dedicated, narrowly scoped primitive rather than reusing that path.
/// Every other line — including the frontmatter delimiters, comments, and key order — is left
/// byte-for-byte untouched.
pub fn rewrite_frontmatter_id(content: &str, new_id: &str) -> Result<String, QdevError> {
    let all_lines: Vec<&str> = content.split_inclusive('\n').collect();
    if all_lines.is_empty() {
        return Err(QdevError::logical_failure(
            "missing_frontmatter",
            "File content is empty",
        ));
    }

    let mut open_idx = None;
    for (idx, &line) in all_lines.iter().enumerate() {
        let line_no_eol = line.trim_end_matches(['\r', '\n']);
        let stripped = line_no_eol.strip_prefix('\u{feff}').unwrap_or(line_no_eol);
        if stripped == "---" {
            open_idx = Some(idx);
            break;
        } else if stripped.trim().is_empty() {
            continue;
        } else {
            return Err(QdevError::logical_failure(
                "missing_frontmatter",
                "Missing opening frontmatter delimiter '---' at start of document",
            ));
        }
    }
    let open_line = open_idx.ok_or_else(|| {
        QdevError::logical_failure(
            "missing_frontmatter",
            "Missing opening frontmatter delimiter '---'",
        )
    })?;

    // Only a delimiter at column 0 closes the frontmatter. A `---` inside a block scalar is
    // necessarily indented past its key, so it cannot be mistaken for one.
    let mut close_idx = None;
    for (idx, &line) in all_lines.iter().enumerate().skip(open_line + 1) {
        let line_no_eol = line.trim_end_matches(['\r', '\n']);
        if line_no_eol == "---" || line_no_eol == "..." {
            close_idx = Some(idx);
            break;
        }
    }
    let close_line = close_idx.ok_or_else(|| {
        QdevError::logical_failure(
            "unclosed_frontmatter",
            "Unclosed frontmatter delimiter '---'",
        )
    })?;

    let mut found = false;
    let mut result_lines: Vec<String> = Vec::new();
    for &line in &all_lines[(open_line + 1)..close_line] {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if !found && !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
            if let Some((key, value)) = trimmed.split_once(':') {
                if key.trim() == "id" {
                    // Take the line ending from the id line itself, not from the file as a
                    // whole: deriving it from any CRLF anywhere would rewrite this one line
                    // with CRLF in an otherwise-LF file, for pure diff noise.
                    let newline = if line.ends_with("\r\n") {
                        "\r\n"
                    } else if line.ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    // Preserve a trailing inline comment on the id line, if any, so a
                    // renumber doesn't silently discard it.
                    let trailing_comment =
                        inline_comment_start(value).map(|idx| value[idx..].trim_end());
                    let new_line = match trailing_comment {
                        Some(comment) => format!("id: {} {}{}", new_id, comment, newline),
                        None => format!("id: {}{}", new_id, newline),
                    };
                    result_lines.push(new_line);
                    found = true;
                    continue;
                }
            }
        }
        result_lines.push(line.to_string());
    }

    if !found {
        return Err(QdevError::logical_failure(
            "missing_field",
            "Frontmatter has no top-level 'id' field",
        ));
    }

    let mut final_content = String::new();
    for &line in &all_lines[..=open_line] {
        final_content.push_str(line);
    }
    for line in result_lines {
        final_content.push_str(&line);
    }
    for &line in &all_lines[close_line..] {
        final_content.push_str(line);
    }
    Ok(final_content)
}

/// Index at which an inline `#` comment starts in a frontmatter scalar value, or `None`. Only a
/// `#` that is outside any quoted scalar and preceded by whitespace opens a YAML comment: in
/// `id: "AD-1#2"` the `#` is part of the value, and treating it as a comment would re-emit half
/// the old value as a comment on the rewritten line.
fn inline_comment_start(value: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_is_space = true;
    let mut escaped = false;
    for (idx, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            prev_is_space = false;
            continue;
        }
        match ch {
            // Only double-quoted YAML scalars use backslash escapes; inside them a `\"` is a
            // literal quote and must not flip the quote state.
            '\\' if in_double => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && prev_is_space => return Some(idx),
            _ => {}
        }
        prev_is_space = ch.is_whitespace();
    }
    None
}

/// Rewrites citations in `content` matching `pattern` from `old_id` to `new_id`,
/// touching only occurrences whose captured id (the pattern's first capture group) is exactly
/// `old_id` — never every match of the citation pattern. Returns the rewritten content and the
/// number of citations changed.
pub fn rewrite_citations(
    content: &str,
    pattern: &regex::Regex,
    old_id: &str,
    new_id: &str,
) -> (String, usize) {
    let mut count = 0usize;
    let rewritten = pattern.replace_all(content, |caps: &regex::Captures| {
        let (whole, whole_start) = match caps.get(0) {
            Some(m) => (m.as_str(), m.start()),
            None => return String::new(),
        };
        let id_match = match caps.get(1) {
            Some(m) if m.as_str() == old_id => m,
            _ => return whole.to_string(),
        };
        count += 1;
        // Substitute the id inside whatever delimiters the configured pattern matched rather
        // than emitting a fixed `[NEWID]`: `config.hygiene.citation_pattern` is configurable,
        // and a non-bracket citation syntax must survive the renumber intact.
        let start = id_match.start() - whole_start;
        let end = start + id_match.as_str().len();
        format!("{}{}{}", &whole[..start], new_id, &whole[end..])
    });
    (rewritten.into_owned(), count)
}

/// Translates a simple glob (`*` = any run of non-separator characters, `**` = any run including
/// separators, `?` = one character, everything else literal) into an anchored regex, then tests
/// `rel_path` (workspace-relative, `/`-separated) against it. Module path globs have no prior
/// matcher in qdev (spec-1-11 is the first place `config.modules[].paths` is matched against
/// real paths rather than merely stored), so this is a minimal, dependency-free implementation
/// rather than pulling in a glob crate for one call site.
pub fn glob_match(pattern: &str, rel_path: &str) -> bool {
    // A pattern with no glob metacharacters names a directory (or a single file): match the
    // path itself and everything beneath it. Without this, the natural config value
    // `paths = ["crates/qdev-core"]` is fully anchored and matches no file at all, so
    // `--fix-ids` silently rewrites zero citations and reports that as "none to rewrite".
    if !pattern.contains(['*', '?']) {
        // A trailing slash is a natural way to write a directory, and must not change the
        // meaning — `glob_literal_prefix` trims one too, so pruning and matching stay in step.
        let dir = pattern.trim_end_matches('/');
        return rel_path == dir || rel_path.starts_with(&format!("{}/", dir));
    }

    let mut regex_str = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && i + 2 < chars.len() && chars[i + 1] == '*' && chars[i + 2] == '*' {
            // "/**" matches either nothing (the directory itself) or "/" plus anything below it,
            // so "dir/**" covers both "dir" and "dir/anything/at/any/depth".
            regex_str.push_str("(?:/.*)?");
            i += 3;
        } else if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            regex_str.push_str(".*");
            i += 2;
        } else if c == '*' {
            regex_str.push_str("[^/]*");
            i += 1;
        } else if c == '?' {
            regex_str.push_str("[^/]");
            i += 1;
        } else {
            regex_str.push_str(&regex::escape(&c.to_string()));
            i += 1;
        }
    }
    regex_str.push('$');
    match regex::Regex::new(&regex_str) {
        Ok(re) => re.is_match(rel_path),
        Err(_) => false,
    }
}

/// The union of every configured module's `paths` globs, in declaration order with duplicates
/// removed by first occurrence — the set `--fix-ids` rewrites citations under. Ties the module
/// registry and the citation-pattern config together for the first time (spec-1-11).
pub fn module_path_patterns(config: &Config) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut patterns = Vec::new();
    for module in &config.modules {
        for path in &module.paths {
            if seen.insert(path.clone()) {
                patterns.push(path.clone());
            }
        }
    }
    patterns
}

/// The leading literal directory prefix of a glob — everything before the first `*` or `?`,
/// truncated at the last `/`. `"crates/*/src/**"` yields `"crates"`; a pattern with no
/// metacharacters is entirely literal.
fn glob_literal_prefix(pattern: &str) -> &str {
    match pattern.find(['*', '?']) {
        Some(idx) => match pattern[..idx].rfind('/') {
            Some(slash) => &pattern[..slash],
            None => "",
        },
        None => pattern.trim_end_matches('/'),
    }
}

/// Whether any file under `dir_rel` could match one of `patterns` — a sound over-approximation
/// used to prune the walk. Comparing literal prefixes never prunes a directory that could hold
/// a match, and an empty `patterns` list means "no filtering", so everything is descended.
fn could_contain_match(patterns: &[String], dir_rel: &str) -> bool {
    if patterns.is_empty() || dir_rel.is_empty() {
        return true;
    }
    patterns.iter().any(|pattern| {
        let prefix = glob_literal_prefix(pattern);
        prefix.is_empty()
            || prefix == dir_rel
            || prefix.starts_with(&format!("{}/", dir_rel))
            || dir_rel.starts_with(&format!("{}/", prefix))
    })
}

/// Lists every regular file under `dir` (workspace-relative paths, `/`-separated), skipping
/// `.git`, `.qdev`, and symlinked directories so a workspace walk never follows out of the
/// workspace or touches the gitignored cache, and pruned to the directories that could contain
/// a match for one of `patterns` (empty = no pruning).
///
/// Pruning happens *during* the walk rather than as a filter afterwards: collecting first means
/// enumerating `target/` and `node_modules/` in full before discarding them, which dominates
/// the runtime of `--fix-ids` in any repo with build output.
///
/// The walk is iterative — a worklist rather than recursion — so a deeply nested tree cannot
/// overflow the stack part-way through a renumber.
pub fn collect_workspace_files_matching(
    workspace_root: &Path,
    dir: &Path,
    patterns: &[String],
    out: &mut Vec<String>,
) {
    const SKIP_DIRS: [&str; 2] = [".git", ".qdev"];
    let appended_from = out.len();
    let mut worklist: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];

    let relative_to_root = |path: &Path| -> String {
        path.strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    };

    while let Some(current) = worklist.pop() {
        if !current.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&current) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if SKIP_DIRS.contains(&name) {
                        continue;
                    }
                }
                if !could_contain_match(patterns, &relative_to_root(&path)) {
                    continue;
                }
                worklist.push(path);
            } else if path.is_file() {
                out.push(relative_to_root(&path));
            }
        }
    }

    // `read_dir` order is filesystem-dependent and the worklist visits siblings in reverse
    // discovery order, so the paths this call appends are sorted to make the result
    // deterministic. Only this call's own entries are touched: a caller accumulating into a
    // shared vector keeps whatever ordering it had already established.
    out[appended_from..].sort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{DeferredWorkRecord, EntityRecord, SqliteStore};
    use crate::write::Author;
    use std::fs;
    use tempfile::TempDir;

    fn make_entity(id: &str, kind: EntityKind, source_path: &str) -> EntityRecord {
        EntityRecord {
            id: id.to_string(),
            kind,
            title: None,
            status: None,
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

    #[test]
    fn test_scan_duplicate_planning_ids_finds_collisions() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let dir = root.join("docs/specs/stories");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "---\nid: E1S2\ntitle: A\n---\n").unwrap();
        fs::write(dir.join("b.md"), "---\nid: E1S2\ntitle: B\n---\n").unwrap();
        fs::write(dir.join("c.md"), "---\nid: E1S3\ntitle: C\n---\n").unwrap();

        let storage = crate::config::StorageConfig::default();
        let scan = scan_duplicate_planning_ids(root, &storage).unwrap();
        assert_eq!(scan.groups.len(), 1);
        assert_eq!(scan.groups[0].0, "E1S2");
        assert_eq!(scan.groups[0].1.len(), 2);
        assert!(scan.all_ids.contains("E1S3"));

        let findings = find_duplicate_planning_ids(root, &storage).unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.code == "duplicate_planning_id"));
        assert!(findings.iter().all(|f| f.severity == "error"));
    }

    /// Duplicate-id detection must cover every directory hydration reads, not `specs_dir` alone:
    /// an unreported collision in `state_dir` is the precondition for a purge that erases the
    /// surviving entity.
    #[test]
    fn test_scan_duplicate_planning_ids_covers_state_dir() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let sprints = root.join("docs/state/sprints");
        fs::create_dir_all(&sprints).unwrap();
        fs::write(
            sprints.join("sprint-1.md"),
            "---\nid: sprint-1\ntitle: A\n---\n",
        )
        .unwrap();
        fs::write(
            sprints.join("sprint-1-copy.md"),
            "---\nid: sprint-1\ntitle: B\n---\n",
        )
        .unwrap();

        let storage = crate::config::StorageConfig::default();
        let scan = scan_duplicate_planning_ids(root, &storage).unwrap();
        assert_eq!(
            scan.groups.len(),
            1,
            "a collision in state_dir must be reported: {:?}",
            scan.groups
        );
        assert_eq!(scan.groups[0].0, "sprint-1");
        assert_eq!(
            scan.groups[0].1,
            vec![
                "docs/state/sprints/sprint-1-copy.md".to_string(),
                "docs/state/sprints/sprint-1.md".to_string()
            ]
        );

        let findings = find_duplicate_planning_ids(root, &storage).unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|f| f.code == "duplicate_planning_id"));
    }

    #[test]
    fn test_find_orphan_deferred_work() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .upsert_entity(&make_entity(
                "DW-a1b2",
                EntityKind::DeferredWork,
                "docs/state/dw/DW-a1b2.md",
            ))
            .unwrap();
        store
            .upsert_deferred_work(&DeferredWorkRecord {
                id: "DW-a1b2".to_string(),
                origin_story_id: Some("E9S9".to_string()),
                target_module: "core".to_string(),
                status: Some("open".to_string()),
                safety_risk: Some("negligible".to_string()),
                rationale: None,
                gate: None,
                resolution: None,
            })
            .unwrap();

        let findings = find_orphan_deferred_work(&store).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "orphan_deferred_work");
        assert_eq!(findings[0].path, "docs/state/dw/DW-a1b2.md");
    }

    #[test]
    fn test_find_dw_missing_rationale() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .upsert_entity(&make_entity(
                "DW-c3d4",
                EntityKind::DeferredWork,
                "docs/state/dw/DW-c3d4.md",
            ))
            .unwrap();
        store
            .upsert_deferred_work(&DeferredWorkRecord {
                id: "DW-c3d4".to_string(),
                origin_story_id: None,
                target_module: "core".to_string(),
                status: Some("open".to_string()),
                safety_risk: Some("unacceptable".to_string()),
                rationale: Some("   ".to_string()),
                gate: None,
                resolution: None,
            })
            .unwrap();

        let findings = find_dw_missing_rationale(&store).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "dw_missing_rationale");
    }

    #[test]
    fn test_find_unregistered_target_modules() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut story = make_entity("E1S1", EntityKind::Story, "docs/specs/stories/E1S1.md");
        story.target_modules = Some(r#"["ghost","core"]"#.to_string());
        story.epic_id = Some("E1".to_string());
        story.seq = Some(1);
        store.upsert_entity(&story).unwrap();

        let mut config = Config::default();
        config.modules = vec![crate::config::ModuleConfig {
            id: "core".to_string(),
            paths: vec!["crates/core/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        }];

        let findings = find_unregistered_target_modules(&store, &config).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.as_deref().unwrap().contains("ghost"));
    }

    #[test]
    fn test_filter_by_changed_keeps_whole_cycle_group() {
        let findings = vec![
            FindingRecord {
                path: "a.md".to_string(),
                code: "dependency_cycle".to_string(),
                severity: "error".to_string(),
                message: Some("depends_on cycle: A -> B -> A".to_string()),
                found_at: "t".to_string(),
            },
            FindingRecord {
                path: "b.md".to_string(),
                code: "dependency_cycle".to_string(),
                severity: "error".to_string(),
                message: Some("depends_on cycle: A -> B -> A".to_string()),
                found_at: "t".to_string(),
            },
            FindingRecord {
                path: "c.md".to_string(),
                code: "duplicate_planning_id".to_string(),
                severity: "error".to_string(),
                message: None,
                found_at: "t".to_string(),
            },
        ];
        let mut changed = HashSet::new();
        changed.insert("b.md".to_string());
        let filtered = filter_by_changed(findings, &changed);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|f| f.path != "c.md"));
    }

    #[test]
    fn test_filter_by_changed_drops_findings_outside_diff() {
        let findings = vec![FindingRecord {
            path: "a.md".to_string(),
            code: "duplicate_planning_id".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "t".to_string(),
        }];
        let filtered = filter_by_changed(findings, &HashSet::new());
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_next_available_id_skips_used() {
        let old: Identifier = "E1S2".parse().unwrap();
        let mut used = HashSet::new();
        used.insert("E1S1".to_string());
        used.insert("E1S2".to_string());
        used.insert("E1S3".to_string());
        let next = next_available_id(&old, &used).unwrap();
        assert_eq!(next.to_string(), "E1S4");
    }

    #[test]
    fn test_next_available_id_rejects_unsupported_kind() {
        let old: Identifier = "DW-abcd".parse().unwrap();
        let used = HashSet::new();
        assert!(next_available_id(&old, &used).is_err());
    }

    #[test]
    fn test_rewrite_frontmatter_id_preserves_everything_else() {
        let content = "---\nid: E1S2\ntitle: \"Hello\"\n# a comment\nstatus: draft\n---\n\nBody [E1S2] text.\n";
        let rewritten = rewrite_frontmatter_id(content, "E1S9").unwrap();
        assert!(rewritten.contains("id: E1S9"));
        assert!(rewritten.contains("title: \"Hello\""));
        assert!(rewritten.contains("# a comment"));
        assert!(rewritten.contains("status: draft"));
        // Body citations are untouched by this narrowly scoped primitive.
        assert!(rewritten.contains("Body [E1S2] text."));
    }

    #[test]
    fn test_rewrite_citations_only_touches_exact_id() {
        let pattern = regex::Regex::new(crate::config::DEFAULT_CITATION_PATTERN).unwrap();
        let content = "See [E1S2] and [E1S20] and [E1S2/NG-1].";
        let (rewritten, count) = rewrite_citations(content, &pattern, "E1S2", "E1S9");
        assert_eq!(count, 1);
        assert_eq!(rewritten, "See [E1S9] and [E1S20] and [E1S2/NG-1].");
    }

    #[test]
    fn test_glob_match_double_star() {
        assert!(glob_match("crates/core/**", "crates/core/src/lib.rs"));
        assert!(glob_match("crates/core/**", "crates/core"));
        assert!(!glob_match("crates/core/**", "crates/other/src/lib.rs"));
        assert!(glob_match("*.md", "readme.md"));
        assert!(!glob_match("*.md", "docs/readme.md"));
    }

    #[test]
    fn test_has_error_finding() {
        let findings = vec![FindingRecord {
            path: "a.md".to_string(),
            code: "x".to_string(),
            severity: "warning".to_string(),
            message: None,
            found_at: "t".to_string(),
        }];
        assert!(!has_error_finding(&findings));
    }
}
