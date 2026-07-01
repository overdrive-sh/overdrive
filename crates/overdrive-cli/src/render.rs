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

use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, IdempotencyOutcome, IssuedCertSummary, RestartOutcome,
    StopOutcome,
};

// workload-kind-discriminator slice 05 — Schedule submit/alloc-status
// render functions and the SCHEDULE_EXECUTION_TRACKING_URL SSOT
// constant (KPI K5 byte-equality across surfaces). Per ADR-0047 §1
// + slice 05 spec.
pub mod schedule;

// `render::listener` was deleted in service-vip-allocator step 02-01.
// Per ADR-0049 § 5 / `.claude/rules/development.md` § "Deletion
// discipline": the per-listener VIP rendering (and the
// SERVICE_VIP_ALLOCATOR_TRACKING_URL SSOT constant that fronted the
// pending-allocation form) was structurally obsolete once `Listener`
// lost its `vip` field — VIPs are now platform-issued service-wide
// via `ServiceVipAllocator`, rendered at the service-level surface
// (not per-listener). The module had no callers outside its own
// `#[cfg(test)] mod tests`, so production code AND its tests were
// deleted together in the same commit.

use crate::commands::alloc::AllocStatusOutput;
use crate::commands::cluster::ClusterStatusOutput;
use crate::commands::deploy::{DeployOutput, StopOutput};
use crate::commands::node::NodeListOutput;
use crate::commands::workload::RestartOutput;
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

/// Render a successful `deploy` as a multi-line operator-facing
/// summary.
///
/// Per ADR-0020 §Decision §2 the labelled set is `Accepted.`,
/// `Workload ID:`, `Intent key:`, `Spec digest:`, `Outcome:`, `Endpoint:`,
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
pub fn workload_submit_accepted(out: &DeployOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Accepted.");
    let _ = writeln!(s, "Workload ID:   {}", out.workload_id);
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
const fn outcome_human(outcome: IdempotencyOutcome) -> &'static str {
    match outcome {
        IdempotencyOutcome::Inserted => "created",
        IdempotencyOutcome::Unchanged => "unchanged",
    }
}

/// Render the result of `overdrive job stop` per AC.
///
/// On `Stopped`, the line is `Stopped workload '<id>'.`; on
/// `AlreadyStopped` the line names the idempotent path so the operator
/// knows the call was a no-op. Per ADR-0027 + Step 02-04 AC.
#[must_use]
pub fn workload_stop_accepted(out: &StopOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    match out.outcome {
        StopOutcome::Stopped => {
            let _ = writeln!(s, "Stopped workload '{}'.", out.workload_id);
        }
        StopOutcome::AlreadyStopped => {
            let _ = writeln!(s, "Workload '{}' was already stopped (no-op).", out.workload_id);
        }
    }
    let _ = writeln!(s, "Endpoint: {}", out.endpoint);
    s
}

/// Render the result of `overdrive workload restart` per ADR-0073.
///
/// On `Restarted`, the line names the fresh-instance placement; on
/// `Resumed` it names the resumed-from-stopped path so the operator
/// knows a stop sentinel was cleared. Sibling of
/// [`workload_stop_accepted`].
#[must_use]
pub fn workload_restart_accepted(out: &RestartOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    match out.outcome {
        RestartOutcome::Restarted => {
            let _ = writeln!(s, "Restarted workload '{}' with a fresh instance.", out.workload_id);
        }
        RestartOutcome::Resumed => {
            let _ = writeln!(
                s,
                "Resumed workload '{}' (cleared the stop intent and placed a fresh instance).",
                out.workload_id,
            );
        }
    }
    let _ = writeln!(s, "Endpoint: {}", out.endpoint);
    s
}

/// Render an `AllocStatusOutput` as the single, operator-facing,
/// kind-aware alloc-status summary — the LIVE render path.
///
/// This is the ONE alloc-status renderer `overdrive alloc status`
/// dispatches through: `main.rs` calls `commands::alloc::status(..)`
/// (returning an `AllocStatusOutput`) and prints
/// `render::alloc_status(&out)`. There is no second/duplicate renderer
/// — the historical flat `alloc_status` and the test-only
/// `alloc_status_kind_aware` were consolidated into this one function
/// (workload-kind-discriminator step 02-02 built the kind-aware body but
/// never wired it into the command; this is that wiring landed for real).
///
/// The body branches on [`overdrive_core::aggregate::WorkloadKind`]
/// (carried on `out.snapshot.kind`) per design [D4] / ADR-0047 §4:
///
/// - **Service**: `Service '<name>' (kind: Service)` header + `Spec
///   digest:` + `Replicas (desired/running): N/M` + per-alloc table
///   (`Alloc / State / Restarts / Since`, NO Exit column).
/// - **Job**: `Job '<name>' (kind: Job)` header + `Spec digest:` +
///   `Verdict:` (derived) + per-attempt table (`Attempt / State / Exit
///   / Started / Duration`) + stderr tail on Failed.
/// - **Schedule**: `Schedule '<name>' (kind: Schedule)` header + cron
///   tracking URL.
///
/// On empty-state (`allocations_total == 0`) the output is prefixed with
/// the `phase-1-first-workload` reference carried in
/// `empty_state_message` — the load-bearing onboarding signpost
/// (DWD-05) for an operator who has submitted a workload but sees no
/// allocations yet. This wrapper concern is preserved across the
/// kind-aware body (which has no access to the wrapper-level
/// `empty_state_message`); the signpost reads first so it is the
/// operator's primary signal before the (empty) kind-specific body.
///
/// After the kind-specific body, the shared VIP / Listeners /
/// Issued-certificates sections render (presence-guarded + additive):
/// the VIP rides on `out.snapshot.vip` (Service-only per ADR-0049 /
/// #183); Listeners are an INTENT property rendered at ANY allocation
/// count (including 0) so a pre-convergence UDP Service surfaces
/// `5353/udp`; Issued certificates are the built-in-ca #215 consumer
/// side (ADR-0067 #215-boundary) projecting audit-row FACTS per running
/// alloc, kind-agnostically.
#[must_use]
pub fn alloc_status(out: &AllocStatusOutput) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();

    // Empty-state onboarding signpost (DWD-05) — a wrapper concern, not
    // a kind-aware-body concern: the kind-aware body sees only
    // `out.snapshot` and has no access to the wrapper-level
    // `empty_state_message`. Rendered FIRST so an operator who submitted
    // a workload but sees no allocations yet reads the `phase-1-first-
    // workload` pointer before the (necessarily empty) kind-specific
    // body. Gated on BOTH `allocations_total == 0` AND a non-empty
    // message (asymmetric `&&` — a flip to `||` would either leak the
    // hint when allocations exist or fire a blank `writeln!`).
    if out.allocations_total == 0 && !out.empty_state_message.is_empty() {
        let _ = writeln!(s, "{}", out.empty_state_message);
    }

    // Kind-specific body (the workload-kind-discriminator [D4] design).
    render_kind_aware_body(&mut s, &out.snapshot);

    // VIP + Listeners are the Service frontend — group them (VIP first).
    // The VIP rides on the snapshot envelope (`vip: Some(..)` for a
    // Service, `None` for Job/Schedule per ADR-0049 / #183); the line is
    // omitted entirely when absent.
    render_vip_section(&mut s, out.snapshot.vip.as_deref());
    // Listeners are an INTENT property carried on the snapshot envelope,
    // independent of allocations/convergence — render them at ANY
    // allocation count (including 0) so a pre-convergence UDP Service
    // surfaces `5353/udp`. Gated only on listener presence.
    render_listeners_section(&mut s, &out.snapshot.listeners);
    // Issued-certificate summaries are the consumer-side of built-in-ca
    // #215 (ADR-0067 #215-boundary): the server projects the current SVID
    // FACTS per running alloc onto the snapshot envelope. Workload-kind-
    // AGNOSTIC (the server projection filters only on `AllocState::Running`
    // with no `WorkloadKind` filter, and SVIDs are issued to Jobs as much
    // as Services), so it renders after the kind-specific body for every
    // kind. Presence-guarded + additive: a workload with no issued certs
    // renders byte-identically to before.
    render_issued_certificates_section(&mut s, &out.snapshot.issued_certificates);
    s
}

