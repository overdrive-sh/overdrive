// `TransitionReason` is the load-bearing single-source-of-truth enum from
// ADR-0032 §3 / ADR-0033 §1. Both the streaming `LifecycleEvent`
// surface and the snapshot `AllocStatusRowBody.last_transition.reason`
// surface serialise the SAME variant; byte-equality across surfaces is a
// structural property guaranteed by the type system, not by discipline.
//
// Variant taxonomy (ADR-0032 §3 amended 2026-04-30, cause-class refactor):
// the enum carries TWO classes of variant:
//
//   1. **Progress markers** — payload-less, name the lifecycle phase
//      (`Scheduling`, `Starting`, `Started`, `BackoffPending`, `Stopped`).
//      Emitted on healthy progress.
//
//   2. **Cause-class failure variants** — typed payloads naming the
//      structured cause (`ExecBinaryNotFound { path }`,
//      `ExecPermissionDenied { path }`, etc.). Emitted on failure
//      transitions; the payload IS the cause-specific data the operator
//      needs and a free-form `detail: String` cannot encode without
//      stringly-typing.
//
// The enum is NOT `Copy` (cause-class variants carry `String` /
// non-`Copy` payloads) and NOT `Hash` (same reason). Consumers that
// previously relied on `Copy` either clone (cheap for progress markers,
// owned-data for cause variants) or take by reference. The action shim
// is the single writer of `AllocStatusRow.reason` (cf. ADR-0023); the
// reconciler emits the variant on `Action::*` payloads at action emit
// time.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Structured reason for a lifecycle transition.
///
/// Phase 1 variants per ADR-0032 §3 (additive going forward — `#[non_exhaustive]`).
/// `#[serde(tag = "kind", content = "data", rename_all = "snake_case")]`
/// gives a self-describing wire shape: `{"kind": "exec_binary_not_found",
/// "data": {"path": "/usr/local/bin/payments"}}` for cause-class variants;
/// `{"kind": "scheduling"}` for progress markers (serde elides the empty
/// `data` for unit variants by default).
///
/// | Variant | Class | Emitted by | Phase 1 emit? |
/// |---|---|---|---|
/// | `Scheduling` | progress | reconciler — placement decided, action emitted | yes |
/// | `Starting` | progress | reconciler — driver invocation underway | yes |
/// | `Started` | progress | driver(exec) — driver returned `Ok(handle)` | yes |
/// | `BackoffPending { attempt }` | progress | reconciler — holding off restart | yes |
/// | `Stopped { by }` | progress | reconciler — observed terminal stop | yes |
/// | `ExecBinaryNotFound { path }` | cause | `ExecDriver` — `spawn(2)` ENOENT | yes |
/// | `ExecPermissionDenied { path }` | cause | `ExecDriver` — `spawn(2)` EACCES | yes |
/// | `ExecBinaryInvalid { path, kind }` | cause | `ExecDriver` — `spawn(2)` ENOEXEC / ELIBBAD | yes |
/// | `CgroupSetupFailed { kind, source }` | cause | `ExecDriver` — cgroup mkdir / write failure | yes |
/// | `DriverInternalError { detail }` | cause | `ExecDriver` — uncategorised driver failure | yes |
/// | `RestartBudgetExhausted { attempts, last_cause_summary }` | cause | reconciler — restart budget hit | yes |
/// | `Cancelled { by }` | cause | reconciler — operator stop intent observed | yes |
/// | `NoCapacity { requested, free }` | cause | reconciler — scheduler returned `NoCapacity` | yes |
/// | `OutOfMemory { peak_bytes, limit_bytes }` | cause | `ExecDriver` — cgroup OOM-killed | NO — Phase 2 |
/// | `WorkloadCrashedImmediately { exit_code, signal, stderr_tail }` | cause | `ExecDriver` — post-spawn exit-code observation | yes |
///
/// **Phase 2 emit-deferred variants**: `OutOfMemory` requires cgroup-events
/// subscription not yet present in the Phase 1 `ExecDriver`; it is defined
/// now for wire-shape forward-compatibility and will be emitted in Phase 2.
/// `WorkloadCrashedImmediately` is emitted in Phase 1 — `ExecDriver` already
/// performs `child.wait()` and produces `ExitKind::Crashed`; the
/// `ExitObserver` maps that to this variant directly.
///
/// **Cause-class payloads carry typed cause-specific data**, NOT a
/// free-form `detail: String`. The previous state-class shape relied on
/// `AllocStatusRow.detail: Option<String>` to encode the cause, which
/// stringly-typed every renderer and forced re-parsing on every read.
/// The cause-class refactor moves that data into the type system. The
/// `detail` field on the row remains for verbatim driver text the
/// payload does not capture (e.g. the raw `errno`-decorated message
/// from `std::io::Error::Display`).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransitionReason {
    // -----------------------------------------------------------------
    // Progress markers (payload-less or minimal payload)
    // -----------------------------------------------------------------
    /// Reconciler picked a placement; action was emitted.
    Scheduling,
    /// Driver invocation underway.
    Starting,
    /// Driver returned `Ok(handle)`.
    Started,
    /// Reconciler holding off restart per backoff window.
    /// `attempt` is the 1-indexed retry number that will fire when the
    /// backoff elapses (matches `WorkloadLifecycleView::restart_counts + 1`).
    BackoffPending { attempt: u32 },
    /// Reconciler observed terminal stop. `by` carries who initiated:
    /// `"operator"` for explicit stop intent, `"reconciler"` for
    /// converged terminal state.
    Stopped { by: StoppedBy },

    // -----------------------------------------------------------------
    // Cause-class failure variants (Phase 1 ExecDriver-observable)
    // -----------------------------------------------------------------
    /// `spawn(2)` returned ENOENT for the configured binary path.
    /// Replaces the previous state-class `DriverStartFailed` for the
    /// missing-binary case; the broken-binary regression target
    /// (US-02 KPI-02) emits this variant.
    ExecBinaryNotFound { path: String },
    /// `spawn(2)` returned EACCES — the binary exists but is not
    /// executable by the running uid.
    ExecPermissionDenied { path: String },
    /// `spawn(2)` returned ENOEXEC / ELIBBAD / similar — the file is
    /// not a valid executable for this kernel/architecture.
    /// `kind` carries the OS-reported sub-cause (e.g. `"not_executable"`,
    /// `"bad_elf"`, `"wrong_arch"`).
    ExecBinaryInvalid { path: String, kind: String },
    /// Cgroup setup failed (scope mkdir, PID enrolment, limit write).
    /// `kind` is one of `"create_scope"`, `"place_pid"`,
    /// `"write_limits"`; `source` is the verbatim `std::io::Error`
    /// `Display`.
    CgroupSetupFailed { kind: String, source: String },
    /// Driver returned an uncategorised failure that did not fit any
    /// of the more specific cause variants. Falls back on the verbatim
    /// driver `Display` text in `detail`. Operators seeing this variant
    /// have a signal to file an issue — the driver should grow a more
    /// specific variant.
    DriverInternalError { detail: String },
    /// Reconciler hit restart budget; will not emit further restart
    /// actions for this alloc id. `last_cause_summary` carries the
    /// `human_readable()` rendering of the most recent failure cause-
    /// variant the reconciler observed, so the operator sees both
    /// "we gave up" and "this is what kept failing" in one transition.
    /// `attempts` is the count of attempts made (= the budget max in
    /// Phase 1, hard-coded to 5).
    ///
    /// **Why `String` and not `Box<TransitionReason>`**: rkyv's
    /// `Archive` derive cannot resolve a recursive enum — the
    /// archived-size computation overflows. The reconciler renders the
    /// prior cause via `human_readable()` at observe time; the
    /// rendered prose IS the auditable artifact, and the structured
    /// per-attempt history (cause-of-each-attempt) lives in the
    /// reconciler's private libSQL view (`NextView`), not on the wire.
    /// The wire only carries the terminal "we gave up because of X"
    /// summary.
    RestartBudgetExhausted { attempts: u32, last_cause_summary: String },
    /// Operator submitted a stop intent and the reconciler converged
    /// the allocation to the terminal state in response.
    /// `by` distinguishes operator stop from cluster-driven stop
    /// (e.g. node drain — Phase 2+).
    Cancelled { by: CancelledBy },
    /// Scheduler returned `NoCapacity`. Carries the requested vs free
    /// resource envelope at the time of the placement attempt; the
    /// previous string-formatted "requested X / free Y" diagnostic
    /// becomes typed structured data.
    NoCapacity { requested: ResourceEnvelope, free: ResourceEnvelope },

    // -----------------------------------------------------------------
    // Cause-class failure variants (Phase 2 emit-deferred)
    // -----------------------------------------------------------------
    /// Cgroup OOM-killed the workload. Requires Phase 2 `ExecDriver`
    /// cgroup-events subscription; defined now for wire-shape forward-
    /// compatibility.
    OutOfMemory { peak_bytes: u64, limit_bytes: u64 },
    /// Workload exited with a non-zero status or signal within the
    /// post-spawn settle window. Emitted by the `ExitObserver` when
    /// `ExitKind::Crashed` is received from the driver — `ExecDriver`
    /// already performs `child.wait()` in Phase 1 and that is exactly
    /// how `ExitKind::Crashed` is produced. The `exit_code` field
    /// carries the process exit status (`None` for signal-only exits);
    /// `signal` carries the signal number cast to `u8` (`None` when
    /// absent); `stderr_tail` carries the last few lines of stderr
    /// captured by the driver's ring buffer.
    WorkloadCrashedImmediately {
        exit_code: Option<i32>,
        signal: Option<u8>,
        stderr_tail: Option<String>,
    },
    /// The per-alloc transparent-mTLS intercept could not be installed, so the
    /// alloc is failed fail-closed rather than run with cleartext (D-MTLS-18).
    /// `stage` is one of `"outbound_attach"`, `"leg_f_bind"`,
    /// `"leg_c_transparent_listener"`, `"inbound_tproxy"` — the install step
    /// that failed; `detail` is the verbatim `Display` of the underlying
    /// `MtlsInterceptInstallError` (which names the privilege / kernel-feature
    /// remediation an operator acts on). Mirrors the `CgroupSetupFailed {
    /// kind, source }` cause-class shape — `stage` is a `String` carrying a
    /// closed vocabulary, NOT a sub-enum.
    MtlsInterceptInstallFailed { stage: String, detail: String },
    /// The alloc's per-workload network namespace + veth could NOT be
    /// provisioned (or a free network slot could not be assigned), so the alloc
    /// is failed fail-closed rather than spawned without its netns
    /// (transparent-mtls-enrollment D-TME-12 / AC14, Path A / ADR-0071). Unlike
    /// `MtlsInterceptInstallFailed` — which supersedes an already-`Running` row
    /// when the post-spawn intercept install fails — this fires at the
    /// PRE-`Running` provision seam (the provision precedes `Driver::start`), so
    /// the alloc never reached Running and a persistent provision failure (slot
    /// exhaustion, EPERM creating the netns/veth) drives the alloc to `Failed`
    /// instead of looping `Pending` forever. `stage` is one of
    /// `"net_slot_assign"` (no free slot) or `"netns_provision"` (the netns/veth
    /// shell-out failed); `detail` is the verbatim `Display` of the underlying
    /// `NetSlotExhausted` / `VethProvisionError`. Mirrors the
    /// `MtlsInterceptInstallFailed { stage, detail }` cause-class shape — `stage`
    /// is a `String` carrying a closed vocabulary, NOT a sub-enum.
    WorkloadNetnsProvisionFailed { stage: String, detail: String },
}

