// SCAFFOLD: true
//
// RED scaffold per nw-distill / .claude/rules/testing.md §"RED scaffolds and
// intentionally-failing commits". Created during the DISTILL wave for feature
// `cli-submit-vs-deploy-and-alloc-status`; the crafter replaces this stub
// with the real implementation as part of slice 01 in DELIVER.
//
// `TransitionReason` is the load-bearing single-source-of-truth enum from
// ADR-0032 §3 / ADR-0033 §1. Both the streaming `SubmitEvent::LifecycleTransition`
// surface and the snapshot `AllocStatusRowBody.last_transition.reason`
// surface serialise the SAME variant; byte-equality across surfaces is a
// structural property guaranteed by the type system, not by discipline.
//
// The action shim writes `Option<TransitionReason>` into
// `AllocStatusRow.reason` as part of the row-write amendment in slice 01;
// the field-extension on `AllocStatusRow` is deferred from this DISTILL
// scaffold to the crafter's GREEN-phase work because adding a field to a
// load-bearing type breaks compilation across the workspace, which would
// classify the scaffold as BROKEN rather than RED. See
// `docs/feature/cli-submit-vs-deploy-and-alloc-status/distill/wave-decisions.md`
// DWD-03.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Structured reason for a lifecycle transition.
///
/// Phase 1 variants per ADR-0032 §3 (additive going forward — `#[non_exhaustive]`):
///
/// | Variant | Emitted by |
/// |---|---|
/// | `Scheduling` | reconciler — placement decided, action emitted |
/// | `Starting` | reconciler — driver invocation underway |
/// | `Started` | driver(exec) — driver returned `Ok(handle)` |
/// | `DriverStartFailed` | driver(exec) — driver returned `StartRejected` |
/// | `BackoffPending` | reconciler — holding off restart |
/// | `BackoffExhausted` | reconciler — restart budget hit |
/// | `Stopped` | reconciler — observed terminal stop |
/// | `NoCapacity` | reconciler — scheduler returned `NoCapacity` |
///
/// Verbatim driver text (e.g. `"stat /no/such: no such file or directory"`)
/// lives in the row's `detail: Option<String>` field, NOT in this enum.
/// This separation is intentional: the enum carries the structured class;
/// the detail carries opaque diagnostic text.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransitionReason {
    /// Reconciler picked a placement; action was emitted.
    Scheduling,
    /// Driver invocation underway.
    Starting,
    /// Driver returned `Ok(handle)`.
    Started,
    /// Driver returned `StartRejected`. Verbatim driver text lives in
    /// the row's `detail` field; this variant only signals the class.
    DriverStartFailed,
    /// Reconciler holding off restart per backoff window.
    BackoffPending,
    /// Reconciler hit restart budget; will not emit further restart
    /// actions for this alloc id.
    BackoffExhausted,
    /// Reconciler observed terminal stop (operator stop intent, or
    /// converged terminal state).
    Stopped,
    /// Scheduler returned `NoCapacity`. Verbatim "requested X / free Y"
    /// text lives in the row's `detail`.
    NoCapacity,
}

impl TransitionReason {
    /// Human-readable rendering for the snapshot's `Last transition:`
    /// block (ADR-0033 §4 mapping table). The streaming surface
    /// serialises the `snake_case` discriminator via serde; the CLI
    /// renderer maps the enum to this human-readable shape.
    ///
    /// The bodies below MUST remain panicking under this scaffold.
    /// The crafter replaces the panic with the real mapping in slice
    /// 01 GREEN.
    #[must_use]
    pub fn human_readable(self) -> &'static str {
        // RED scaffold — replaced by the real mapping during DELIVER
        // slice 01. The crafter wires per-variant string literals from
        // ADR-0033 §4 ("scheduling", "starting", "driver started",
        // "driver start failed", "backoff (attempt N)",
        // "backoff exhausted", "stopped", "no capacity").
        let _ = self;
        panic!("Not yet implemented -- RED scaffold")
    }
}
