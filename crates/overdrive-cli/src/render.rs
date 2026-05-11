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
    AllocStateWire, AllocStatusResponse, IdempotencyOutcome, StopOutcome, TerminalReason,
    TransitionSource,
};
use overdrive_core::TransitionReason;

// workload-kind-discriminator slice 05 — Schedule submit/alloc-status
// render functions and the SCHEDULE_EXECUTION_TRACKING_URL SSOT
// constant (KPI K5 byte-equality across surfaces). Per ADR-0047 §1
// + slice 05 spec.
pub mod schedule;

// workload-kind-discriminator slice 06 — Service `[[listener]]`
// render functions and the SERVICE_VIP_ALLOCATOR_TRACKING_URL SSOT
// constant (KPI K6 byte-equality across surfaces). Per ADR-0047 §1
// + slice 06 spec.
pub mod listener;

use crate::commands::alloc::AllocStatusOutput;
use crate::commands::cluster::ClusterStatusOutput;
use crate::commands::job::{StopOutput, SubmitOutput};
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
pub fn workload_submit_accepted(out: &SubmitOutput) -> String {
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
    let _ = writeln!(s, "Workload ID:   {}", out.workload_id);
    let _ = writeln!(s, "Spec digest:   {}", out.spec_digest);
    let _ = writeln!(s, "Allocations:   {}", out.allocations_total);
    if out.allocations_total == 0 && !out.empty_state_message.is_empty() {
        let _ = writeln!(s, "{}", out.empty_state_message);
    }
    s
}

/// Render a typed [`AllocStatusResponse`] as the journey TUI mockup
/// from ADR-0033 §4 (amended 2026-04-30 — cause-class rendering).
///
/// Per slice 01 step 01-03 / S-AS-04 / S-AS-05 / S-AS-06: the renderer
/// is a pure function over the typed response. Three case-arms drive
/// the output:
///
/// * **Running** — full envelope with `Restart budget: U / M used`,
///   per-row `Last transition` block.
/// * **Failed** — adds `(backoff exhausted)` to the budget line when
///   `restart_budget.exhausted` is set; surfaces the verbatim
///   driver error from the row's `error` field.
/// * **Pending-no-capacity** — never shows `Allocations: 0`; the
///   row's `reason: TransitionReason::NoCapacity` is rendered
///   explicitly via `human_readable()` plus the `error` line for
///   the requested-vs-free diagnostic.
///
/// `human_readable()` lives on `TransitionReason` so the snapshot and
/// streaming surfaces share one rendering function.
#[must_use]
pub fn alloc_snapshot(out: &AllocStatusResponse) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    if let Some(workload_id) = &out.workload_id {
        let _ = writeln!(s, "Workload ID:   {workload_id}");
    }
    if let Some(digest) = &out.spec_digest {
        let _ = writeln!(s, "Spec digest:   {digest}");
    }
    let _ = writeln!(s, "Replicas:      {}/{}", out.replicas_running, out.replicas_desired);
    if let Some(budget) = &out.restart_budget {
        if budget.exhausted {
            let _ = writeln!(
                s,
                "Restart budget: {used} / {max} used (backoff exhausted)",
                used = budget.used,
                max = budget.max,
            );
        } else {
            let _ = writeln!(s, "Restart budget: {} / {} used", budget.used, budget.max);
        }
    }

    for row in &out.rows {
        let _ = writeln!(s);
        let _ = writeln!(s, "Allocation:    {}", row.alloc_id);
        let _ = writeln!(s, "  state:       {}", state_label(row.state));
        if let Some(reason) = &row.reason {
            let _ = writeln!(s, "  reason:      {}", reason.human_readable());
        }
        if let Some(error) = &row.error {
            let _ = writeln!(s, "  error:       {error}");
        }
        if let Some(last) = &row.last_transition {
            let from =
                last.from.map_or_else(|| "(initial)".to_owned(), |f| state_label(f).to_owned());
            let to = state_label(last.to);
            let _ = writeln!(
                s,
                "  Last transition: {at} {from} → {to} reason: {reason} source: {source}",
                at = last.at,
                reason = last.reason.human_readable(),
                source = source_label(last.source),
            );
        }
    }
    s
}