/// Initiator of a `Stopped` transition.
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
pub enum StoppedBy {
    /// Operator submitted explicit stop intent.
    Operator,
    /// Reconciler converged the allocation to a terminal state (the
    /// reconciler actioned a stop, not the process itself).
    Reconciler,
    /// The workload process exited naturally (clean exit with no stop
    /// intent from the operator or reconciler).
    ///
    /// **Additive position**: variants are append-only to preserve
    /// pre-existing rkyv discriminants (`Operator=0`, `Reconciler=1`,
    /// `Process=2`). New variants land at the tail of the
    /// discriminant space; existing archived rows decode unchanged.
    Process,
    /// The system garbage-collected an allocation whose desired
    /// intent disappeared (per ADR-0037 Amendment 2026-05-14;
    /// `workload-gc-absent-stale-allocs`). Distinct from
    /// [`Self::Reconciler`] (which represents an explicit
    /// reconciler-actioned stop with desired intent still present)
    /// — `SystemGc` records that the allocation was withdrawn
    /// because no operator intent referenced it any longer.
    ///
    /// Appended after `Process` to keep the pre-existing rkyv
    /// discriminants stable. This variant takes discriminant `3`.
    /// Existing archived rows decode unchanged.
    SystemGc,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_by_process_human_readable() {
        let reason = TransitionReason::Stopped { by: StoppedBy::Process };
        assert_eq!(reason.human_readable(), "stopped (by process)");
    }