/// Append an indented cause-detail block for a single Service per-alloc
/// table row, IFF the row carries a structured transition `reason` or a
/// verbatim driver `error`.
///
/// Preserves RCA finding S-A4: a Service allocation whose backend died on
/// startup (e.g. `bind: Address already in use`) must read as Failed WITH
/// its captured cause on the alloc-status snapshot, not as a healthy bare
/// count. The designed `Alloc / State / Restarts / Since` table columns
/// already carry the `state` (so the row reads `Failed`); this adds the
/// cause beneath the row without a new column. Presence-guarded and
/// additive — a Running row with no `reason`/`error` emits nothing, so
/// the table stays byte-identical for the healthy case. Shares
/// `state_label` / `TransitionReason::human_readable()` vocabulary with
/// the rest of the renderer.
fn render_row_cause_detail(
    out: &mut String,
    row: &overdrive_control_plane::api::AllocStatusRowBody,
) {
    use std::fmt::Write as _;
    if let Some(reason) = &row.reason {
        let _ = writeln!(out, "    reason: {}", reason.human_readable());
    }
    if let Some(error) = &row.error {
        let _ = writeln!(out, "    error: {error}");
    }
    if let Some(code) = row.exit_code {
        let _ = writeln!(out, "    exit code: {code}");
    }
}

/// Append the operator-facing `VIP:` line to `out` IFF the workload
/// carries a platform-issued Service VIP. `AllocStatusResponse.vip` is
/// `Some` only for `WorkloadKind::Service` reads (populated from the
/// allocator memo per ADR-0049 / #183); `None` for Job/Schedule, in
/// which case the line is omitted entirely (never `VIP: None`) — the
/// same presence-guard discipline as `render_listeners_section`. The
/// VIP is the Service frontend address; it is grouped with `Listeners:`
/// (VIP first). The label is aligned to a value column at offset 15.
/// Called once by the single live `alloc_status` renderer, after the
/// kind-specific body.
fn render_vip_section(out: &mut String, vip: Option<&str>) {
    use std::fmt::Write as _;
    let Some(vip) = vip else {
        return;
    };
    let _ = writeln!(out, "VIP:           {vip}");
}

/// Append the operator-facing `Listeners:` section to `out` IFF the
/// intent declared at least one listener. Each listener renders as
/// `<port>/<protocol>` using the protocol enum's canonical lowercase
/// `as_str()` (per `.claude/rules/development.md` § "Label enums own
/// their string representation"). Called once by the single live
/// `alloc_status` renderer, after the kind-specific body.
fn render_listeners_section(out: &mut String, listeners: &[overdrive_core::aggregate::Listener]) {
    use std::fmt::Write as _;
    if listeners.is_empty() {
        return;
    }
    let _ = writeln!(out, "Listeners:");
    for listener in listeners {
        let _ = writeln!(out, "  {}/{}", listener.port.get(), listener.protocol.as_str());
    }
}

