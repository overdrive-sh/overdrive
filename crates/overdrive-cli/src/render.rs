//! Output-rendering functions for CLI commands.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`, rendering is separated from
//! handler logic: handlers return typed `Result<Output, CliError>` and
//! never touch `stdout`; `main.rs` formats the output via the functions
//! in this module. Tests exercise the render functions directly — a
//! rendering drift cannot bleed into handler correctness.
//!
//! Output shapes mimic `talosctl cluster status`: a readable
//! multi-line key-value layout, one label per line, `println!`-based
//! (no progress spinners) so the first output lands within the
//! 100ms target on localhost per US-05 AC.

use overdrive_control_plane::api::IdempotencyOutcome;

use crate::commands::alloc::AllocStatusOutput;
use crate::commands::cluster::ClusterStatusOutput;
use crate::commands::job::SubmitOutput;
use crate::commands::node::NodeListOutput;
use crate::http_client::CliError;

/// Render a `ClusterStatusOutput` as a multi-line operator-facing
/// summary.
///
/// Per ADR-0020 §Decision §4 the output is four lines — `Mode`,
/// `Region`, `Reconcilers`, `Broker counters`. The `Commit index` line
/// was dropped: it was an in-memory `u64` and never a substitute for
/// an authoritative metrics endpoint. Activity-rate observability is
/// provided by `broker.dispatched` (heartbeat reconciler ticks).
///
/// Each field is labelled on its own line so an operator can scan the
/// output at a glance; reconciler names and broker counters expand onto
/// subsequent indented lines so the top-level labels stay aligned.
#[must_use]
pub fn cluster_status(out: &ClusterStatusOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Mode:          {}", out.mode);
    let _ = writeln!(s, "Region:        {}", out.region);
    let _ = writeln!(s, "Reconcilers:   {}", out.reconcilers.join(", "));
    let _ = writeln!(
        s,
        "Broker counters: queued={} cancelled={} dispatched={}",
        out.broker.queued, out.broker.cancelled, out.broker.dispatched,
    );
    s
}

/// Render a `NodeListOutput` as a table, falling back to the
/// empty-state message when no rows are present.
///
/// The empty-state message is wired through
/// `NodeListOutput::empty_state_message` so operators always see an
/// explicit pointer to the `phase-1-first-workload` onboarding step.
#[must_use]
pub fn node_list(out: &NodeListOutput) -> String {
    use std::fmt::Write as _;
    if out.rows.is_empty() {
        let mut s = String::new();
        s.push_str("0 nodes registered\n");
        s.push_str(&out.empty_state_message);
        s.push('\n');
        return s;
    }

    let mut s = String::new();
    s.push_str("NODE ID              REGION\n");
    for row in &out.rows {
        let _ = writeln!(s, "{:<20} {}", row.node_id, row.region);
    }
    s
}

/// Render a successful `job submit` as a multi-line operator-facing
/// summary.
///
/// Per ADR-0020 §Decision §2 the labelled set is `Accepted.`,
/// `Job ID:`, `Intent key:`, `Spec digest:`, `Outcome:`, `Endpoint:`,
/// `Next:`. The `Commit index:` line was dropped — `commit_index` was
/// an in-memory `u64`, never a substitute for the spec digest as a
/// stable identity (see ADR-0020 §Considered alternatives §D).
///
/// `outcome` is rendered in human form — `created` for `Inserted`,
/// `unchanged` for `Unchanged`. The JSON wire form stays lowercase per
/// `serde(rename_all = "lowercase")`; the CLI does NOT surface the raw
/// lowercase JSON form to the operator (operators do not read JSON
/// here).
///
/// Each field is labelled on its own line so an operator can scan the
/// output at a glance; the trailing `Next:` line points at the
/// follow-up command so the operator can continue without consulting
/// the docs.
#[must_use]
pub fn job_submit_accepted(out: &SubmitOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Accepted.");
    let _ = writeln!(s, "Job ID:        {}", out.job_id);
    let _ = writeln!(s, "Intent key:    {}", out.intent_key);
    let _ = writeln!(s, "Spec digest:   {}", out.spec_digest);
    let _ = writeln!(s, "Outcome:       {}", outcome_human(out.outcome));
    let _ = writeln!(s, "Endpoint:      {}", out.endpoint);
    let _ = writeln!(s, "Next: {}", out.next_command);
    s
}

/// Map an [`IdempotencyOutcome`] to its human-form rendering for the
/// CLI surface. `Inserted` becomes `created` (matching the operator's
/// mental model — "your spec was created"); `Unchanged` becomes
/// `unchanged` (verbatim). The JSON wire form is `inserted` /
/// `unchanged` per `serde(rename_all = "lowercase")` and stays
/// distinct from this human-form rendering.
fn outcome_human(outcome: IdempotencyOutcome) -> &'static str {
    match outcome {
        IdempotencyOutcome::Inserted => "created",
        IdempotencyOutcome::Unchanged => "unchanged",
    }
}

/// Render an `AllocStatusOutput` as a multi-line operator-facing
/// summary.
///
/// On empty-state (`allocations_total == 0`) the output includes the
/// `phase-1-first-workload` reference carried in `empty_state_message`
/// — this is the load-bearing onboarding signpost for an operator who
/// has submitted a job but sees no allocations yet.
#[must_use]
pub fn alloc_status(out: &AllocStatusOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Job ID:        {}", out.job_id);
    let _ = writeln!(s, "Spec digest:   {}", out.spec_digest);
    let _ = writeln!(s, "Allocations:   {}", out.allocations_total);
    if out.allocations_total == 0 && !out.empty_state_message.is_empty() {
        let _ = writeln!(s, "{}", out.empty_state_message);
    }
    s
}

/// Render a [`CliError`] as an operator-facing multi-line error block.
///
/// For [`CliError::Transport`] the rendered form carries two concrete
/// next-step suggestions — "Verify the endpoint in the operator config"
/// and "Start the control plane" — so the operator has a clear recovery
/// path without consulting docs. There is no `--endpoint` / env-var
/// override surface (per whitepaper §8 the operator config is the sole
/// source), so no third suggestion pointing at a runtime override. For
/// other variants the `Display` form is sufficient and is returned
/// verbatim.
///
/// This function NEVER emits raw reqwest Debug output or low-level
/// transport tokens — those are stripped by `http_client.rs` before
/// the error reaches here.
#[must_use]
pub fn cli_error(err: &CliError) -> String {
    use std::fmt::Write as _;
    match err {
        CliError::Transport { endpoint, cause } => {
            let mut s = String::new();
            let _ = writeln!(s, "Error: could not reach the control plane at {endpoint}.");
            let _ = writeln!(s, "Cause: {cause}.");
            let _ = writeln!(s, "The endpoint is unreachable. Try one of:");
            let _ = writeln!(
                s,
                "  1. Verify the endpoint in `~/.overdrive/config` is correct \
                 (check the port and scheme).",
            );
            let _ = writeln!(s, "  2. Start the control plane: `overdrive serve --bind <addr>`.");
            s
        }
        other => format!("{other}\n"),
    }
}