    #[test]
    fn is_failure_false_for_stopped_variants() {
        assert!(!TransitionReason::Stopped { by: StoppedBy::Operator }.is_failure());
        assert!(!TransitionReason::Stopped { by: StoppedBy::Reconciler }.is_failure());
        assert!(!TransitionReason::Stopped { by: StoppedBy::Process }.is_failure());
        assert!(!TransitionReason::Stopped { by: StoppedBy::SystemGc }.is_failure());
    }

    #[test]
    fn is_failure_true_for_cause_class_variants() {
        assert!(TransitionReason::DriverInternalError { detail: "test".into() }.is_failure());
        assert!(
            TransitionReason::RestartBudgetExhausted {
                attempts: 3,
                last_cause_summary: "signal 9".into(),
            }
            .is_failure()
        );
    }

    // ----------------------------------------------------------------
    // ADR-0037 prerequisite — `TerminalCondition` enum surface tests.
    // The variant equality cases below pin the closed first-party
    // shape (`BackoffExhausted`, `Stopped`, `Custom`); the rkyv
    // roundtrip property at the row level lives in
    // `tests/acceptance/terminal_condition_roundtrip.rs`.
    // ----------------------------------------------------------------

    #[test]
    fn terminal_condition_backoff_exhausted_carries_attempts() {
        let a = TerminalCondition::BackoffExhausted { attempts: 5 };
        let b = TerminalCondition::BackoffExhausted { attempts: 5 };
        let c = TerminalCondition::BackoffExhausted { attempts: 6 };
        assert_eq!(a, b, "equal-attempts BackoffExhausted variants must compare equal");
        assert_ne!(a, c, "differing-attempts BackoffExhausted variants must compare unequal");
    }