/// Append the operator-facing `Issued certificates:` section to `out` IFF
/// the response carries at least one [`IssuedCertSummary`]
/// (built-in-ca #215 consumer-side, ADR-0067 #215-boundary). Each summary
/// renders the four audit-row FACTS — `serial`, `spiffe_id`,
/// `issuer_serial`, `not_after` — via their `Display` impls. The summary
/// carries NO certificate PEM/DER bytes and NO private key (the durable
/// `issued_certificates` audit row persists only facts; per the workload-
/// identity model the workload holds nothing), so this render structurally
/// cannot leak cert material.
///
/// The section is purely additive: omitted entirely when the response
/// carries no issued certificates, so output for a workload with no issued
/// certs is byte-identical to before this section existed — the same
/// presence-guard discipline as `render_listeners_section` /
/// `render_vip_section`. The server projects exactly the latest-by-
/// `issued_at` summary per running alloc (landed in step 03-01); this
/// render shows exactly the summaries provided, one row per alloc, not a
/// history list.
///
/// Rendered workload-kind-AGNOSTICALLY by the single live `alloc_status`
/// renderer (after the kind-specific body, for Job / Service / Schedule
/// alike): the server projection (`handlers::issued_certificates_for_rows`)
/// filters only on `AllocState::Running` with no `WorkloadKind` filter, and
/// SVIDs are issued to Jobs as much as Services, so a Job alloc-status
/// legitimately carries summaries. The presence-guard keeps the no-cert
/// output byte-identical for every kind.
fn render_issued_certificates_section(out: &mut String, summaries: &[IssuedCertSummary]) {
    use std::fmt::Write as _;
    if summaries.is_empty() {
        return;
    }
    let _ = writeln!(out, "Issued certificates:");
    for summary in summaries {
        let _ = writeln!(out, "  serial:        {}", summary.serial);
        let _ = writeln!(out, "    spiffe_id:     {}", summary.spiffe_id);
        let _ = writeln!(out, "    issuer_serial: {}", summary.issuer_serial);
        let _ = writeln!(out, "    not_after:     {}", summary.not_after);
    }
}

/// Lowercase variant label for an `AllocStateWire`. Shared between
/// the Service per-alloc table and the Job per-attempt table.
const fn state_label(state: AllocStateWire) -> &'static str {
    match state {
        AllocStateWire::Pending => "Pending",
        AllocStateWire::Running => "Running",
        AllocStateWire::Draining => "Draining",
        AllocStateWire::Suspended => "Suspended",
        AllocStateWire::Terminated => "Terminated",
        AllocStateWire::Failed => "Failed",
        // `AllocStateWire` is `#[non_exhaustive]`; render unknown
        // future variants verbatim rather than panicking.
        _ => "(unknown)",
    }
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

/// Map any [`CliError`] to the operator-visible CLI exit code.
///
/// Per slice 02 step 02-04 acceptance criteria S-CLI-05: every
/// pre-Accepted failure shape (`HttpStatus`, `Transport`, `BodyDecode`,
/// `InvalidSpec`, `ConfigLoad`) maps to exit code **2**. Convergence
/// outcomes (`Succeeded` / `Failed`) are emitted on the
/// streaming success path and map to exit 0 / the workload's non-zero
/// exit code respectively (see
/// [`crate::commands::deploy::deploy_streaming`]); they never flow through
/// this function.
///
/// A non-zero streaming exit signals the workload reached the server
/// but exited non-zero (or did not converge to running). Exit code 2 is
/// "the CLI never got past pre-Accepted plumbing" — the operator
/// distinguishes this from "the workload itself failed" via the exit
/// code alone.
#[must_use]
pub const fn cli_error_to_exit_code(err: &CliError) -> i32 {
    match err {
        // Slice 07 / US-07 — a spec-rejection (e.g. probes on a
        // non-Service workload) is a clean "your spec is wrong" exit,
        // distinct from a plumbing failure. The operator gets exit 1
        // (spec rejected) so scripts can distinguish "fix the spec"
        // from "the CLI never reached the server" (exit 2).
        CliError::ParseError(_) => 1,
        // Every other CliError variant is pre-Accepted plumbing — the
        // CLI never got an `Accepted` line on the streaming bus. Per
        // S-CLI-05 the parametrised expectation is exit 2.
        _ => 2,
    }
}
// ---------------------------------------------------------------------------
// Job-kind render fns — slice 02 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §3 [D2] / [D7]: Job kind workloads are run-to-completion;
// they have no converged-running terminal shape. The structural fix closing the
// bug under audit (RCA: B+C+D conjunction) renders Job-kind submits via
// these dedicated functions whose output cannot contain the historical
// `"is running with"` substring patterns.

/// Render the operator-facing submit echo for a Job-kind workload.
///
/// Per slice 02 spec / S-02-06: emitted BEFORE any streaming events
/// so the operator sees the kind upfront and understands a Job is
/// run-to-completion (not a long-running Service).
///
/// Form: `Submitting job '<name>' (kind=Job, run-to-completion)\n`.
#[must_use]
pub fn format_job_submit_echo(workload_name: &str) -> String {
    format!("Submitting job '{workload_name}' (kind=Job, run-to-completion)\n")
}

/// Render the operator-facing terminal-success line for a Job-kind
/// workload. Pure function. Per slice 02 spec / S-02-01.
///
/// A Job that exits 0 reports `Succeeded` with exit code, duration,
/// and attempts. The CLI maps `Succeeded` → process exit 0.
///
/// Form: `Job '<name>' succeeded. (exit code 0, took <duration>, attempts <N>)\n`
#[must_use]
pub fn format_job_succeeded_summary(
    workload_name: &str,
    exit_code: i32,
    took_human: &str,
    attempts: u32,
) -> String {
    format!(
        "Job '{workload_name}' succeeded. (exit code {exit_code}, took {took_human}, attempts {attempts})\n"
    )
}

/// Render the operator-facing terminal-stopped line for a Job-kind
/// workload. Pure function.
///
/// An operator stop is neither success nor failure — the workload was
/// interrupted before natural completion.
///
/// Form: `Job '<name>' stopped by <initiator>. (took <duration>, attempts <N>)\n`
#[must_use]
pub fn format_job_stopped_summary(
    workload_name: &str,
    stopped_by: &str,
    took_human: &str,
    attempts: u32,
) -> String {
    format!(
        "Job '{workload_name}' stopped by {stopped_by}. (took {took_human}, attempts {attempts})\n"
    )
}

/// Decide whether a Job's retry budget is exhausted. Pure function.
/// Extracted from `consume_stream_job` for testability.
#[must_use]
pub const fn is_backoff_exhausted(attempts: u32, max_attempts: u32) -> bool {
    attempts >= max_attempts && max_attempts > 1
}

