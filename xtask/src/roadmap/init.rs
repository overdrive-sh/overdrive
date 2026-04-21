//! `cargo xtask roadmap init` — one-shot scaffold for the roadmap
//! Project v2 + required labels. Safe to re-run; dry-run is the default.
//!
//! Pragmatic one-off utility. Shells out to `gh` like the rest of the
//! module. Reads and writes `.context/roadmap-sync-state.json` so that
//! subsequent `sync` invocations can infer `--project-number` from state.

use eyre::Result;

use super::{gh, state};

/// CLI-facing options for `roadmap init`.
#[derive(Debug, Clone)]
pub struct InitOpts {
    /// GitHub owner (user or org) under which the project lives.
    pub owner: String,
    /// Project title. Matched case-insensitively when looking for an
    /// existing project.
    pub title: String,
    /// Optional `owner/repo` — if set, links the project to the repo
    /// (idempotent) and `ensure_labels` runs against the same repo.
    pub repo: Option<String>,
    /// If true, make actual `gh` calls. When false (the safe default), we
    /// only print the plan.
    pub commit: bool,
    /// Ignore an existing `project` section in the state file and
    /// re-provision. Intentional safety speed-bump — off by default.
    pub force: bool,
    pub workspace_root: std::path::PathBuf,
}

/// Option values for the `Phase` single-select field. Matches the
/// `phase-N` convention used by `sync` when setting the field on each
/// issue.
const PHASE_OPTIONS: &[&str] =
    &["phase-1", "phase-2", "phase-3", "phase-4", "phase-5", "phase-6", "phase-7"];

const PHASE_FIELD_NAME: &str = "Phase";
const DEPENDS_FIELD_NAME: &str = "Depends on";

pub fn init(opts: &InitOpts) -> Result<()> {
    eprintln!(
        "xtask roadmap init: owner={}, title={:?}, repo={:?}, dry_run={}, force={}",
        opts.owner, opts.title, opts.repo, !opts.commit, opts.force,
    );

    // 1. Load state. Short-circuit if already provisioned.
    let mut st = state::SyncState::load(&opts.workspace_root)?;
    if let Some(existing) = st.project.as_ref() {
        if !opts.force {
            eprintln!(
                "xtask roadmap init: state already records project #{} ({:?}) under {}",
                existing.number, existing.title, existing.owner,
            );
            eprintln!(
                "xtask roadmap init: pass --force to re-provision (or delete {} to start over)",
                state::SyncState::path(&opts.workspace_root).display(),
            );
            return Ok(());
        }
        eprintln!(
            "xtask roadmap init: --force set; ignoring recorded project #{}",
            existing.number
        );
    }

    // 2. Pre-flight: gh auth. Hard-fail with --commit, soft-warn in dry-run.
    if opts.commit {
        gh::check_auth()?;
    } else if let Err(e) = gh::check_auth() {
        eprintln!("xtask roadmap init: [dry-run] warning: gh check failed: {e}");
        eprintln!("xtask roadmap init: [dry-run] continuing; would fail with --commit");
    }

    // If --repo was provided, validate its shape. We accept either just
    // `name` or `owner/name` — `gh project link --repo` accepts a bare
    // name (it resolves against the project's owner) but callers often
    // copy/paste the `owner/name` form from the sync side. Normalise to
    // the `name` half for the `link` call.
    let repo_name_for_link: Option<String> =
        opts.repo.as_deref().map(|r| match r.split_once('/') {
            Some((_, name)) if !name.is_empty() => name.to_string(),
            Some(_) | None => r.to_string(),
        });

    // 3. Find or create project.
    let summary = if opts.commit {
        if let Some(existing) = gh::project_find_by_title(&opts.owner, &opts.title)? {
            eprintln!(
                "[project] reusing existing #{} ({:?}) under {}",
                existing.number, existing.title, opts.owner,
            );
            existing
        } else {
            let created = gh::project_create(&opts.owner, &opts.title)?;
            eprintln!(
                "[project] created #{} ({:?}) under {}",
                created.number, created.title, opts.owner,
            );
            created
        }
    } else {
        eprintln!(
            "[project] (dry-run) would look up {:?} under {}; if missing, \
             create via `gh project create --owner {} --title {:?} --format json`",
            opts.title, opts.owner, opts.owner, opts.title,
        );
        // Placeholder summary — enough to drive the rest of the dry-run
        // plan. Real values come back only on --commit.
        gh::ProjectSummary { id: "<project_node_id>".into(), number: 0, title: opts.title.clone() }
    };

    // 4. Ensure fields — look up existing by name; create if missing.
    let (phase_field_id, depends_field_id) = ensure_fields(opts, &summary)?;

    // 5. Link project to repo (idempotent, swallows "already linked").
    if let Some(repo_name) = repo_name_for_link.as_deref() {
        if opts.commit {
            gh::project_link_repo(&opts.owner, summary.number, repo_name)?;
            eprintln!("[project] linked to repo {repo_name}");
        } else {
            eprintln!(
                "[project] (dry-run) would link project #{} to repo {repo_name} via \
                 `gh project link {} --owner {} --repo {repo_name}`",
                summary.number, summary.number, opts.owner,
            );
        }
    }

    // 6. Create labels. If --repo is `owner/name`, use it. If just a bare
    //    `name`, label operations need a full `owner/name` — assume the
    //    same owner as the project.
    if let Some(repo_slug) = label_repo_slug(opts, &opts.owner) {
        gh::ensure_labels(&repo_slug, !opts.commit)?;
    } else {
        eprintln!("[label] --repo not supplied; skipping label bootstrap");
        eprintln!(
            "[label] pass --repo owner/name on a future `init` run to create the 18 \
             area/* + type/* labels"
        );
    }

    // 7. Persist state.
    if opts.commit {
        st.project = Some(state::ProjectMeta {
            owner: opts.owner.clone(),
            number: summary.number,
            title: summary.title.clone(),
            project_id: summary.id,
            phase_field_id,
            depends_on_field_id: depends_field_id,
            created_at: now_rfc3339(),
        });
        st.save(&opts.workspace_root)?;
        eprintln!(
            "xtask roadmap init: wrote state to {}",
            state::SyncState::path(&opts.workspace_root).display(),
        );
    } else {
        eprintln!(
            "xtask roadmap init: (dry-run) would write project metadata to {}",
            state::SyncState::path(&opts.workspace_root).display(),
        );
    }

    Ok(())
}