    #[test]
    fn terminal_condition_stopped_reuses_existing_stopped_by() {
        let by_op = TerminalCondition::Stopped { by: StoppedBy::Operator };
        let by_re = TerminalCondition::Stopped { by: StoppedBy::Reconciler };
        let by_pr = TerminalCondition::Stopped { by: StoppedBy::Process };
        assert_ne!(by_op, by_re, "Operator vs Reconciler must compare unequal");
        assert_ne!(by_re, by_pr, "Reconciler vs Process must compare unequal");
    }

    #[test]
    fn terminal_condition_custom_carries_type_name_and_optional_detail() {
        let with_payload = TerminalCondition::Custom {
            type_name: "vendor.io/quota.QuotaExhausted".to_owned(),
            detail: Some(vec![1, 2, 3]),
        };
        let same = TerminalCondition::Custom {
            type_name: "vendor.io/quota.QuotaExhausted".to_owned(),
            detail: Some(vec![1, 2, 3]),
        };
        let no_detail = TerminalCondition::Custom {
            type_name: "vendor.io/quota.QuotaExhausted".to_owned(),
            detail: None,
        };
        assert_eq!(with_payload, same, "structurally identical Custom must compare equal");
        assert_ne!(
            with_payload, no_detail,
            "Custom with vs without detail payload must compare unequal"
        );
    }
}