/// Render the operator-facing terminal-failure line for a Job-kind
/// workload. Pure function. Per slice 02 spec / S-02-02.
///
/// Form: `Job '<name>' failed. (exit code <N>, took <duration>, attempts <X> of <Y> [(backoff exhausted)])\nstderr tail:\n<tail>`
#[must_use]
pub fn format_job_failed_summary(
    workload_name: &str,
    exit_code: Option<i32>,
    took_human: &str,
    attempts: u32,
    max_attempts: u32,
    backoff_exhausted: bool,
    stderr_tail: &str,
) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let attempts_str = if backoff_exhausted {
        format!("{attempts} of {max_attempts} (backoff exhausted)")
    } else {
        format!("{attempts} of {max_attempts}")
    };
    let exit_display =
        exit_code.map_or_else(|| "none (killed by signal)".to_string(), |c| c.to_string());
    let _ = writeln!(
        s,
        "Job '{workload_name}' failed. (exit code {exit_display}, took {took_human}, attempts {attempts_str})"
    );
    if !stderr_tail.is_empty() {
        // Per step 02-05 / ADR-0033 Amendment 2026-05-10: the header
        // names the line budget so the operator knows whether they're
        // looking at the workload's full stderr or the trailing
        // window. `STDERR_TAIL_LINES` is the project-wide SSOT —
        // sourced from the trait surface in `overdrive_core`, NOT
        // hardcoded here.
        let _ = writeln!(
            s,
            "stderr (last {} lines):",
            overdrive_core::traits::driver::STDERR_TAIL_LINES
        );
        // Indent each line for operator-readability.
        for line in stderr_tail.lines() {
            let _ = writeln!(s, "  {line}");
        }
    }
    s
}

/// Render an intermediate Job attempt-failed line. Pure function.
/// Per slice 02 spec / S-02-03 — intermediate (non-terminal) line;
/// the streaming session stays open after this is emitted.
///
/// Form: `Job '<name>' attempt <N> failed (exit <X>). Retrying in <duration>.\n`
#[must_use]
pub fn format_job_attempt_failed(
    workload_name: &str,
    attempt_index: u32,
    exit_code: i32,
    next_attempt_delay: &str,
) -> String {
    format!(
        "Job '{workload_name}' attempt {attempt_index} failed (exit {exit_code}). Retrying in {next_attempt_delay}.\n"
    )
}

/// Render the streaming running summary line — the
/// operator-facing exit-0 success render. Pure function.
///
/// Per slice 04 of `workload-kind-discriminator`: the function's sole
/// caller is the Service code path (post-WorkloadSpec discriminator),
/// so the rendered vocabulary names "Service".
/// `JobSubmitEvent` carries no converged-running terminal variant in the
/// post-slice-02 tagged-event design. The literal `"live"` (RCA root
/// cause D) is gone; the `took_human` argument carries a measured
/// Clock-derived value rendered by `format_human_duration`.
///
/// Form: `Service '<name>' is running with <running>/<desired> replicas (took <duration>)`.
#[must_use]
pub fn format_running_summary(
    workload_name: &str,
    running: u32,
    desired: u32,
    took_human: &str,
) -> String {
    format!(
        "Service '{workload_name}' is running with {running}/{desired} replicas (took {took_human})\n"
    )
}

/// Format a [`std::time::Duration`] for operator-facing display.
///
/// Replaces the historical `"live"` literal (US-06 of
/// `workload-kind-discriminator`) used as a duration placeholder in
/// the streaming running summary. The output format is
/// chosen for human readability at typical convergence latencies
/// (single-digit ms to a few seconds):
///
/// - `<1ms` → `"<1ms"`
/// - `<1s`  → `"<N>ms"`
/// - `<60s` → `"<N>.<dec>s"` (one decimal place)
/// - `>=60s` → `"<M>m<S>s"`
///
/// Pure function; no allocations beyond the returned `String`.
#[must_use]
pub fn format_human_duration(took: std::time::Duration) -> String {
    let total_millis = took.as_millis();
    if total_millis == 0 {
        return "<1ms".to_string();
    }
    if total_millis < 1_000 {
        return format!("{total_millis}ms");
    }
    let total_secs = took.as_secs();
    if total_secs < 60 {
        // Render with one decimal place for sub-minute durations.
        let tenths = (took.as_millis() % 1_000) / 100;
        return format!("{total_secs}.{tenths}s");
    }
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{minutes}m{seconds}s")
}

// ---------------------------------------------------------------------------
// Job-kind alloc-status render fns — slice 03 step 02-02
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §4 / [D4] of the workload-kind-discriminator feature:
// the alloc-status render layer branches on `AllocStatusResponse.kind`
// without re-fetching intent. Service shows replicas + Restarts (no
// Exit column); Job shows Verdict + per-attempt Exit codes + stderr
// tail; Schedule shows cron + deferral. The match on `WorkloadKind`
// is exhaustive — adding a future kind requires adding one match arm.

/// Operator-facing terminal verdict for a Job-kind workload.
///
/// Computed from the rows' terminal field at render time per
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state" — Verdict is DERIVED from the row's terminal, NOT
/// persisted as a column on the wire. Sourced fresh on every render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobVerdict {
    /// Job exited with a clean terminal (any attempt rolled
    /// `Terminated` / `Stopped { by: Process }` with `exit_code` 0).
    Succeeded,
    /// Job exhausted its backoff budget — every attempt failed.
    Failed,
    /// Job has at least one Running attempt and no terminal yet.
    InProgress,
}

/// Render the operator-facing `Verdict:` line for a Job-kind alloc
/// status. Pure function. Per slice 03 / S-03-02, S-03-03, S-03-04.
#[must_use]
pub fn format_job_verdict(verdict: JobVerdict) -> String {
    let body = match verdict {
        JobVerdict::Succeeded => "Succeeded",
        JobVerdict::Failed => "Failed (backoff exhausted)",
        JobVerdict::InProgress => "In progress (no terminal yet)",
    };
    format!("Verdict: {body}\n")
}