/// Lowercase variant label for an `AllocStateWire`. Shared between
/// the headline state line and the transition arrow rendering.
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

/// Render the source-attribution segment of a `Last transition` line.
fn source_label(source: TransitionSource) -> String {
    match source {
        TransitionSource::Reconciler => "reconciler".to_owned(),
        TransitionSource::Driver(driver) => format!("driver({driver})"),
        // `TransitionSource` is `#[non_exhaustive]` — forward-compat fallback.
        _ => "(unknown)".to_owned(),
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
/// outcomes (`ConvergedRunning` / `ConvergedFailed`) are emitted on the
/// streaming success path and map to 0 / 1 respectively (see
/// [`crate::commands::job::submit_streaming`]); they never flow through
/// this function.
///
/// Exit code 1 is reserved for `ConvergedFailed` only — the workload
/// reached the server but did not converge to running. Exit code 2 is
/// "the CLI never got past pre-Accepted plumbing" — the operator
/// distinguishes this from "the workload itself failed" via the exit
/// code alone.
#[must_use]
pub const fn cli_error_to_exit_code(_err: &CliError) -> i32 {
    // Every CliError variant is pre-Accepted — the CLI never got an
    // `Accepted` line on the streaming bus. Per S-CLI-05 the
    // parametrised expectation is exit 2 across the board.
    2
}

/// Render the operator-facing `Error:` block emitted on
/// `SubmitEvent::ConvergedFailed`. Pure function — no I/O.
///
/// Per `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/02-04`
/// step 02-04 acceptance criteria S-CLI-04 and the journey TUI mockup
/// in `docs/.../journey/walking-skeleton.md`. Five labelled sections:
///
/// ```text
/// Error: workload '<name>' did not converge to running.
///   reason: <human_readable rendering>
///   last-event: <verbatim driver text>
///   reproducer: overdrive alloc status --job <name>
///
/// Hint: <variant-specific hint>
/// ```
///
/// `reason` argument is the standalone `SubmitEvent::ConvergedFailed.reason`
/// field. When present it carries the most-recent cause-class
/// `TransitionReason`. When absent the renderer falls back to the
/// `terminal_reason`'s inner cause (`BackoffExhausted` / `DriverError` carry
/// one); for `Timeout` the reason line cites the configured cap.
///
/// `last_event_detail` is the verbatim driver text (typically the same
/// source as `SubmitEvent::ConvergedFailed.error`). Optional — Phase-2
/// terminal causes may not carry verbatim text.
///
/// `terminal_reason` controls the `Hint:` line mapping per the criteria's
/// cause-class table.
#[must_use]
pub fn format_failed_block(
    workload_name: &str,
    reason: Option<&TransitionReason>,
    last_event_detail: Option<&str>,
    terminal_reason: &TerminalReason,
) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Error: workload '{workload_name}' did not converge to running.");

    // `reason:` line — standalone reason wins; otherwise derive from
    // terminal_reason. The streaming `ConvergedFailed.reason` carries
    // the most recent cause-class TransitionReason; for cap-fired
    // timeouts that field is None and the terminal_reason is the only
    // signal.
    let reason_text = reason
        .map(TransitionReason::human_readable)
        .or_else(|| derive_reason_from_terminal(terminal_reason));
    if let Some(text) = reason_text {
        let _ = writeln!(s, "  reason: {text}");
    }

    if let Some(detail) = last_event_detail {
        let _ = writeln!(s, "  last-event: {detail}");
    }

    // Reproducer line — points the operator at `alloc status --job <name>`
    // for the structured snapshot. Per US-02 walking-skeleton transcript.
    let _ = writeln!(s, "  reproducer: overdrive alloc status --job {workload_name}");

    // Blank line separates the structured block from the variant-specific
    // Hint line.
    let _ = writeln!(s);

    let hint = derive_hint(reason, terminal_reason);
    let _ = writeln!(s, "Hint: {hint}");
    s
}

/// Compute the `reason:` line text when no standalone reason was carried
/// on the streaming `ConvergedFailed` event. Falls back to the inner
/// cause of `BackoffExhausted` / `DriverError`, or the cap-cited
/// rendering for `Timeout`.
fn derive_reason_from_terminal(terminal: &TerminalReason) -> Option<String> {
    match terminal {
        TerminalReason::BackoffExhausted { cause, .. } | TerminalReason::DriverError { cause } => {
            Some(cause.human_readable())
        }
        TerminalReason::Timeout { after_seconds } => {
            Some(format!("workload did not converge within {after_seconds}s"))
        }
        TerminalReason::StreamInterrupted => {
            Some("server-side stream interrupted before convergence".to_owned())
        }
        // `TerminalReason` is `#[non_exhaustive]` for forward-compat.
        // Future variants get a generic rendering until the renderer
        // grows a specific arm.
        _ => None,
    }
}

/// Map a `(reason, terminal_reason)` pair to the operator-facing `Hint:`
/// text per the criteria's cause-class table. The mapping consults the
/// inner cause when reason is None — the operator still gets variant-
/// specific guidance.
fn derive_hint(reason: Option<&TransitionReason>, terminal_reason: &TerminalReason) -> String {
    // Resolve the cause-class TransitionReason to consult, preferring
    // the standalone `reason` field over the terminal_reason's inner
    // cause. Either source flows through the same hint table.
    let cause: Option<&TransitionReason> = reason.map_or_else(
        || match terminal_reason {
            TerminalReason::BackoffExhausted { cause, .. }
            | TerminalReason::DriverError { cause } => Some(cause),
            _ => None,
        },
        Some,
    );

    if let Some(cause) = cause {
        return hint_for_transition_reason(cause).to_owned();
    }

    // No cause-class reason → consult the terminal_reason for the
    // outer-shape hint (Timeout is the canonical example).
    match terminal_reason {
        TerminalReason::Timeout { .. } => {
            "workload did not converge within the server cap; consider --detach for \
             long-running submits"
                .to_owned()
        }
        TerminalReason::StreamInterrupted => {
            "server-side stream was interrupted; re-run `overdrive job submit` or \
             consult `overdrive alloc status --job <id>` for the current state"
                .to_owned()
        }
        _ => "see alloc status for full context".to_owned(),
    }
}

/// Hint text for a cause-class `TransitionReason` per the step 02-04
/// criteria mapping table.
const fn hint_for_transition_reason(reason: &TransitionReason) -> &'static str {
    match reason {
        TransitionReason::ExecBinaryNotFound { .. }
        | TransitionReason::ExecPermissionDenied { .. } => {
            "fix the spec's exec.command path and re-run"
        }
        TransitionReason::ExecBinaryInvalid { .. } => {
            "the file at exec.command is not a valid executable; verify the build artefact"
        }
        TransitionReason::CgroupSetupFailed { .. } => {
            "check cgroup v2 delegation; see overdrive cluster doctor"
        }
        TransitionReason::RestartBudgetExhausted { .. } => {
            "the workload failed repeatedly; address the root cause and re-submit"
        }
        TransitionReason::NoCapacity { .. } => "reduce resource requests or scale the cluster",
        // Every other variant — including progress markers (which
        // should never reach the failed-block renderer in practice but
        // are matched for forward-compat) and the generic
        // `DriverInternalError` — falls back to the neutral hint.
        // `TransitionReason` is `#[non_exhaustive]` so the catch-all
        // arm here ALSO covers any Phase 2+ variant added without
        // updating the mapping table.
        _ => "see alloc status for full context",
    }
}