/// Reconciler-emitted classification of *why* an allocation reached a
/// terminal lifecycle state.
///
/// Per ADR-0037 §1, this enum is the *publication boundary* between
/// reconciler-private View state (`restart_counts`,
/// `last_failure_seen_at`, the live backoff policy) and downstream
/// consumers (the durable `AllocStatusRow.terminal` field, the
/// streaming `LifecycleEvent.terminal` field, the HTTP
/// `RestartBudget.exhausted` projection). The reconciler's *decision*
/// rides on this type — it is not a derived value computed by
/// downstream consumers from inputs they would otherwise need to read
/// out of reconciler memory. See `.claude/rules/development.md`
/// §"Persist inputs, not derived state" for the layering rule the
/// ADR honours.
///
/// # Variants
///
/// - [`Self::BackoffExhausted`] — `WorkloadLifecycle` reached its restart
///   budget at the deciding tick. `attempts` is the count *consumed*
///   at that moment (in Phase 1, the budget is hard-coded; in
///   future phases the same variant carries the post-policy attempts).
/// - [`Self::Stopped`] — explicit operator stop converged. The
///   allocation reached `Stopped` because the operator (or the
///   reconciler itself) requested it, not because of a failure. The
///   inner [`StoppedBy`] reuses ADR-0032's existing initiator enum.
/// - [`Self::Custom`] — forward-compat for WASM third-party
///   reconcilers per whitepaper §18 (*Extension Model*). `type_name`
///   is a CamelCase identifier scoped by the reconciler's canonical
///   name (e.g. `"vendor.io/quota.QuotaExhausted"`); `detail` is
///   opaque rkyv-encoded bytes the reconciler may attach. Streaming
///   forwards `Custom` verbatim; well-known first-party variants stay
///   in the closed set above and are compile-time-checked at every
///   consumer.
///
/// # `SemVer` convention
///
/// Per ADR-0037 §3 (mirroring K8s `Condition.Reason` shape):
///
/// - **Adding a new well-known variant** — additive minor; existing
///   consumers use [`Self::Custom`] / a wildcard arm + warn-and-skip
///   shape until they explicitly handle the new variant.
/// - **Renaming or removing a variant** — major-bump breaking change.
///   The `#[non_exhaustive]` attribute on this enum is what makes the
///   minor-bump path safe: external `match` sites are required to
///   carry a wildcard arm, so adding a new variant cannot silently
///   change their behaviour.
///
/// # Field-shape rationale
///
/// `String` (not `Box<TerminalCondition>` or a recursive enum) is
/// chosen for `Custom.type_name` for the same reason
/// `RestartBudgetExhausted.last_cause_summary` is `String` on
/// [`TransitionReason`]: rkyv's `Archive` derive cannot resolve a
/// recursive enum, and the type-name is meant to be opaque to the
/// platform anyway — the reconciler emits a stable string id, the
/// consumer renders it. `Option<Vec<u8>>` for `detail` mirrors
/// rkyv-supported sum types and lets a reconciler attach a structured
/// payload if it has one without forcing every emitter to.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalCondition {
    /// `WorkloadLifecycle`: restart budget reached; no further attempts
    /// will be scheduled. `attempts` is the count consumed at the
    /// moment of the deciding tick.
    BackoffExhausted { attempts: u32 },
    /// `WorkloadLifecycle`: explicit operator stop converged. The
    /// allocation reached `Stopped` because the operator (or the
    /// reconciler itself) requested it, not because of a failure.
    Stopped { by: StoppedBy },
    /// Forward-compat for WASM third-party reconcilers per
    /// whitepaper §18. `type_name` is a CamelCase identifier scoped
    /// by the reconciler (e.g. `"vendor.io/quota.QuotaExhausted"`);
    /// `detail` is opaque bytes the reconciler may attach.
    Custom { type_name: String, detail: Option<Vec<u8>> },
    /// `WorkloadLifecycle`: workload exited cleanly (Job-kind natural
    /// termination, exit code `0` is the canonical success but the
    /// variant carries the observed `exit_code` verbatim because the
    /// publication boundary owns the cleanliness classification —
    /// downstream consumers must not redo the comparison from a row
    /// they no longer hold the policy for). Per ADR-0037 Amendment
    /// 2026-05-10: typed natural-exit terminals replace the previous
    /// reliance on `AllocStatusRow.exit_code` + heuristic mapping at
    /// every consumer.
    ///
    /// Emission lands in slice 02-04 (WorkloadLifecycle reconciler
    /// natural-exit emission); the row-shape change lands in 02-05.
    /// This variant exists at the type level from slice 02a (this
    /// step) so every downstream `match` site is forward-compatible
    /// with the additive shape ahead of runtime emission.
    ///
    /// **Additive position**: appended after `Custom` to keep the
    /// pre-existing rkyv discriminants (`BackoffExhausted=0`,
    /// `Stopped=1`, `Custom=2`) stable. This variant takes
    /// discriminant `3`, `Failed` takes discriminant `4`. Existing
    /// archived rows decode unchanged.
    Completed { exit_code: i32 },
    /// `WorkloadLifecycle`: workload exited with a non-zero status (Job-kind
    /// natural termination interpreted as failure by the reconciler).
    /// Per ADR-0037 Amendment 2026-05-10. The `exit_code` field
    /// carries the observed status verbatim — the reconciler does
    /// the success/failure classification at the publication
    /// boundary and the variant identity (`Completed` vs `Failed`)
    /// IS the classification. Downstream consumers branch on the
    /// variant, never on `exit_code != 0`.
    ///
    /// Common Unix exit codes operators see in this variant:
    /// `1` (generic failure), `127` (command-not-found),
    /// `137` (SIGKILL — typically OOM under cgroup-v2),
    /// `255` (generic shell failure). The full `i32` range is
    /// supported so the variant can carry signal-encoded statuses
    /// if a future driver emits them. `None` means the process was
    /// killed by a signal without producing an exit code (OOM-killer,
    /// external SIGKILL).
    Failed { exit_code: Option<i32> },
    /// `ServiceLifecycle`: the Service alloc has converged to a
    /// stable, fully-probed Running state per ADR-0055 / ADR-0056.
    /// All declared startup probes (or the inferred default per
    /// ADR-0058) reported Pass; the alloc is considered Stable
    /// from the publication boundary onward.
    ///
    /// `settled_in_ms` is the elapsed wall-clock duration (in
    /// milliseconds) from alloc start to the deciding tick that
    /// observed the last-to-Pass startup probe. `witness` names
    /// which probe's Pass moved the reconciler to Stable (see
    /// [`ProbeWitness`] for the per-field semantics).
    ///
    /// **Additive position**: appended after `Failed` to keep the
    /// pre-existing rkyv discriminants (`BackoffExhausted=0`,
    /// `Stopped=1`, `Custom=2`, `Completed=3`, `Failed=4`) stable.
    /// This variant takes discriminant `5`. Existing archived rows
    /// decode unchanged (the canonical `Option<TerminalCondition>`
    /// embeddings in `AllocStatusRowV1` continue to decode through
    /// the same `AllocStatusRowEnvelope::V1` shape; the V1 fixture
    /// is regenerated against the new canonical layout per
    /// `.claude/rules/development.md` § "rkyv schema evolution"
    /// greenfield single-cut exception, because no shipped consumer
    /// has persisted bytes against the pre-existing V1 layout).
    Stable { settled_in_ms: u64, witness: ProbeWitness },
    /// `ServiceLifecycle`: the Service alloc transitioned to a
    /// terminal Failed state per ADR-0055 / ADR-0056. The `reason`
    /// carries the typed failure cause (startup-timeout / startup-
    /// probe-failed / early-exit / liveness-probe-failed); see
    /// [`ServiceFailureReason`] for the per-variant semantics.
    ///
    /// **Additive position**: appended after `Stable` and takes
    /// discriminant `6`. Same greenfield rationale as `Stable`.
    ServiceFailed { reason: ServiceFailureReason },
}