/// Derive a [`JobVerdict`] from a Job-kind alloc status's per-attempt
/// rows. Pure function — operates on the wire-shape rows.
///
/// The classification rule (per design [D4] / `.claude/rules/development.md`
/// § "Persist inputs, not derived state"):
///
/// - any `Terminated` row with `exit_code: Some(0)` → `Succeeded`
/// - any `Running` row with no terminal sibling → `InProgress`
/// - empty `rows` (no allocations yet) → `InProgress`
/// - else (every row is `Failed` or terminated-non-zero) → `Failed`
#[must_use]
pub fn derive_job_verdict(rows: &[overdrive_control_plane::api::AllocStatusRowBody]) -> JobVerdict {
    use overdrive_control_plane::api::AllocStateWire;
    let any_succeeded = rows
        .iter()
        .any(|r| matches!(r.state, AllocStateWire::Terminated) && r.exit_code == Some(0));
    if any_succeeded {
        return JobVerdict::Succeeded;
    }
    let any_running = rows.iter().any(|r| matches!(r.state, AllocStateWire::Running));
    if any_running {
        return JobVerdict::InProgress;
    }
    if rows.is_empty() {
        return JobVerdict::InProgress;
    }
    JobVerdict::Failed
}

/// Render the operator-facing header for a Job-kind alloc status.
/// Pure function. Per slice 03 / step 02-02 acceptance criteria.
///
/// Form:
/// ```text
/// Job '<name>' (kind: Job)
/// Spec digest: <digest>
/// Verdict: <verdict body>
/// ```
#[must_use]
pub fn format_job_alloc_status_header(
    workload_name: &str,
    spec_digest: &str,
    verdict: JobVerdict,
) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Job '{workload_name}' (kind: Job)");
    let _ = writeln!(s, "Spec digest: {spec_digest}");
    s.push_str(&format_job_verdict(verdict));
    s
}

/// Render the per-attempt table for a Job-kind alloc status.
/// Pure function. Per slice 03 / step 02-02 acceptance criteria.
///
/// Columns: `Attempt / State / Exit / Started / Duration`. Running
/// attempts (no terminal yet) render Exit as em-dash (—, U+2014).
/// KPI K3 byte-equality: every persisted `exit_code`'s canonical
/// decimal form appears in the rendered Exit cell verbatim.
#[must_use]
pub fn format_job_alloc_status_attempts_table(
    rows: &[overdrive_control_plane::api::AllocStatusRowBody],
) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<8} {:<12} {:<6} {:<20} {:<10}",
        "Attempt", "State", "Exit", "Started", "Duration",
    );
    for (i, row) in rows.iter().enumerate() {
        let exit_cell = row.exit_code.map_or_else(|| "\u{2014}".to_string(), |c| c.to_string());
        let started = row.started_at.as_deref().unwrap_or("\u{2014}");
        // Phase 1: duration is not yet observed end-to-end; render em-dash.
        let duration = "\u{2014}";
        let _ = writeln!(
            s,
            "{:<8} {:<12} {:<6} {:<20} {:<10}",
            i + 1,
            state_label(row.state),
            exit_cell,
            started,
            duration,
        );
    }
    s
}

/// Append the operator-facing alloc-status body to `out`, branching on
/// [`overdrive_core::aggregate::WorkloadKind`] per design [D4].
///
/// - Service: `Service '...' (kind: Service)` header, `Spec digest:`, `Replicas (desired/running): N/M`, per-alloc table (`Alloc / State / Restarts / Since`; NO Exit column).
/// - Job: `Job '...' (kind: Job)` header, `Spec digest:`, derived Verdict, per-attempt table (`Attempt / State / Exit / Started / Duration`), stderr tail on Failed.
/// - Schedule: `Schedule '...' (kind: Schedule)` header + cron tracking URL.
///
/// The match on `WorkloadKind` is EXHAUSTIVE per ADR-0047 §1 — no
/// catch-all wildcard. Future kinds require explicit arms. `kind` is
/// `None` (forward-compat) → treated as Service.
///
/// This is the kind-specific body of the single live `alloc_status`
/// renderer. It does NOT render the empty-state signpost (a wrapper
/// concern owned by `alloc_status`, which has the `AllocStatusOutput`
/// wrapper fields) nor the kind-agnostic issued-certificates section
/// (also rendered by `alloc_status` after this body, for every kind).
fn render_kind_aware_body(out: &mut String, response: &AllocStatusResponse) {
    use overdrive_core::aggregate::WorkloadKind;
    use std::fmt::Write as _;
    let kind = response.kind.unwrap_or(WorkloadKind::Service);
    let workload_name = response.workload_id.as_deref().unwrap_or("(unknown)");
    let spec_digest = response.spec_digest.as_deref().unwrap_or("");

    match kind {
        WorkloadKind::Service => {
            let _ = writeln!(out, "Service '{workload_name}' (kind: Service)");
            if !spec_digest.is_empty() {
                let _ = writeln!(out, "Spec digest: {spec_digest}");
            }
            let _ = writeln!(
                out,
                "Replicas (desired/running): {}/{}",
                response.replicas_desired, response.replicas_running,
            );
            // Per-alloc table — Service columns: Alloc / State / Restarts / Since.
            let _ =
                writeln!(out, "{:<24} {:<12} {:<10} {:<20}", "Alloc", "State", "Restarts", "Since");
            // Restarts default to 0 in Phase 1 (per-alloc restart counter
            // not surfaced on the wire row body yet — this is a
            // forward-compat placeholder).
            // Restarts default to 0 in Phase 1 (per-alloc restart counter
            // not surfaced on the wire row body yet — this is a
            // forward-compat placeholder). A Failed/errored alloc's
            // captured cause is surfaced on an indented detail line beneath
            // its table row — preserving RCA finding S-A4 (a Service alloc
            // whose backend died on startup, e.g. `bind: Address already in
            // use`, must read as Failed WITH its cause, not a healthy bare
            // count). The designed `Alloc / State / Restarts / Since`
            // columns are unchanged; the detail line is additive and
            // presence-guarded (only emitted when the row carries a
            // structured `reason` or a verbatim driver `error`).
            for row in &response.rows {
                let since = row.started_at.as_deref().unwrap_or("\u{2014}");
                let _ = writeln!(
                    out,
                    "{:<24} {:<12} {:<10} {:<20}",
                    row.alloc_id,
                    state_label(row.state),
                    "0",
                    since,
                );
                render_row_cause_detail(out, row);
            }
            // VIP + Listeners (the Service frontend) are NOT rendered here
            // — the wrapper `alloc_status` renders them once, after this
            // body, for every kind (presence-guarded: `vip` is `Some` only
            // for Service, listeners only when declared). Keeping them out
            // of this arm avoids a double-render.
        }
        WorkloadKind::Job => {
            let verdict = derive_job_verdict(&response.rows);
            out.push_str(&format_job_alloc_status_header(workload_name, spec_digest, verdict));
            out.push('\n');
            out.push_str(&format_job_alloc_status_attempts_table(&response.rows));
            // stderr tail on Failed: pull from the last attempt's
            // `error` field if present (the action shim threads
            // `prior_row.detail` / `prior_row.stderr_tail` onto the
            // wire row body's `error` field).
            if matches!(verdict, JobVerdict::Failed)
                && let Some(last) = response.rows.last()
                && let Some(err) = &last.error
                && !err.is_empty()
            {
                out.push('\n');
                let _ = writeln!(
                    out,
                    "stderr (last {} lines):",
                    overdrive_core::traits::driver::STDERR_TAIL_LINES
                );
                for line in err.lines() {
                    let _ = writeln!(out, "  {line}");
                }
            }
        }
        WorkloadKind::Schedule => {
            // Schedule branch — minimal Phase-1 rendering. Slice 05
            // (deploy_schedule) provides the deferral surface;
            // here we name the kind so the dispatcher is exhaustive.
            let _ = writeln!(out, "Schedule '{workload_name}' (kind: Schedule)");
            if !spec_digest.is_empty() {
                let _ = writeln!(out, "Spec digest: {spec_digest}");
            }
            let _ = writeln!(out, "{}", crate::render::schedule::SCHEDULE_EXECUTION_TRACKING_URL);
        }
    }
}

