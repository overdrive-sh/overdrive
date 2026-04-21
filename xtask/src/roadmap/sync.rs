//! Sync orchestration — parse roadmap, ensure labels, create issues in
//! two passes, update state file. Pragmatic one-off utility.

use eyre::{Result, bail, eyre};

use super::{gh, parse, state};

/// CLI-facing options for the sync run.
#[derive(Debug, Clone)]
pub struct SyncOpts {
    pub repo: String,
    pub project_number: u64,
    /// If true, make actual `gh` calls. When false (the safe default), we
    /// only print the plan.
    pub commit: bool,
    pub limit: Option<usize>,
    pub phase: Option<u8>,
    pub resume: bool,
    pub roadmap_file: std::path::PathBuf,
    pub workspace_root: std::path::PathBuf,
}

/// `owner/repo` -> owner.
fn repo_owner(repo: &str) -> Result<&str> {
    repo.split('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| eyre!("--repo must look like owner/name, got {repo:?}"))
}

/// The 12 area labels + 6 type labels we need to exist in the repo.
fn required_labels() -> Vec<Label> {
    const AREAS: &[&str] = &[
        "control-plane",
        "dataplane",
        "storage",
        "security",
        "observability",
        "gateway",
        "drivers",
        "os",
        "sdk",
        "cli",
        "testing",
        "ci",
    ];
    const TYPES: &[&str] =
        &["primitive", "integration", "migration", "sdk", "hardening", "research"];
    let mut out: Vec<Label> = AREAS
        .iter()
        .map(|a| Label {
            name: format!("area/{a}"),
            color: "1d76db".into(),
            description: format!("Area: {a}"),
        })
        .collect();
    out.extend(TYPES.iter().map(|t| Label {
        name: format!("type/{t}"),
        color: "5319e7".into(),
        description: format!("Type: {t}"),
    }));
    out
}

#[derive(Debug, Clone)]
struct Label {
    name: String,
    color: String,
    description: String,
}

pub fn sync(opts: &SyncOpts) -> Result<()> {
    eprintln!(
        "xtask roadmap sync: repo={}, project={}, dry_run={}, phase={:?}, limit={:?}, resume={}",
        opts.repo, opts.project_number, !opts.commit, opts.phase, opts.limit, opts.resume
    );

    // 1. Parse roadmap.
    let rows = parse::parse_file(&opts.roadmap_file)?;
    eprintln!(
        "xtask roadmap sync: parsed {} rows from {}",
        rows.len(),
        opts.roadmap_file.display()
    );

    // 2. Pre-flight: gh auth (only if we're going to commit, but also
    //    useful in dry-run to fail early).
    if opts.commit {
        gh::check_auth()?;
    } else {
        // Soft check — print warning but continue.
        if let Err(e) = gh::check_auth() {
            eprintln!("xtask roadmap sync: [dry-run] warning: gh check failed: {e}");
            eprintln!("xtask roadmap sync: [dry-run] continuing; would fail with --commit");
        }
    }

    let owner = repo_owner(&opts.repo)?;

    // 3. Label bootstrap.
    ensure_labels(opts, &required_labels())?;

    // 4. Project validation — look up field IDs for "Phase" and "Depends on".
    let (project_id, phase_field, depends_field) = if opts.commit {
        let fields = gh::project_field_list(owner, opts.project_number)?;
        let phase_field = fields
            .fields
            .nodes
            .iter()
            .find(|f| f.name().eq_ignore_ascii_case("Phase"))
            .ok_or_else(|| {
                eyre!(
                    "Project v2 #{} under {owner} has no 'Phase' field. Create it as a \
                     single-select with options phase-1 … phase-7.",
                    opts.project_number
                )
            })?
            .clone_single_select()?;
        let depends_field = fields
            .fields
            .nodes
            .iter()
            .find(|f| f.name().eq_ignore_ascii_case("Depends on"))
            .ok_or_else(|| {
                eyre!(
                    "Project v2 #{} under {owner} has no 'Depends on' field. Create it as a \
                     text field.",
                    opts.project_number
                )
            })?
            .clone_text()?;
        let project_id = gh::project_node_id(owner, opts.project_number)?;
        (Some(project_id), Some(phase_field), Some(depends_field))
    } else {
        eprintln!(
            "xtask roadmap sync: [dry-run] skipping Project v2 validation \
             (would verify #{} under {owner})",
            opts.project_number
        );
        (None, None, None)
    };

    // 5. Filter rows by phase / limit.
    let filtered: Vec<&parse::RoadmapRow> = rows
        .iter()
        .filter(|r| opts.phase.is_none_or(|p| r.phase == p))
        .take(opts.limit.unwrap_or(usize::MAX))
        .collect();
    eprintln!("xtask roadmap sync: {} rows after phase/limit filter", filtered.len());

    // 6. Build known-ID set once (for dependency resolution).
    let all_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();

    // 7. Load state (for --resume).
    let mut state = if opts.resume {
        state::SyncState::load(&opts.workspace_root)?
    } else {
        state::SyncState::default()
    };

    // --- Pass 1: create issues -----------------------------------------
    let mut created_in_run = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for row in &filtered {
        if state.issue_for(&row.id).is_some() {
            skipped += 1;
            eprintln!(
                "[{}] → #{} (skipped — already in state)",
                row.id,
                state.issue_for(&row.id).unwrap().issue_number
            );
            continue;
        }

        let title = format!("[{}] {}", row.id, row.title);
        let body = render_body_with_unresolved_deps(row, &all_ids);
        let labels = labels_for(row);

        if !opts.commit {
            eprintln!(
                "[{}] → (dry-run) would create issue: {}\n         labels=[{}]",
                row.id,
                title,
                labels.join(", ")
            );
            continue;
        }

        match gh::create_issue(&opts.repo, &title, &body, &labels) {
            Ok((number, url)) => {
                let record = state::CreatedIssue {
                    issue_number: number,
                    url: url.clone(),
                    created_at: now_rfc3339(),
                    body_finalized: false,
                };
                state.record(row.id.clone(), record);
                // Persist incrementally so a rate-limit mid-run doesn't lose state.
                if let Err(e) = state.save(&opts.workspace_root) {
                    eprintln!("[{}] → #{number} (created but state save failed: {e})", row.id);
                } else {
                    eprintln!("[{}] → #{number} (created)", row.id);
                }

                // Attach to Project + set Phase field inline so we don't have to
                // walk the list twice.
                if let (Some(pid), Some(pf)) = (&project_id, &phase_field) {
                    if let Err(e) = attach_to_project(owner, opts, pid, pf, &url, row.phase) {
                        eprintln!("[{}] → #{number} (project attach failed: {e})", row.id);
                    }
                }
                created_in_run += 1;
            }
            Err(e) => {
                failed += 1;
                eprintln!("[{}] → FAILED: {e}", row.id);
            }
        }
    }

    // --- Pass 2: resolve `Depends on` references to real #N numbers ----
    if opts.commit {
        for row in &filtered {
            let Some(issue) = state.issue_for(&row.id).cloned() else { continue };
            if issue.body_finalized {
                continue;
            }
            let deps = parse::extract_dependencies(row, &all_ids);
            // If no deps map to created issues, we can still mark the body
            // final (the "None" placeholder is already accurate).
            let resolved: Vec<(String, u64, String)> = deps
                .iter()
                .filter_map(|d| {
                    state.issue_for(d).map(|i| {
                        let title = rows
                            .iter()
                            .find(|r| &r.id == d)
                            .map(|r| r.title.clone())
                            .unwrap_or_default();
                        (d.clone(), i.issue_number, title)
                    })
                })
                .collect();
            let body = render_body_resolved(row, &resolved);
            match gh::edit_issue_body(&opts.repo, issue.issue_number, &body) {
                Ok(()) => {
                    let mut updated = issue;
                    updated.body_finalized = true;
                    state.record(row.id.clone(), updated);
                    let _ = state.save(&opts.workspace_root);

                    // Also set the "Depends on" project field, if configured.
                    if let (Some(pid), Some(df)) = (&project_id, &depends_field) {
                        let depends_text = if resolved.is_empty() {
                            String::new()
                        } else {
                            resolved
                                .iter()
                                .map(|(_, n, _)| format!("#{n}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        // Find the item ID by URL. Cheap path: re-add is
                        // idempotent and returns the existing item ID.
                        if let Ok(item_id) = gh::project_item_add(
                            owner,
                            opts.project_number,
                            &state.issue_for(&row.id).unwrap().url,
                        ) {
                            let _ =
                                gh::project_field_set_text(pid, &item_id, &df.id, &depends_text);
                        }
                    }
                    eprintln!(
                        "[{}] → #{} (body resolved)",
                        row.id,
                        state.issue_for(&row.id).unwrap().issue_number
                    );
                }
                Err(e) => eprintln!("[{}] → body resolve FAILED: {e}", row.id),
            }
        }
    } else {
        // Dry-run: preview the resolved-body shape using roadmap IDs.
        for row in &filtered {
            let deps = parse::extract_dependencies(row, &all_ids);
            if deps.is_empty() {
                continue;
            }
            eprintln!("[{}] (dry-run) depends on: {}", row.id, deps.join(", "));
        }
    }

    eprintln!(
        "xtask roadmap sync: total={}, created={}, skipped={}, failed={}",
        filtered.len(),
        created_in_run,
        skipped,
        failed
    );
    Ok(())
}

fn ensure_labels(opts: &SyncOpts, labels: &[Label]) -> Result<()> {
    for label in labels {
        if !opts.commit {
            // Cheap path in dry-run: we don't want to error out if the repo
            // is inaccessible; just describe what we'd do.
            eprintln!("[label] {} (dry-run — would create if missing)", label.name);
            continue;
        }
        match gh::label_exists(&opts.repo, &label.name) {
            Ok(true) => eprintln!("[label] {} (exists)", label.name),
            Ok(false) => {
                gh::create_label(&opts.repo, &label.name, &label.color, &label.description)?;
                eprintln!("[label] {} (created)", label.name);
            }
            Err(e) => bail!("label check failed for {}: {e}", label.name),
        }
    }
    Ok(())
}

fn attach_to_project(
    owner: &str,
    opts: &SyncOpts,
    project_id: &str,
    phase_field: &SingleSelectField,
    issue_url: &str,
    phase: u8,
) -> Result<()> {
    let item_id = gh::project_item_add(owner, opts.project_number, issue_url)?;
    let option_name = format!("phase-{phase}");
    let Some(opt) = phase_field.options.iter().find(|o| o.name == option_name) else {
        bail!(
            "project Phase field has no option '{option_name}' (found: {})",
            phase_field.options.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(", ")
        );
    };
    gh::project_field_set_single_select(
        owner,
        opts.project_number,
        project_id,
        &item_id,
        &phase_field.id,
        &opt.id,
    )
}

/// Area + type labels for a row.
fn labels_for(row: &parse::RoadmapRow) -> Vec<String> {
    let mut out: Vec<String> = row.areas.iter().map(|a| format!("area/{a}")).collect();
    out.push(format!("type/{}", row.r#type));
    out
}

fn render_body_with_unresolved_deps(row: &parse::RoadmapRow, all_ids: &[String]) -> String {
    // Pass 1: render with placeholder references. Pass 2 will overwrite with
    // real issue numbers. Plain-text roadmap IDs are fine for the placeholder.
    let deps = parse::extract_dependencies(row, all_ids);
    let deps_block = if deps.is_empty() {
        "None".to_string()
    } else {
        deps.iter().map(|d| format!("- roadmap id {d} (resolving)")).collect::<Vec<_>>().join("\n")
    };
    body_template(row, &deps_block)
}

fn render_body_resolved(row: &parse::RoadmapRow, resolved: &[(String, u64, String)]) -> String {
    let deps_block = if resolved.is_empty() {
        "None".to_string()
    } else {
        resolved
            .iter()
            .map(|(roadmap_id, number, title)| {
                format!("- #{number} — {title} (roadmap id {roadmap_id})")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    body_template(row, &deps_block)
}

fn body_template(row: &parse::RoadmapRow, deps_block: &str) -> String {
    let notes = if row.notes.is_empty() { "—" } else { &row.notes };
    format!(
        "## Summary\n{title}\n\n\
         ## Source\nWhitepaper {source} — https://github.com/overdrive-sh/overdrive/blob/main/docs/whitepaper.md\n\n\
         ## Notes\n{notes}\n\n\
         ## Depends on\n{deps_block}\n\n\
         ## Acceptance\n<!-- TODO(assignee): add ≤3 acceptance bullets before picking this up -->\n",
        title = row.title,
        source = row.source,
        notes = notes,
        deps_block = deps_block
    )
}

// RFC3339 timestamp. One-off script — hand-format rather than pull in
// `time`'s formatting feature just for this.
fn now_rfc3339() -> String {
    let dt = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

// --- Local helpers for the field-type discriminator ----------------------
// The real `gh project field-list` JSON has per-type shapes; we normalise
// them into these small structs at the sync-layer boundary.

#[derive(Debug, Clone)]
struct SingleSelectField {
    id: String,
    options: Vec<gh::ProjectFieldOption>,
}

#[derive(Debug, Clone)]
struct TextField {
    id: String,
}

trait ProjectFieldExt {
    fn clone_single_select(&self) -> Result<SingleSelectField>;
    fn clone_text(&self) -> Result<TextField>;
}

impl ProjectFieldExt for gh::ProjectField {
    fn clone_single_select(&self) -> Result<SingleSelectField> {
        match self {
            Self::SingleSelect { id, options, .. } => {
                Ok(SingleSelectField { id: id.clone(), options: options.clone() })
            }
            Self::Other { name, .. } => bail!("field '{name}' is not a single-select"),
        }
    }
    fn clone_text(&self) -> Result<TextField> {
        // `gh project field-list` currently returns text fields under the
        // untagged `Other` arm — we only need the id for item-edit calls.
        match self {
            Self::Other { id, .. } | Self::SingleSelect { id, .. } => {
                Ok(TextField { id: id.clone() })
            }
        }
    }
}