// Companion types `ServiceFailureReason` and `ProbeWitness` are
// defined below. `TerminalCondition::Stable` / `::ServiceFailed`
// carry these types as payloads; the Service-lifecycle reconciler
// (slice 01-03+ per ADR-0055) is the sole emitter, and downstream
// consumers branch on the variant for operator-facing render.

/// Reason a Service alloc transitioned to a terminal Failed state.
///
/// Per ADR-0055 § 4 + ADR-0056 (wire projection): single
/// `#[non_exhaustive]` enum; additive variants only.
///
/// Lives in `transition_reason.rs` (and is re-exported from
/// `service_lifecycle.rs`) so it can be carried inside
/// [`TerminalCondition::ServiceFailed`] without inducing a module-
/// dependency cycle. The Service-lifecycle reconciler is the sole
/// constructor; downstream consumers branch on the variant for
/// operator-facing render.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(tag = "reason", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceFailureReason {
    /// Startup deadline elapsed before any successful probe.
    /// `probe_idx` names the last-attempted probe; `attempts`
    /// records how many attempts were made.
    StartupTimeout { probe_idx: u32, attempts: u32 },
    /// Startup probe exhausted `max_attempts` without a Pass result
    /// within `startup_deadline`. `last_fail` carries the last
    /// observed `ProbeStatus::Fail.last_fail_reason` for direct
    /// operator-renderable surface.
    StartupProbeFailed { probe_idx: u32, last_fail: String, attempts: u32 },
    /// Workload exited before any startup probe could pass AND
    /// within `startup_deadline` window. Closes RCA-A coinflip
    /// case per US-08.
    EarlyExit { exit_code: Option<i32> },
    /// Liveness probe consecutive-failure count reached its
    /// threshold AND restart-budget reached
    /// `RESTART_BACKOFF_CEILING`. Composes with the existing
    /// `BackoffExhausted` JobLifecycle pathway for Service-kind
    /// liveness-driven restart attempts.
    LivenessProbeFailed { probe_idx: u32, attempts: u32 },
    /// Service-kind workload exhausted the general restart-attempt
    /// budget (`WorkloadLifecycleView::restart_counts >=
    /// RESTART_BACKOFF_CEILING`). Distinct from
    /// [`Self::LivenessProbeFailed`] (which fires when liveness
    /// consecutive-failures + restart-budget jointly exhaust).
    ///
    /// `attempts` is the count at the deciding tick. `cause`
    /// disambiguates the budget that ran out; `last_exit_code` is
    /// read from the latest `AllocStatusRow.exit_code` observation
    /// at projection time. Per ADR-0059 Q2.
    BackoffExhausted { attempts: u32, cause: BackoffCause, last_exit_code: Option<i32> },
    /// Fallback projection for third-party WASM reconciler terminals
    /// (`TerminalCondition::Custom`). `source` carries the
    /// reconciler's canonical name (`type_name`); `message` carries
    /// the rendered detail bytes (UTF-8 if valid, lowercase-hex
    /// otherwise). Per ADR-0059 Q3.
    Other { source: String, message: String },
    /// Streaming-loop wall-clock cap timer expired before any
    /// terminal arrived. Synthesised by `build_stream`; the
    /// reconciler MUST NOT emit this variant. Per ADR-0059 Q4.
    Timeout { after_seconds: u32 },
    /// Broadcast channel closed mid-stream (server shutdown / action
    /// shim teardown). Synthesised by `build_stream`; the reconciler
    /// MUST NOT emit this variant. Per ADR-0059 Q4.
    StreamInterrupted,
}