/// Render the streaming stopped summary line — the
/// operator-facing exit-0 success render fired when a workload
/// reaches a clean terminal stop. Pure function.
///
/// Mirrors `format_running_summary`'s shape (single line, trailing
/// newline). The `kind` argument is the workload-kind discriminator
/// per ADR-0047 / slice 04 of `workload-kind-discriminator`: it picks
/// the operator-facing vocabulary so a Service stop reads `Service
/// '...' was stopped by ...`, a Job stop reads `Job '...' was stopped
/// by ...`, and a Schedule stop reads `Schedule '...' was deregistered
/// by ...` (Schedule is registered/deregistered, not "stopped" — the
/// vocabulary mirrors slice 05's submit-side phrasing).
///
/// The `by` argument names the initiator: operator-driven stop intent,
/// reconciler-driven convergence to terminal, or natural process exit.
/// `StoppedBy` is `#[non_exhaustive]` per
/// `overdrive_core::transition_reason`; the catch-all arm carries
/// neutral phrasing so a future variant does not silently render an
/// empty initiator.
///
/// RCA: `docs/feature/fix-converged-stopped-cli-arm/deliver/rca.md`.
#[must_use]
pub fn format_stopped_summary(
    workload_name: &str,
    kind: overdrive_core::aggregate::WorkloadKind,
    by: overdrive_core::transition_reason::StoppedBy,
) -> String {
    let initiator = match by {
        overdrive_core::transition_reason::StoppedBy::Operator => "operator",
        overdrive_core::transition_reason::StoppedBy::Reconciler => "reconciler",
        overdrive_core::transition_reason::StoppedBy::Process => "process",
        // `StoppedBy` is `#[non_exhaustive]` — neutral phrasing for
        // any Phase-2+ variant added without updating this mapping.
        _ => "an unrecognised initiator",
    };
    match kind {
        overdrive_core::aggregate::WorkloadKind::Service => {
            format!("Service '{workload_name}' was stopped by {initiator}.\n")
        }
        overdrive_core::aggregate::WorkloadKind::Job => {
            format!("Job '{workload_name}' was stopped by {initiator}.\n")
        }
        overdrive_core::aggregate::WorkloadKind::Schedule => {
            format!("Schedule '{workload_name}' was deregistered by {initiator}.\n")
        }
    }
}

// ---------------------------------------------------------------------------
// Service-kind streaming render fns — step 01-03e3 (ADR-0056 / ADR-0059).
// ---------------------------------------------------------------------------
//
// Per ADR-0056: the Service-kind streaming wire surface emits the
// typed `ServiceSubmitEvent` enum. The CLI render layer projects each
// terminal variant into operator-facing text. The `format_stopped_summary`
// (kind-aware) function above already renders the `Stopped` variant —
// these two functions cover the `Stable` and `Failed` shapes.

/// Render the operator-facing `Stable` terminal summary for a Service
/// workload per ADR-0055. Pure function.
///
/// Form: `Service '<name>' is stable (settled in <ms>; witness: <role> probe[<idx>] (<mech>))`.
#[must_use]
pub fn format_service_stable_summary(
    workload_name: &str,
    settled_in_ms: u64,
    witness: &overdrive_core::transition_reason::ProbeWitness,
) -> String {
    let inferred = if witness.inferred { " inferred" } else { "" };
    format!(
        "Service '{workload_name}' is stable (settled in {settled_in_ms}ms; \
         witness:{inferred} {role} probe[{idx}] ({mech}))\n",
        role = witness.role,
        idx = witness.probe_idx,
        mech = witness.mechanic_summary,
    )
}