// ---------------------------------------------------------------------------
// Job-kind render fns — slice 02 of `workload-kind-discriminator`.
// ---------------------------------------------------------------------------
//
// Per ADR-0047 §3 [D2] / [D7]: Job kind workloads are run-to-completion;
// they have no `ConvergedRunning` shape. The structural fix closing the
// bug under audit (RCA: B+C+D conjunction) renders Job-kind submits via
// these dedicated functions whose output cannot contain the historical
// `"is running with"` / `"(took live)"` substrings.

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
    exit_code: i32,
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
    let _ = writeln!(
        s,
        "Job '{workload_name}' failed. (exit code {exit_code}, took {took_human}, attempts {attempts_str})"
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

/// Render the streaming `ConvergedRunning` summary line — the
/// operator-facing exit-0 success render. Pure function.
///
/// Per slice 04 of `workload-kind-discriminator`: the function's sole
/// caller is the Service code path (post-WorkloadSpec discriminator),
/// so the rendered vocabulary names "Service". The legacy "Job"
/// vocabulary was renamed in a single-cut greenfield migration —
/// `JobSubmitEvent` carries no `ConvergedRunning` variant in the
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
/// the streaming `ConvergedRunning` summary. The output format is
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

/// Render the operator-facing alloc-status output, branching on
/// [`overdrive_core::aggregate::WorkloadKind`] per design [D4].
///
/// - Service: header + `Replicas (desired/running): N/M` + per-alloc
///   table with columns `Alloc / State / Restarts / Since` (NO Exit).
/// - Job: header (`Job '...' (kind: Job)`) + Verdict + per-attempt
///   table with columns `Attempt / State / Exit / Started / Duration`
///   + stderr tail on Failed.
/// - Schedule: header + cron + deferral note.
///
/// The match on `WorkloadKind` is EXHAUSTIVE per ADR-0047 §1 — no
/// catch-all wildcard. Future kinds require explicit arms.
#[must_use]
pub fn alloc_status_kind_aware(out: &AllocStatusResponse) -> String {
    use overdrive_core::aggregate::WorkloadKind;
    use std::fmt::Write as _;
    let kind = out.kind.unwrap_or(WorkloadKind::Service);
    let workload_name = out.workload_id.as_deref().unwrap_or("(unknown)");
    let spec_digest = out.spec_digest.as_deref().unwrap_or("");

    match kind {
        WorkloadKind::Service => {
            let mut s = String::new();
            let _ = writeln!(s, "Service '{workload_name}' (kind: Service)");
            if !spec_digest.is_empty() {
                let _ = writeln!(s, "Spec digest: {spec_digest}");
            }
            let _ = writeln!(
                s,
                "Replicas (desired/running): {}/{}",
                out.replicas_desired, out.replicas_running,
            );
            // Per-alloc table — Service columns: Alloc / State / Restarts / Since.
            let _ =
                writeln!(s, "{:<24} {:<12} {:<10} {:<20}", "Alloc", "State", "Restarts", "Since");
            // Restarts default to 0 in Phase 1 (per-alloc restart counter
            // not surfaced on the wire row body yet — this is a
            // forward-compat placeholder).
            for row in &out.rows {
                let since = row.started_at.as_deref().unwrap_or("\u{2014}");
                let _ = writeln!(
                    s,
                    "{:<24} {:<12} {:<10} {:<20}",
                    row.alloc_id,
                    state_label(row.state),
                    "0",
                    since,
                );
            }
            s
        }
        WorkloadKind::Job => {
            let mut s = String::new();
            let verdict = derive_job_verdict(&out.rows);
            s.push_str(&format_job_alloc_status_header(workload_name, spec_digest, verdict));
            s.push('\n');
            s.push_str(&format_job_alloc_status_attempts_table(&out.rows));
            // stderr tail on Failed: pull from the last attempt's
            // `error` field if present (the action shim threads
            // `prior_row.detail` / `prior_row.stderr_tail` onto the
            // wire row body's `error` field).
            if matches!(verdict, JobVerdict::Failed) {
                if let Some(last) = out.rows.last() {
                    if let Some(err) = &last.error {
                        if !err.is_empty() {
                            s.push('\n');
                            let _ = writeln!(s, "stderr (last lines):");
                            for line in err.lines() {
                                let _ = writeln!(s, "  {line}");
                            }
                        }
                    }
                }
            }
            s
        }
        WorkloadKind::Schedule => {
            // Schedule branch — minimal Phase-1 rendering. Slice 05
            // (job_submit_schedule) provides the deferral surface;
            // here we name the kind so the dispatcher is exhaustive.
            let mut s = String::new();
            let _ = writeln!(s, "Schedule '{workload_name}' (kind: Schedule)");
            if !spec_digest.is_empty() {
                let _ = writeln!(s, "Spec digest: {spec_digest}");
            }
            let _ = writeln!(s, "{}", crate::render::schedule::SCHEDULE_EXECUTION_TRACKING_URL);
            s
        }
    }
}

/// Render the streaming `ConvergedStopped` summary line — the
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