/// Disambiguator on [`ServiceFailureReason::BackoffExhausted`]
/// naming the budget that ran out. Per ADR-0059 Q2.
///
/// Phase 1 emits only [`Self::AttemptBudget`].
/// [`Self::LivenessBudget`] is defined for forward-compat with
/// the future `LivenessRestartGovernor` (ADR-0055 §7) — Phase 2+.
/// Including the discriminator now keeps the wire shape additive
/// without splitting the variant later.
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
pub enum BackoffCause {
    /// General post-spawn crash loop — driver exit + WorkloadLifecycle
    /// restart budget ran out. Phase 1 emit site.
    AttemptBudget,
    /// Liveness-probe-driven restart budget ran out. Reserved for
    /// Phase 2+ when the `LivenessRestartGovernor` (ADR-0055 §7)
    /// lands; today the `LivenessProbeFailed` variant covers this
    /// case end-to-end.
    LivenessBudget,
}

/// Names which probe's Pass moved the reconciler to Stable.
///
/// Per DDD-7 (multi-probe AND-of-all semantic): when N startup
/// probes are declared, all must Pass; the witness names the
/// **last-to-Pass** probe. Renderer surfaces as `witness:
/// startup probe #<idx> (<mechanic_summary>)`.
///
/// Lives in `transition_reason.rs` (and is re-exported from
/// `service_lifecycle.rs`) so it can be carried inside
/// [`TerminalCondition::Stable`] without inducing a module-
/// dependency cycle.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct ProbeWitness {
    /// 0-indexed position of the witnessing probe within its role
    /// array.
    pub probe_idx: u32,
    /// Operator-facing role name (`"startup"` / `"readiness"` /
    /// `"liveness"`). Carried as `String` to keep
    /// `transition_reason.rs` decoupled from `observation::ProbeRole`
    /// (which lives in a separate module — projection happens at the
    /// reconciler's emission site).
    pub role: String,
    /// Operator-facing summary (e.g. `"tcp 0.0.0.0:8080"`,
    /// `"http GET http://0.0.0.0:8080/healthz"`,
    /// `"exec /usr/local/bin/healthcheck.sh"`). Reconciler
    /// composes from `ProbeDescriptor.mechanic` at the deciding
    /// tick.
    pub mechanic_summary: String,
    /// `true` IFF this witness was the platform's inferred default
    /// probe per ADR-0058.
    pub inferred: bool,
}

/// Initiator of a `Cancelled` transition.
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
pub enum CancelledBy {
    /// Operator submitted explicit stop intent.
    Operator,
    /// Cluster-driven cancellation (Phase 2+: node drain, eviction).
    Cluster,
}

/// Resource envelope carried by the `NoCapacity` cause variant.
///
/// Mirrors the production `Resources` shape from
/// `overdrive_core::traits::driver` but is defined here to keep the
/// `TransitionReason` self-contained without pulling the full driver
/// trait surface into wire-typed contexts. The crafter wires the
/// `From<&Resources> for ResourceEnvelope` projection in slice 01
/// GREEN.
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
pub struct ResourceEnvelope {
    pub cpu_milli: u32,
    pub memory_bytes: u64,
}