// ---------------------------------------------------------------------------
// Probes-section render fns — slice 06 step 02-03 (ADR-0033 enrichment /
// US-06 / K4).
// ---------------------------------------------------------------------------
//
// The Probes section is rendered IFF `kind == Service AND
// probes_present`; it is ABSENT for Job / Schedule per US-06. The
// kind-guard is the load-bearing render contract — property-tested by
// `ProbeRenderIsKindGuarded` in
// `tests/acceptance/probes_section_render.rs`.
//
// `ProbeRenderRow` is the typed render-input (newtype/typed discipline
// per `.claude/rules/development.md` § "Newtypes"). It is composed by
// the caller from the spec-side `ProbeDescriptor` (mechanic, role,
// inferred, failure_threshold) and the observation-side
// `ProbeResultRow` (status, last_observed_at_unix_ms,
// consecutive_failures). The render layer is pure over this input — it
// performs no hydration of its own.

/// Typed render-input for a single probe row in the Probes section.
///
/// Composed by the caller from the spec-side `ProbeDescriptor` and the
/// observation-side `ProbeResultRow`. `status == None` materialises the
/// `last=pending` rendering per US-06 (row absence IS pending).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRenderRow {
    /// Role of this probe (`startup` / `readiness` / `liveness`).
    pub role: overdrive_core::observation::probe_result_row::ProbeRole,
    /// 0-indexed position within the role array.
    pub probe_idx: overdrive_core::observation::probe_result_row::ProbeIdx,
    /// Concrete mechanic — drives the per-mechanic summary line shape.
    pub mechanic: overdrive_core::aggregate::probe_descriptor::ProbeMechanic,
    /// Latest observed outcome; `None` for a declared-but-not-yet-ticked
    /// probe (renders `last=pending`).
    pub status: Option<overdrive_core::observation::probe_result_row::ProbeStatus>,
    /// Wall-clock (UNIX-epoch ms) of the latest observation; `None`
    /// when no row exists yet.
    pub last_observed_at_unix_ms: Option<u64>,
    /// `true` IFF the platform synthesised this probe per ADR-0058
    /// (renders an `(inferred)` suffix).
    pub inferred: bool,
    /// Consecutive failures observed for this probe. Drives the
    /// `(<consecutive_failures>/<threshold>)` ratio suffix when the
    /// probe is currently failing under a declared threshold.
    pub consecutive_failures: u32,
    /// Failure threshold for this probe (liveness `failure_threshold`,
    /// readiness `success_threshold`); `None` for startup probes (no
    /// ratio suffix).
    pub failure_threshold: Option<u32>,
}

/// Render the operator-facing per-mechanic summary for a probe, per the
/// US-06 AC shapes.
///
/// - `Tcp` renders `tcp <host>:<port>`.
/// - `Http` renders `http GET http://<host>:<port><path>`; the host
///   defaults to the bind-side wildcard `0.0.0.0` when the descriptor
///   omits it.
/// - `Exec` renders `exec <command>` (space-joined argv).
///
/// Distinct from the reconciler's compact `ProbeWitness.mechanic_summary`
/// surface (`http <host>:<port><path>`) — this is the operator-facing
/// alloc-status render shape.
#[must_use]
pub fn format_probe_mechanic_summary(
    mechanic: &overdrive_core::aggregate::probe_descriptor::ProbeMechanic,
) -> String {
    use overdrive_core::aggregate::probe_descriptor::ProbeMechanic;
    match mechanic {
        ProbeMechanic::Tcp { host, port } => format!("tcp {host}:{port}"),
        ProbeMechanic::Http { path, port, host } => {
            let host = host.as_deref().unwrap_or("0.0.0.0");
            format!("http GET http://{host}:{port}{path}")
        }
        ProbeMechanic::Exec { command } => format!("exec {}", command.join(" ")),
    }
}

/// Render the operator-facing `last=...` status fragment for a probe
/// row. `None` → `last=pending`; `Pass` → `last=pass`; `Fail` →
/// `last=fail (<reason>)`.
fn format_probe_last_status(
    status: Option<&overdrive_core::observation::probe_result_row::ProbeStatus>,
) -> String {
    use overdrive_core::observation::probe_result_row::ProbeStatus;
    match status {
        None => "last=pending".to_string(),
        Some(ProbeStatus::Pass) => "last=pass".to_string(),
        Some(ProbeStatus::Fail { last_fail_reason }) => {
            format!("last=fail ({last_fail_reason})")
        }
    }
}

/// Render the `Probes:` section of an alloc-status output.
///
/// Per US-06 / K4 the section is emitted IFF `kind` is
/// `WorkloadKind::Service` and the probe set is non-empty. For Job /
/// Schedule allocs (or an empty probe set) the function returns an
/// empty string — the kind-guard is the load-bearing render contract
/// (`ProbeRenderIsKindGuarded` property test).
///
/// Each row carries `role`, `probe_idx`, mechanic summary, last
/// status, and `last_observed_at`. An `(inferred)` suffix marks
/// synthesised default probes, `last=pending` marks
/// declared-but-unobserved probes, and a
/// `(<consecutive_failures>/<threshold>)` ratio suffix marks a probe
/// currently failing under a declared threshold.
///
/// `no_color` is honoured per the `NO_COLOR` env-var AC: when `true`
/// the output carries zero ANSI escape sequences. Phase 1 emits no
/// colour on either branch (the render is plain text), so the flag is
/// observed-and-respected rather than toggling a colour path that does
/// not yet exist — the structural guarantee is that no ANSI escape can
/// appear in the output regardless of the flag, which the `NO_COLOR`
/// proptest pins.
#[must_use]
pub fn probes_section(
    kind: overdrive_core::aggregate::WorkloadKind,
    probes: &[ProbeRenderRow],
    no_color: bool,
) -> String {
    use overdrive_core::aggregate::WorkloadKind;
    use std::fmt::Write as _;

    // Kind-guard: Service-only, and only when probes are present.
    if !matches!(kind, WorkloadKind::Service) || probes.is_empty() {
        return String::new();
    }
    // `no_color` is respected by construction — Phase 1 render is plain
    // text with no ANSI sequences on either branch. Bind it so a future
    // colourised branch must thread the flag rather than ignore it.
    let _ = no_color;

    let mut s = String::new();
    let _ = writeln!(s, "Probes:");
    for probe in probes {
        let role = probe.role.as_str();
        let mechanic = format_probe_mechanic_summary(&probe.mechanic);
        let last = format_probe_last_status(probe.status.as_ref());
        let observed = probe.last_observed_at_unix_ms.map_or_else(
            || "last_observed_at=\u{2014}".to_string(),
            |ms| format!("last_observed_at={ms}"),
        );
        let inferred_suffix = if probe.inferred { " (inferred)" } else { "" };

        // Failure-ratio suffix: rendered only when the probe is
        // currently failing AND a threshold is declared.
        let failing = matches!(
            probe.status,
            Some(overdrive_core::observation::probe_result_row::ProbeStatus::Fail { .. })
        );
        let ratio_suffix = match (failing, probe.failure_threshold) {
            (true, Some(threshold)) => {
                format!(" ({}/{threshold})", probe.consecutive_failures)
            }
            _ => String::new(),
        };

        let _ = writeln!(
            s,
            "  {role} probe[{idx}] {mechanic} {last} {observed}{ratio_suffix}{inferred_suffix}",
            idx = probe.probe_idx.get(),
        );
    }
    s
}