/// Find (or create) the `Phase` + `Depends on` fields on the project.
/// Returns their GraphQL IDs so they can be persisted to state.
fn ensure_fields(opts: &InitOpts, summary: &gh::ProjectSummary) -> Result<(String, String)> {
    if !opts.commit {
        eprintln!(
            "[field] (dry-run) would list fields via `gh project field-list {} --owner {} \
             --format json`",
            summary.number, opts.owner,
        );
        eprintln!(
            "[field] (dry-run) would create `Phase` single-select ({}) if missing",
            PHASE_OPTIONS.join(","),
        );
        eprintln!("[field] (dry-run) would create `{DEPENDS_FIELD_NAME}` text if missing");
        return Ok(("<phase_field_id>".into(), "<depends_on_field_id>".into()));
    }

    let fields = gh::project_field_list(&opts.owner, summary.number)?;

    let phase_id =
        ensure_single_select_field(opts, summary, &fields, PHASE_FIELD_NAME, PHASE_OPTIONS)?;
    let depends_id = ensure_text_field(opts, summary, &fields, DEPENDS_FIELD_NAME)?;

    Ok((phase_id, depends_id))
}

/// Look up `name` in `fields`. Return its ID if present; otherwise create
/// it as a single-select with `options` and return the new ID.
fn ensure_single_select_field(
    opts: &InitOpts,
    summary: &gh::ProjectSummary,
    fields: &gh::ProjectFieldList,
    name: &str,
    options: &[&str],
) -> Result<String> {
    if let Some(existing) = fields.fields.nodes.iter().find(|f| f.name().eq_ignore_ascii_case(name))
    {
        let id = field_id(existing);
        eprintln!("[field] {name} (exists, id={id})");
        return Ok(id);
    }
    let created =
        gh::project_field_create_single_select(&opts.owner, summary.number, name, options)?;
    let id = field_id(&created);
    eprintln!("[field] {name} (created, id={id})");
    Ok(id)
}

/// Look up `name` in `fields`. Return its ID if present; otherwise create
/// it as a free-form text field and return the new ID.
fn ensure_text_field(
    opts: &InitOpts,
    summary: &gh::ProjectSummary,
    fields: &gh::ProjectFieldList,
    name: &str,
) -> Result<String> {
    if let Some(existing) = fields.fields.nodes.iter().find(|f| f.name().eq_ignore_ascii_case(name))
    {
        let id = field_id(existing);
        eprintln!("[field] {name} (exists, id={id})");
        return Ok(id);
    }
    let created = gh::project_field_create_text(&opts.owner, summary.number, name)?;
    let id = field_id(&created);
    eprintln!("[field] {name} (created, id={id})");
    Ok(id)
}

fn field_id(field: &gh::ProjectField) -> String {
    match field {
        gh::ProjectField::SingleSelect { id, .. } | gh::ProjectField::Other { id, .. } => {
            id.clone()
        }
    }
}

/// If `--repo` was supplied, return the `owner/name` slug to pass to the
/// label-creation helper. If the user supplied a bare `name`, assume the
/// project owner is also the repo owner. Returns `None` when `--repo` was
/// omitted.
fn label_repo_slug(opts: &InitOpts, project_owner: &str) -> Option<String> {
    let repo = opts.repo.as_deref()?;
    if repo.contains('/') {
        Some(repo.to_string())
    } else {
        Some(format!("{project_owner}/{repo}"))
    }
}

/// RFC3339 timestamp — matches `sync::now_rfc3339` (intentional duplication
/// to keep the two subcommands decoupled; this is a one-off utility).
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