impl TransitionReason {
    /// Human-readable rendering for the snapshot's `Last transition:`
    /// block (ADR-0033 §4 mapping table). The streaming surface
    /// serialises the `snake_case` discriminator + structured payload
    /// via serde; the CLI renderer maps the enum to this human-readable
    /// shape on the snapshot side AND on the streaming-line render side
    /// (operators see the same prose in both surfaces).
    ///
    /// Returns `String` rather than `&'static str` because cause-class
    /// variants interpolate their payloads (`"binary not found:
    /// /usr/local/bin/payments"`); progress markers return owned copies
    /// of static strings to keep the return type uniform.
    ///
    /// Reference rendering shapes (per ADR-0033 §4 amendment 2026-04-30):
    ///
    /// | Variant | Rendering |
    /// |---|---|
    /// | `Scheduling` | `"scheduling"` |
    /// | `Starting` | `"starting"` |
    /// | `Started` | `"driver started"` |
    /// | `BackoffPending { attempt }` | `format!("backoff (attempt {attempt})")` |
    /// | `Stopped { by: Operator }` | `"stopped (by operator)"` |
    /// | `Stopped { by: Reconciler }` | `"stopped"` |
    /// | `Stopped { by: Process }` | `"stopped (by process)"` |
    /// | `ExecBinaryNotFound { path }` | `format!("binary not found: {path}")` |
    /// | `ExecPermissionDenied { path }` | `format!("permission denied: {path}")` |
    /// | `ExecBinaryInvalid { path, kind }` | `format!("binary invalid ({kind}): {path}")` |
    /// | `CgroupSetupFailed { kind, source }` | `format!("cgroup {kind} failed: {source}")` |
    /// | `DriverInternalError { detail }` | `format!("driver internal error: {detail}")` |
    /// | `RestartBudgetExhausted { attempts, last_cause_summary }` | `format!("restart budget exhausted after {attempts} attempts (last: {last_cause_summary})")` |
    /// | `Cancelled { by: Operator }` | `"cancelled (by operator)"` |
    /// | `Cancelled { by: Cluster }` | `"cancelled (by cluster)"` |
    /// | `NoCapacity { requested, free }` | `format!("no capacity (requested {requested:?} / free {free:?})")` |
    /// | `OutOfMemory { peak_bytes, limit_bytes }` | `format!("OOM-killed (peak {peak_bytes} / limit {limit_bytes})")` |
    /// | `WorkloadCrashedImmediately { exit_code, signal, .. }` | `format!("crashed (exit {exit_code:?}, signal {signal:?})")` |
    #[must_use]
    pub fn human_readable(&self) -> String {
        match self {
            // Progress markers
            Self::Scheduling => "scheduling".to_owned(),
            Self::Starting => "starting".to_owned(),
            Self::Started => "driver started".to_owned(),
            Self::BackoffPending { attempt } => {
                format!("backoff (attempt {attempt})")
            }
            Self::Stopped { by: StoppedBy::Operator } => "stopped (by operator)".to_owned(),
            Self::Stopped { by: StoppedBy::Reconciler } => "stopped".to_owned(),
            Self::Stopped { by: StoppedBy::Process } => "stopped (by process)".to_owned(),
            Self::Stopped { by: StoppedBy::SystemGc } => "stopped (by system gc)".to_owned(),

            // Cause-class failures (Phase 1 emit)
            Self::ExecBinaryNotFound { path } => format!("binary not found: {path}"),
            Self::ExecPermissionDenied { path } => format!("permission denied: {path}"),
            Self::ExecBinaryInvalid { path, kind } => {
                format!("binary invalid ({kind}): {path}")
            }
            Self::CgroupSetupFailed { kind, source } => {
                format!("cgroup {kind} failed: {source}")
            }
            Self::DriverInternalError { detail } => {
                format!("driver internal error: {detail}")
            }
            Self::RestartBudgetExhausted { attempts, last_cause_summary } => {
                format!(
                    "restart budget exhausted after {attempts} attempts (last: {last_cause_summary})",
                )
            }
            Self::Cancelled { by: CancelledBy::Operator } => "cancelled (by operator)".to_owned(),
            Self::Cancelled { by: CancelledBy::Cluster } => "cancelled (by cluster)".to_owned(),
            Self::NoCapacity { requested, free } => {
                format!(
                    "no capacity (requested cpu={req_cpu}m mem={req_mem}b / free cpu={free_cpu}m mem={free_mem}b)",
                    req_cpu = requested.cpu_milli,
                    req_mem = requested.memory_bytes,
                    free_cpu = free.cpu_milli,
                    free_mem = free.memory_bytes,
                )
            }

            // Cause-class failures (Phase 2 emit-deferred forward-compat)
            Self::OutOfMemory { peak_bytes, limit_bytes } => {
                format!("OOM-killed (peak {peak_bytes} / limit {limit_bytes})")
            }
            Self::WorkloadCrashedImmediately { exit_code, signal, .. } => {
                format!("crashed (exit {exit_code:?}, signal {signal:?})")
            }
            Self::MtlsInterceptInstallFailed { stage, detail } => {
                format!("mTLS intercept install failed ({stage}): {detail}")
            }
            Self::WorkloadNetnsProvisionFailed { stage, detail } => {
                format!("workload netns provision failed ({stage}): {detail}")
            }
        }
    }

    /// Returns `true` for cause-class variants (failure transitions);
    /// `false` for progress markers.
    ///
    /// Useful for renderers that distinguish "tell me the phase" from
    /// "tell me what went wrong." The streaming `LifecycleTransition`
    /// line carries either class; the snapshot's `last_transition`
    /// renders both with the same `human_readable()` output but the
    /// CLI's `Error:` block in `submit` only fires on cause-class
    /// terminal events.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        match self {
            // Progress markers — healthy lifecycle progress.
            Self::Scheduling
            | Self::Starting
            | Self::Started
            | Self::BackoffPending { .. }
            | Self::Stopped { .. } => false,
            // Cause-class failures.
            Self::ExecBinaryNotFound { .. }
            | Self::ExecPermissionDenied { .. }
            | Self::ExecBinaryInvalid { .. }
            | Self::CgroupSetupFailed { .. }
            | Self::DriverInternalError { .. }
            | Self::RestartBudgetExhausted { .. }
            | Self::Cancelled { .. }
            | Self::NoCapacity { .. }
            | Self::OutOfMemory { .. }
            | Self::WorkloadCrashedImmediately { .. }
            | Self::MtlsInterceptInstallFailed { .. }
            | Self::WorkloadNetnsProvisionFailed { .. } => true,
        }
    }
}