/// Render the operator-facing `Failed` block for a Service workload
/// per ADR-0056 / ADR-0059. Pure function.
///
/// Renders the operator-facing `Failed` block against the typed
/// `ServiceFailureReason` discriminator. The five-section shape
/// (header / reason / last-event / reproducer / hint) gives the
/// operator a consistent failure render.
/// `early_exit_timing` carries `(elapsed_secs, startup_deadline_secs)`
/// for the Slice 08 `EarlyExit` multi-line block (S-SHCP-CLI-07). It is
/// rendered ONLY for the `EarlyExit` reason; `None` (or any non-
/// `EarlyExit` reason) omits the `elapsed:` line. The values are
/// supplied by the caller from the stream-side elapsed measurement +
/// the live `startup_deadline` policy — they are NOT carried on the
/// `EarlyExit { exit_code }` wire variant (extending that variant would
/// bump the rkyv `AllocStatusRowEnvelope`; per the persist-inputs rule
/// the elapsed/deadline are recomputed render-side, not persisted).
#[must_use]
pub fn format_service_failed_block(
    workload_name: &str,
    reason: &overdrive_core::transition_reason::ServiceFailureReason,
    stderr_tail: Option<&str>,
    early_exit_timing: Option<(u64, u64)>,
) -> String {
    use overdrive_core::transition_reason::{BackoffCause, ServiceFailureReason};
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Error: workload '{workload_name}' did not converge to stable.");

    let reason_text = match reason {
        ServiceFailureReason::StartupTimeout { probe_idx, attempts } => {
            format!("startup probe[{probe_idx}] timed out after {attempts} attempts")
        }
        ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts } => {
            format!("startup probe[{probe_idx}] failed after {attempts} attempts: {last_fail}")
        }
        ServiceFailureReason::EarlyExit { exit_code: Some(code) } => {
            format!("workload exited early with code {code}")
        }
        ServiceFailureReason::EarlyExit { exit_code: None } => {
            "workload killed by signal before startup probe could pass".to_string()
        }
        ServiceFailureReason::LivenessProbeFailed { probe_idx, attempts } => {
            format!("liveness probe[{probe_idx}] failed after {attempts} attempts")
        }
        ServiceFailureReason::BackoffExhausted { attempts, cause, last_exit_code } => {
            let cause_label = match cause {
                BackoffCause::AttemptBudget => "attempt budget",
                BackoffCause::LivenessBudget => "liveness budget",
                _ => "unknown cause",
            };
            let exit_suffix =
                last_exit_code.map(|c| format!(" (last exit code {c})")).unwrap_or_default();
            format!("backoff exhausted after {attempts} attempts ({cause_label}){exit_suffix}")
        }
        ServiceFailureReason::Other { source, message } => {
            format!("custom failure '{source}': {message}")
        }
        ServiceFailureReason::Timeout { after_seconds } => {
            format!("workload did not converge within {after_seconds}s")
        }
        ServiceFailureReason::StreamInterrupted => {
            "server-side stream interrupted before convergence".to_string()
        }
        _ => "unknown failure reason".to_string(),
    };
    let _ = writeln!(s, "  reason: {reason_text}");

    // S-SHCP-CLI-07 / 08 (Slice 08, RCA-A render hardening) — the
    // `EarlyExit` failure on a Service-kind alloc renders a multi-line
    // diagnostic block: the exit code on its own line, the Service-kind
    // guidance explaining why an early exit IS a failure (a Service is
    // expected to stay up; exiting before any startup probe could pass
    // is the RCA-A coinflip case), and the stderr tail. This is the
    // operator-facing surface that the RCA-A guard
    // (`ServiceKindRenderNeverContainsTookLive`) defends — a Service
    // must NEVER render the misleading `(took live)` success phrasing
    // for an early exit.
    if let ServiceFailureReason::EarlyExit { exit_code } = reason {
        match exit_code {
            Some(code) => {
                let _ = writeln!(s, "  exit_code: {code}");
            }
            None => {
                let _ = writeln!(s, "  exit_code: none (killed by signal)");
            }
        }
        if let Some((elapsed_secs, startup_deadline_secs)) = early_exit_timing {
            let _ = writeln!(
                s,
                "  elapsed: {elapsed_secs}s (startup_deadline={startup_deadline_secs}s)"
            );
        }
        let _ = writeln!(
            s,
            "  The workload exited before any startup probe could pass; a Service is expected to stay running."
        );
    }

    if let Some(detail) = stderr_tail
        && !detail.is_empty()
    {
        let _ = writeln!(s, "  last-event: {detail}");
        let _ = writeln!(s, "  stderr_tail: \"{detail}\"");
    }

    let _ = writeln!(s, "  reproducer: overdrive alloc status --job {workload_name}");
    let _ = writeln!(s);
    let _ = writeln!(s, "Hint: see alloc status for full context");
    s
}
