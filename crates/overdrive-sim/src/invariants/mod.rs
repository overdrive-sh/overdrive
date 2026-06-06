//! DST invariant catalogue.
//!
//! The [`Invariant`] enum is the canonical name source for `--only <NAME>`
//! on `cargo dst` and for every invariant entry in
//! `target/dst/summary.json`. `Display` emits kebab-case, lowercase;
//! [`FromStr`] accepts any ASCII-case spelling of a canonical name. A name
//! printed by the harness MUST round-trip losslessly through
//! `FromStr → Display` — the proptest in
//! `crates/overdrive-sim/tests/invariant_roundtrip.rs` enforces that.
//!
//! Phase 1 ships the catalogue definition and canonical-name machinery.
//! The invariant *evaluators* — the code that decides whether an
//! invariant holds in a given run — land in step 06-02. Every name in
//! this enum is already known to `cargo dst`, so CI wiring and
//! artifact shape are stable even before the evaluators exist.

#![allow(clippy::missing_errors_doc)]

use std::fmt::{self, Display};
use std::str::FromStr;

/// Build a TCP `ServiceFrontend` for `vip` on a fixed listener port —
/// the default shape for invariant evaluators that exercised the legacy
/// proto-agnostic `update_service(vip, ...)` surface. The port (8080) is
/// not load-bearing for these invariants; only the VIP and (TCP) proto
/// drive the observed reverse-NAT/forward state.
#[must_use]
pub(crate) fn tcp_frontend(vip: std::net::Ipv4Addr) -> overdrive_core::dataplane::ServiceFrontend {
    let service_vip = overdrive_core::id::ServiceVip::new(std::net::IpAddr::V4(vip))
        .unwrap_or_else(|_| unreachable!("ServiceVip::new is total over IPv4"));
    overdrive_core::dataplane::ServiceFrontend::new(
        service_vip,
        std::num::NonZeroU16::new(8080).unwrap_or_else(|| unreachable!("8080 is non-zero")),
        overdrive_core::dataplane::backend_key::Proto::Tcp,
    )
    .unwrap_or_else(|_| unreachable!("IPv4 ServiceFrontend constructs"))
}

pub mod evaluators;
// phase-2-xdp-service-map Slice 03 (US-03; S-2.2-09). The
// `BackendSetSwapAtomic` invariant pins the SimDataplane's
// `update_service` to a single mutex-guarded reassignment so
// observers see either the pre- or post-swap backend set,
// never a torn state. Mirrors the production `EbpfDataplane`'s
// atomic HASH_OF_MAPS outer-map swap.
pub mod backend_set_swap_atomic;
// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-13 sibling).
// The `MaglevDistributionEven` invariant pins the steady-state
// distribution property of `maglev::generate` — under equal
// weights, every backend occupies its expected share ±5 %. The
// disruption-bound proptest at `tests/integration/maglev_churn.rs`
// pins the churn property; this invariant pins the distribution
// property, both ride on the same pure function.
pub mod maglev_distribution;
// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-14 sibling).
// The `MaglevDeterministic` invariant pins the K3 twin-run identity
// property of `maglev::generate` — two calls with identical inputs
// return bit-identical `Vec<BackendId>` outputs. Sibling to
// `MaglevDistributionEven`: that invariant pins the steady-state
// distribution property, this one pins the determinism property.
pub mod maglev_deterministic;
// phase-2-xdp-service-map Slice 05 (US-05; S-2.2-20). The
// `ReverseNatLockstep` invariant pins the lockstep contract between
// `SimDataplane.services` and `SimDataplane.reverse_nat`: every
// forward-path service backend has a matching `REVERSE_NAT` entry
// pointing back to the original VIP, written/removed under one
// mutex acquisition. Mirrors the production `EbpfDataplane`'s
// `REVERSE_NAT_MAP` lockstep contract.
pub mod reverse_nat_lockstep;
// unconnected-udp-sendmsg4 Slice 02 (US-02; J-PLAT-004 / K3). GH #200,
// ADR-0053 rev 2026-06-05. The `ReplySourceRewriteLockstep` invariant
// pins the cgroup unconnected-UDP reply-path source identity in the
// `SimDataplane`: after `register_local_backend`, the reply source for
// the backend identity is the VIP (`reply_source_for(...) == Some(vip)`),
// written in lockstep with the forward `local_backend` entry under one
// mutex acquisition (DDD-5d). The structural defense BELOW Tier-3 (no
// Tier-2 backstop for `cgroup_sock_addr`). A forward-only mutation turns
// it RED. Sibling to the Tier-3 round-trip at
// `crates/overdrive-dataplane/tests/integration/unconnected_udp_roundtrip.rs`.
pub mod reply_source_rewrite_lockstep;
// phase-2-xdp-service-map Slice 06 (US-06; S-2.2-22 sibling). The
// `SanityChecksFireBeforeServiceMap` invariant pins the kernel-side
// contract that every sanity-rule-violating packet produces
// `XDP_DROP` (slot `MalformedHeader` increments) AND short-circuits
// before SERVICE_MAP lookup. Sibling to the Tier 3 mixed-batch test
// at `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`.
pub mod sanity_checks_fire;
// phase-2-xdp-service-map DISTILL — RED scaffolds per
// `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
// DWD-4. Hosts `assert_hydrator_eventually_converges` +
// `assert_hydrator_idempotent_steady_state` (both panic until DELIVER
// fills them per Slice 08).
pub mod service_map_hydrator;
// fix-exit-observer-running-gate step 01-05 (Solution 4). The
// `ExitEventObservableOutcome` invariant pins the post-condition
// that every `ExitEvent` consumed by the worker `exit_observer`
// produces at least one of (a) an obs row write with state ∈
// {Failed, Terminated}, (b) a degraded `LifecycleEvent` carrying
// `TransitionReason::DriverInternalError`, or (c) a structured
// `tracing::error!` naming the alloc. Closes the gap predecessor
// RCA `fix-exit-observer-write-retry/deliver/rca.md:107-109`
// named and `docs/evolution/2026-05-02-fix-exit-observer-write-
// retry.md:64` left open.
pub mod exit_event_observable_outcome;
// workload-gc-absent-stale-allocs step 01-03. Two DST scenarios
// pinning the GC reconciler arm convergence + resubmit-after-GC
// race: (1) `WorkloadGcOrphanConverges` — Submit Job(X), drain to
// Running, IntentStore::delete("jobs/X"), drive ≤3 ticks, assert
// every alloc reaches a terminal state with `terminal == Some(
// Stopped { by: SystemGc })` AND no fresh alloc placed; (2)
// `WorkloadGcResubmitCreatesFresh` — continues from quiescent
// orphan state, resubmits Job(X), drives ≤5 ticks, asserts ≥1
// fresh alloc reaches Running with `alloc_id != original` AND
// the original alloc's `SystemGc` terminal stamp is durable.
// Closes #148 AC §1.3.
pub mod workload_gc_absent_intent;

// `backend-discovery-bridge-service-reachability` (joint #174 + #175)
// Slice 1 (closes #174) — three DST evaluators per
// `docs/feature/backend-discovery-bridge-service-reachability/
// distill/test-scenarios.md` S-BDB-02..S-BDB-10 (DST invariants) +
// S-BDB-06 (Atlas Q2 crash-recovery). The free-function evaluators
// live in `backend_discovery_bridge::evaluate_bridge_*` and the
// harness dispatches to them from the `Invariant::Bridge*` arms.
pub mod backend_discovery_bridge;

/// Catalogue of invariants the DST harness evaluates.
///
/// Each variant name IS the canonical name printed in both green
/// progress lines and red failure output. `--only <NAME>` resolves
/// against this enum via [`FromStr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invariant {
    /// At most one leader across the Raft cluster at any simulated tick.
    SingleLeader,
    /// No row in any `ObservationStore` carries intent-class data, and
    /// no key in any `IntentStore` carries observation-class data.
    IntentNeverCrossesIntoObservation,
    /// `IntentStore::export_snapshot` → `bootstrap_from` →
    /// `export_snapshot` is byte-identical.
    SnapshotRoundtripBitIdentical,
    /// LWW convergence across a `SimObservationStore` cluster under
    /// arbitrary seeded delivery orders reaches the same row set on
    /// every peer.
    SimObservationLwwConverges,
    /// workflow-primitive step 01-07 (graduates the slice-1
    /// `ReplayEquivalentEmptyWorkflow` placeholder; ADR-0064 §3/§6, K4 —
    /// the load-bearing KPI on the `cargo dst` critical path). Drives the
    /// real `WorkflowEngine` + `SimJournalStore` through three runs of the
    /// `ProvisionRecord` reference workflow: (1) an uninterrupted run
    /// capturing the terminal trajectory; (2) a crash-injected run (kill
    /// after step-0 records, before terminal); (3) a resumed run from the
    /// persisted journal. Asserts the resumed trajectory is byte-identical
    /// to the uninterrupted one (replay-equivalence) AND the resumed run
    /// reaches a terminal `WorkflowStatus` within the step budget
    /// (`assert_eventually!(is_terminal)`, bounded progress). The evaluator
    /// body lives in `crate::invariants::evaluators`.
    ReplayEquivalenceProvisionRecord,
    /// `SimEntropy` seeded with the same `u64` twice produces the same
    /// draw sequence — the twin-run identity property.
    EntropyDeterminismUnderReseed,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// At least one reconciler is registered with the runtime after
    /// boot; the registry is never empty. The evaluator body panics
    /// until DELIVER wires the control-plane runtime into the harness.
    AtLeastOneReconcilerRegistered,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// N (≥3) concurrent evaluations at the same `(ReconcilerName,
    /// TargetResource)` key collapse to exactly one dispatched
    /// invocation and `N - 1` cancellations. The evaluator body panics
    /// until DELIVER ships the broker.
    DuplicateEvaluationsCollapse,
    /// Two drain passes against identical submit sequences produce
    /// element-equal `dispatched_order` vecs at every position. Closes
    /// `docs/feature/fix-eval-broker-drain-determinism` RCA — the
    /// broker's drain order MUST be deterministic, not dependent on
    /// `HashSet` iteration order or other implicit state. Sibling to
    /// `DuplicateEvaluationsCollapse`: that invariant pins counters,
    /// this one pins ordering.
    BrokerDrainOrderIsDeterministic,
    /// SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
    /// Twin invocation of a reconciler's `reconcile` with identical
    /// inputs produces bit-identical `Vec<Action>` outputs. The
    /// evaluator body panics until DELIVER wires the noop-heartbeat
    /// reconciler into the harness.
    ReconcilerIsPure,
    /// phase-1-control-plane-core / fix-eval-reconciler-discarded follow-up.
    /// For any drained `Evaluation { reconciler: R, target: T }`, exactly
    /// one reconciler — R — runs through the dispatch path against T per
    /// tick. The DST-tier peer of the unit/acceptance pin at
    /// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`
    /// (commit `e6f5e5e`). Closes the §8 storm-proofing dispatch-routing
    /// contract end-to-end. Sibling to `DuplicateEvaluationsCollapse`:
    /// that invariant pins broker-side entry collapse, this one pins
    /// dispatcher-side routing.
    DispatchRoutingIsNameRestricted,
    /// `IntentStore::put(k, v)` followed by `IntentStore::get(k)`
    /// returns `Some(v)` byte-for-byte — no framing, no prefix, no
    /// transformation. Closes ADR-0020 §Enforcement: the structural-
    /// regression guard against re-introducing inline row encoding
    /// in `LocalIntentStore`.
    IntentStoreReturnsCallerBytes,
    /// phase-1-first-workload (slice 3, US-03) — eventually invariant.
    /// For every submitted Job, an `AllocStatusRow{state: Running}`
    /// exists within budget N ticks. The harness drives the
    /// convergence loop forward N ticks and inspects the
    /// `ObservationStore` for at least one `Running` row per
    /// submitted job. Lives in
    /// `crates/overdrive-sim/src/invariants/evaluators.rs` per the
    /// existing single-file evaluator pattern.
    JobScheduledAfterSubmission,
    /// phase-1-first-workload (slice 3, US-03) — eventually invariant.
    /// `count(state == Running) == job.replicas` per submitted job.
    /// Vacuous-pass at N=1 (a 1-replica job has at most one Running
    /// row), but the evaluator still has to walk the rows and tally
    /// per job to catch the failure mode where a Running row leaks
    /// across jobs.
    DesiredReplicaCountConverges,
    /// phase-1-first-workload (slice 3, US-03) — always invariant.
    /// Each `alloc_id` agrees on a single `node_id` across the
    /// `alloc_status` snapshot. Two rows for the same `alloc_id`
    /// pinned to different nodes is a double-scheduling violation.
    NoDoubleScheduling,
    /// reconciler-memory-redb step 01-07 — always invariant.
    /// For arbitrary `View` values, `ViewStore::write_through` followed
    /// by `ViewStore::bulk_load` returns byte-equal values. proptest-
    /// backed; covers `WorkloadLifecycleView` (the only meaningful production
    /// View today) and `()` (the unit-View case used by `NoopHeartbeat`).
    /// Catches CBOR encode/decode regressions, ciborium-version skew,
    /// and serde-derive oversights per ADR-0035 §6.
    ViewStoreRoundtripIsLossless,
    /// reconciler-memory-redb step 01-07 — always invariant.
    /// Two `bulk_load` calls against the same backing store produce
    /// `PartialEq`-equal `BTreeMap` results. Catches iteration-order
    /// regressions in the `BTreeMap`-backed `SimViewStore` storage —
    /// any future mutation that swaps `BTreeMap` for `HashMap` or
    /// otherwise destabilises iteration order would surface here.
    BulkLoadIsDeterministic,
    /// reconciler-memory-redb step 01-07 — always invariant.
    /// Under `SimViewStore::inject_fsync_failure`, the runtime's
    /// in-memory `BTreeMap` visible through
    /// `ReconcilerRuntime::loaded_workload_lifecycle_views_for_test` MUST
    /// NOT be updated for the target whose `write_through` failed. The
    /// load-bearing crash-durability invariant from ADR-0035 §5: the
    /// fsync-then-memory ordering rule. A reconciler runtime that
    /// updated the in-memory map before the fsync would surface stale
    /// state to readers across crashes; this invariant catches the
    /// inverse ordering at PR time.
    WriteThroughOrdering,

    /// phase-2-xdp-service-map Slice 03 (US-03; S-2.2-09) — always
    /// invariant. Every observation of
    /// `SimDataplane.services[service]` made concurrent with an
    /// `update_service` call sees either the pre-swap backend set or
    /// the post-swap backend set — never a torn / mixed state. DST
    /// mirror of the production `EbpfDataplane`'s atomic outer-map
    /// swap (`HASH_OF_MAPS`). The evaluator body lives in
    /// `crate::invariants::backend_set_swap_atomic`.
    BackendSetSwapAtomic,

    /// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-13 sibling) —
    /// always invariant. Under equal weights, the Maglev permutation
    /// distributes slots within ±5 % of the per-backend expectation
    /// (`M / N`). Sibling to the `single_backend_removal_shifts_at_
    /// most_two_percent_of_flows` proptest in
    /// `crates/overdrive-sim/tests/integration/maglev_churn.rs`: the
    /// proptest pins the churn property, this invariant pins the
    /// steady-state distribution property. Both ride on the same
    /// `maglev::generate` pure function. The evaluator body lives in
    /// `crate::invariants::maglev_distribution`.
    MaglevDistributionEven,

    /// phase-2-xdp-service-map Slice 04 (US-04; S-2.2-14 sibling) —
    /// always invariant. Two successive `maglev::generate` calls with
    /// identical inputs return bit-identical `Vec<BackendId>` outputs.
    /// The K3 reproducibility property (whitepaper §21) projected onto
    /// the Maglev permutation: any seeded fixture's BPF inner-map
    /// contents must be byte-equal across twin runs. Sibling to
    /// `MaglevDistributionEven`: that invariant pins the steady-state
    /// distribution property, this one pins the determinism property.
    /// The evaluator body lives in
    /// `crate::invariants::maglev_deterministic`.
    MaglevDeterministic,

    /// phase-2-xdp-service-map Slice 05 (US-05; S-2.2-20) — always
    /// invariant. Every forward-path `SimDataplane.services[vip]`
    /// entry has a matching `reverse_nat[BackendKey::from(backend)]`
    /// entry mapping back to the original VIP; removing a backend
    /// purges both in lockstep. DST mirror of the production
    /// `EbpfDataplane`'s `REVERSE_NAT_MAP` lockstep contract — one
    /// mutex acquisition guards both maps. The evaluator body lives
    /// in `crate::invariants::reverse_nat_lockstep`.
    ReverseNatLockstep,

    /// unconnected-udp-sendmsg4 Slice 02 (US-02; J-PLAT-004 / K3) —
    /// always invariant. GH #200, ADR-0053 rev 2026-06-05. After
    /// `register_local_backend(vip, vip_port, backend, proto)`, the
    /// `SimDataplane` reply mirror carries
    /// `BackendKey(backend_ip, backend_port, proto) → vip`: the
    /// unconnected-UDP recvmsg4 reply source the app would read is the
    /// VIP, never the backend, written in lockstep with the forward
    /// `local_backend` entry under one mutex acquisition (DDD-5d). The
    /// structural defense BELOW Tier-3 — there is NO Tier-2
    /// `BPF_PROG_TEST_RUN` backstop for `cgroup_sock_addr` (ENOTSUPP
    /// ≤ 6.8); the kernel recvmsg4 rewrite is a Tier-3-only gate and this
    /// invariant pins the same observable contract on the Sim adapter,
    /// meeting Tier-3 at the shared backend identity. A forward-only /
    /// asymmetric regression (forward entry written, reply mirror not)
    /// turns it RED — the #163-class mutation this slice kills. Mirror of
    /// `ReverseNatLockstep` retargeted to the cgroup same-host reply
    /// path. The evaluator body lives in
    /// `crate::invariants::reply_source_rewrite_lockstep`. RED until the
    /// Sim reply-mirror write lands (Slice 01/02 GREEN).
    ReplySourceRewriteLockstep,

    /// phase-2-xdp-service-map Slice 06 (US-06; S-2.2-22 sibling) —
    /// always invariant. Every packet whose classification violates
    /// a sanity-prologue rule (truncated IPv4 header, pathological
    /// TCP flags, IPv6 `EtherType` when the LB is IPv4-only, ...)
    /// MUST cause the kernel-side dataplane to drop the frame and
    /// MUST short-circuit BEFORE `SERVICE_MAP` lookup. Mirror of the
    /// production XDP / TC sanity prologue: `MalformedHeader` slot
    /// increments and the program returns `XDP_DROP` / `TC_ACT_SHOT`
    /// before any `HoM` lookup. The evaluator body lives in
    /// `crate::invariants::sanity_checks_fire`. Sibling to the Tier 3
    /// mixed-batch test at
    /// `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`.
    SanityChecksFireBeforeServiceMap,

    /// SCAFFOLD: true — phase-2-xdp-service-map DISTILL per ADR-0042
    /// + architecture.md § 8 *ESR pair*. Eventual: from any
    /// combination of `service_backends` rows + starting BPF map
    /// state, repeated reconcile ticks drive
    /// `actual.fingerprint == desired.fingerprint` for every service.
    /// The evaluator body panics with a `RED scaffold` message until
    /// DELIVER ships the body per Slice 08 / S-2.2-26.
    HydratorEventuallyConverges,

    /// SCAFFOLD: true — phase-2-xdp-service-map DISTILL per ADR-0042
    /// + architecture.md § 8 *ESR pair*. Always: once
    /// `actual.fingerprint == desired.fingerprint` for all services,
    /// the hydrator emits zero `Action::DataplaneUpdateService`
    /// actions per tick. The evaluator body panics with a
    /// `RED scaffold` message until DELIVER ships the body per
    /// Slice 08 / S-2.2-27.
    HydratorIdempotentSteadyState,

    /// fix-exit-observer-running-gate step 01-05 (Solution 4) —
    /// eventually invariant. For every `ExitEvent` consumed by the
    /// worker `exit_observer::run_with_retry → handle_exit_event`,
    /// at least one of:
    ///   (a) an obs row write of `AllocStatusRow{state ∈ {Failed,
    ///       Terminated}}` for the same `alloc_id`,
    ///   (b) a degraded `LifecycleEvent` broadcast carrying
    ///       `TransitionReason::DriverInternalError` (May-2
    ///       escalation path),
    ///   (c) a structured `tracing::error!` log naming the
    ///       `alloc_id` and the underlying error
    /// is produced. Closes the gap predecessor RCA
    /// `fix-exit-observer-write-retry/deliver/rca.md:107-109`
    /// named and `docs/evolution/2026-05-02-fix-exit-observer-
    /// write-retry.md:64` left open. With Solution 1' landed in
    /// steps 01-02 / 01-03, the invariant does NOT fire under the
    /// canonical flow — its load-bearing role is preventing
    /// future regressions through any emission path that bypasses
    /// the gate. The evaluator body lives in
    /// `crate::invariants::exit_event_observable_outcome`.
    ExitEventObservableOutcome,

    /// workload-gc-absent-stale-allocs step 01-03 — eventually
    /// invariant. After `IntentStore::delete("jobs/X")` removes
    /// the desired Job, every non-terminal `AllocStatusRow` for
    /// `workload_id == X` reaches a terminal state within 3 ticks
    /// AND carries `terminal == Some(TerminalCondition::Stopped {
    /// by: StoppedBy::SystemGc })` AND no fresh allocation is
    /// placed for X while intent stays absent. Drives end-to-end
    /// through `SimIntentStore` + `SimObservationStore` +
    /// `WorkloadLifecycle` runtime stack — entry through the
    /// `submit` / `tick` harness driving ports, assertions on
    /// `ObservationStore::alloc_status_rows()` (driven port
    /// boundary). The evaluator body lives in
    /// `crate::invariants::workload_gc_absent_intent`. Closes
    /// #148 AC §1.3.
    WorkloadGcOrphanConverges,

    /// workload-gc-absent-stale-allocs step 01-03 — eventually +
    /// always invariant. Continues from
    /// `WorkloadGcOrphanConverges`'s quiescent state: resubmits
    /// `Job(id=X)` to intent, drives ≤5 ticks, asserts (a) ≥1
    /// alloc reaches Running with a fresh `alloc_id` distinct
    /// from the original GC'd row's `alloc_id` (durable
    /// distinctness — the GC'd row is not resurrected) AND (b)
    /// the original alloc's `terminal` field stays
    /// `Some(Stopped { by: SystemGc })` for every tick after
    /// resubmit (the `SystemGc` stamp is durable through the
    /// resubmit cycle). The evaluator body lives in
    /// `crate::invariants::workload_gc_absent_intent`.
    /// Promoted into `ALL` by step 01-04 once the
    /// resurrection-protection helper (`is_intentionally_stopped`)
    /// was generalized to cover both `Operator` and `SystemGc`
    /// stops, the Run-branch's `active_allocs_vec` filter excluded
    /// SystemGc-stopped rows from placement-candidacy, and
    /// `mint_alloc_id` was extended to accept an `attempt` index
    /// so a resubmit mints a distinct `alloc_id` rather than
    /// reusing the GC'd row's id. Closes #148 AC §1.3.
    WorkloadGcResubmitCreatesFresh,
    /// `backend-discovery-bridge-service-reachability` (#174) Slice 1
    /// — eventually invariant. For every Service workload with `>= 1`
    /// listener AND an allocator-issued VIP for its `spec_digest` AND
    /// `>= 1` Running alloc, a `ServiceBackendRow` is eventually written
    /// whose `backends` field contains exactly the Running allocs'
    /// endpoints. The evaluator body lives in
    /// `crate::invariants::backend_discovery_bridge`. Closes S-BDB-02
    /// / S-BDB-03 / S-BDB-04 / S-BDB-10 per
    /// `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`.
    BridgeEventuallyWritesBackendRow,
    /// `backend-discovery-bridge-service-reachability` (#174) Slice 1
    /// — always invariant. Once `obs.service_backends_rows(...).backends
    /// == expected` for every Service workload, the bridge emits zero
    /// `Action::WriteServiceBackendRow` actions on subsequent ticks
    /// given unchanged inputs. Also exercises the View `retain` GC
    /// clause (S-BDB-07). The evaluator body lives in
    /// `crate::invariants::backend_discovery_bridge`.
    BridgeIdempotentSteadyState,
    /// `backend-discovery-bridge-service-reachability` (Atlas Q2)
    /// Slice 1 — always invariant under the crash-recovery scenario
    /// family. Models a crash between `SimViewStore::write_through`
    /// fsync and the runtime's in-memory `BTreeMap::insert`; after
    /// the restart-equivalent `bulk_load`, asserts the bridge's
    /// first post-restart tick re-projects from fresh inputs and
    /// either emits zero actions (idempotent) or emits
    /// `Action::WriteServiceBackendRow` with the new fingerprint (no
    /// silent skip on cached stale state). Proves the
    /// fsync-then-memory ordering rule in
    /// `.claude/rules/development.md` § "Reconciler I/O" is honored
    /// by the bridge's reconcile body. The evaluator body lives in
    /// `crate::invariants::backend_discovery_bridge`. Closes S-BDB-06.
    BridgeRecomputesFingerprintOnReplay,
    /// `backend-discovery-bridge-service-reachability` step 02-04 —
    /// always invariant. Drives the in-process bridge → hydrator
    /// handoff at Tier 1: ticks `BackendDiscoveryBridge::reconcile`
    /// against a Running alloc + projected listener, applies the
    /// emitted `Action::WriteServiceBackendRow` to a
    /// `SimObservationStore`, reads `service_backends_rows` back into
    /// a `ServiceMapHydratorState.desired` projection (mirrors the
    /// runtime `hydrate_desired` arm), then ticks
    /// `ServiceMapHydrator::reconcile` against that state and asserts
    /// exactly one `Action::DataplaneUpdateService` is emitted
    /// carrying the bridge-written row's `vip` + `backends`. Pins
    /// the cross-reconciler fingerprint-identity contract — drift in
    /// either reconciler's encoding fails the invariant. The Tier 3
    /// walking-skeleton (`crates/overdrive-control-plane/tests/integration/
    /// backend_discovery_bridge/walking_skeleton.rs`) exercises the
    /// same property against the real kernel adapter. The evaluator
    /// body lives in
    /// `crate::invariants::service_map_hydrator::evaluate_bridge_to_hydrator_handoff`.
    /// Closes S-BDB-19.
    BridgeToHydratorHandoff,

    /// workflow-primitive step 01-07 (ADR-0064 §6, mirroring ADR-0035's
    /// `WriteThroughOrdering`) — always invariant. Under a
    /// `SimJournalStore` with an injected fsync-failure on the next
    /// `append`, the engine's live-path `ctx.call` record FAILS and the
    /// journal cursor does NOT advance: a subsequent retry is still a LIVE
    /// call (not a replay), the journal carries no phantom half-written
    /// entry, and the engine does not suspend acknowledging an unrecorded
    /// step. fsync-then-suspend is load-bearing (ADR-0063 §4). The
    /// evaluator body lives in `crate::invariants::evaluators`.
    WorkflowJournalWriteOrdering,

    /// workflow-primitive step 01-07 (ADR-0064 §6; US-WP-3 AC1 / K1) —
    /// always invariant. Crash after a `ctx.call` records but before
    /// terminal → resume from the persisted `SimJournalStore` journal →
    /// the recorded effect is replayed WITHOUT re-firing the transport
    /// (the resumed boot's bound `SimInbox` receives zero datagrams) and
    /// the run reaches the same terminal `WorkflowStatus`. The
    /// exactly-once-on-resume guarantee. The evaluator body lives in
    /// `crate::invariants::evaluators`.
    WorkflowExactlyOnceEffectOnResume,

    /// workflow-result-error-model step 02-01 (ADR-0065 §3) — RED scaffold.
    /// Always invariant: under the migrated terminal model, an
    /// uninterrupted `ProvisionRecord` run writes a
    /// `WorkflowStatus::Completed { output }` terminal whose erased CBOR
    /// `output` round-trips back to the workflow's typed `Output` (`()` for
    /// the reference fixtures), and an authored-failure / panic run writes
    /// `WorkflowStatus::Failed { terminal }` carrying the structured
    /// `TerminalError`. Pins the engine's body-`Result` → `WorkflowStatus`
    /// projection at the DST tier. NOT wired into the default catalogue
    /// (`ALL`) yet — the evaluator body is a `todo!` RED scaffold that
    /// lands GREEN in step 02-01, so `cargo dst` (which iterates `ALL`)
    /// stays green until then.
    WorkflowTerminalStatusProjection,
}

impl Invariant {
    /// Every variant in the catalogue, in the order the harness runs
    /// them by default. Keep this list synchronised with the enum —
    /// `ALL` is the default catalogue the harness iterates when
    /// `--only <NAME>` is absent.
    pub const ALL: &'static [Self] = &[
        Self::SingleLeader,
        Self::IntentNeverCrossesIntoObservation,
        Self::SnapshotRoundtripBitIdentical,
        Self::SimObservationLwwConverges,
        // workflow-primitive step 01-07 — graduates the slice-1
        // `ReplayEquivalentEmptyWorkflow` placeholder into a real journal
        // replay against the `WorkflowEngine` + `SimJournalStore`. K4 on
        // the `cargo dst` critical path (ADR-0064 §3/§6).
        Self::ReplayEquivalenceProvisionRecord,
        Self::EntropyDeterminismUnderReseed,
        // SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
        Self::AtLeastOneReconcilerRegistered,
        Self::DuplicateEvaluationsCollapse,
        Self::BrokerDrainOrderIsDeterministic,
        Self::ReconcilerIsPure,
        Self::DispatchRoutingIsNameRestricted,
        Self::IntentStoreReturnsCallerBytes,
        // SCAFFOLD: false — phase-1-first-workload slice 3 (US-03).
        Self::JobScheduledAfterSubmission,
        Self::DesiredReplicaCountConverges,
        Self::NoDoubleScheduling,
        // reconciler-memory-redb step 01-07 — ViewStore DST invariants
        // per ADR-0035 §6.
        Self::ViewStoreRoundtripIsLossless,
        Self::BulkLoadIsDeterministic,
        Self::WriteThroughOrdering,
        // phase-2-xdp-service-map Slice 03 (US-03; S-2.2-09). The
        // `BackendSetSwapAtomic` invariant body lands in GREEN of
        // step 03-01; the variant is registered up front so the
        // canonical name is stable.
        Self::BackendSetSwapAtomic,
        // phase-2-xdp-service-map Slice 04 (US-04; S-2.2-13 sibling).
        // The `MaglevDistributionEven` invariant body lives in
        // `crate::invariants::maglev_distribution`. Sibling to the
        // disruption-bound proptest at
        // `tests/integration/maglev_churn.rs`.
        Self::MaglevDistributionEven,
        // phase-2-xdp-service-map Slice 04 (US-04; S-2.2-14 sibling).
        // The `MaglevDeterministic` invariant body lives in
        // `crate::invariants::maglev_deterministic`. Sibling to
        // `MaglevDistributionEven` — both ride on the same pure
        // `maglev::generate` function.
        Self::MaglevDeterministic,
        // phase-2-xdp-service-map Slice 05 (US-05; S-2.2-20). The
        // `ReverseNatLockstep` invariant body lives in
        // `crate::invariants::reverse_nat_lockstep`.
        Self::ReverseNatLockstep,
        // unconnected-udp-sendmsg4 Slice 02 (US-02; J-PLAT-004 / K3). GH
        // #200. The `ReplySourceRewriteLockstep` invariant body lives in
        // `crate::invariants::reply_source_rewrite_lockstep`. RED until
        // the `SimDataplane::register_local_backend` reply-mirror write
        // lands (Slice 01/02 GREEN) — the outer-loop RED signal for the
        // unconnected-UDP reply-path identity (no Tier-2 backstop).
        Self::ReplySourceRewriteLockstep,
        // phase-2-xdp-service-map Slice 06 (US-06; S-2.2-22 sibling).
        // The `SanityChecksFireBeforeServiceMap` invariant body lives
        // in `crate::invariants::sanity_checks_fire`. Sibling to the
        // Tier 3 mixed-batch test at
        // `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`.
        Self::SanityChecksFireBeforeServiceMap,
        // phase-2-xdp-service-map DISTILL — RED scaffolds per
        // `docs/feature/phase-2-xdp-service-map/distill/wave-decisions.md`
        // DWD-4. Evaluator bodies panic until DELIVER fills them.
        Self::HydratorEventuallyConverges,
        Self::HydratorIdempotentSteadyState,
        // fix-exit-observer-running-gate step 01-05 (Solution 4).
        // The evaluator body lives in
        // `crate::invariants::exit_event_observable_outcome`.
        Self::ExitEventObservableOutcome,
        // workload-gc-absent-stale-allocs steps 01-03 + 01-04.
        // Evaluator bodies live in
        // `crate::invariants::workload_gc_absent_intent`. Both
        // variants are in the default catalogue: step 01-04 closed
        // the resurrection-protection gap (the
        // `is_intentionally_stopped` helper, the
        // `active_allocs_vec` Run-branch filter, and the
        // `mint_alloc_id(workload_id, attempt)` extension) so
        // `WorkloadGcResubmitCreatesFresh` now passes against the
        // production reconciler.
        Self::WorkloadGcOrphanConverges,
        Self::WorkloadGcResubmitCreatesFresh,
        // backend-discovery-bridge-service-reachability (#174 + Atlas Q2)
        // Slice 1 — three evaluators land in
        // `crate::invariants::backend_discovery_bridge::
        // evaluate_bridge_{eventually_writes_backend_row,
        // idempotent_steady_state, recomputes_fingerprint_on_replay}`.
        Self::BridgeEventuallyWritesBackendRow,
        Self::BridgeIdempotentSteadyState,
        Self::BridgeRecomputesFingerprintOnReplay,
        // backend-discovery-bridge-service-reachability step 02-04 —
        // bridge → hydrator handoff (S-BDB-19). The evaluator body
        // lives in
        // `crate::invariants::service_map_hydrator::evaluate_bridge_to_hydrator_handoff`.
        Self::BridgeToHydratorHandoff,
        // workflow-primitive step 01-07 — the two sibling workflow
        // durability invariants (ADR-0064 §6). Evaluator bodies live in
        // `crate::invariants::evaluators`.
        Self::WorkflowJournalWriteOrdering,
        Self::WorkflowExactlyOnceEffectOnResume,
        // workflow-result-error-model step 02-01 (ADR-0065 §3, D3) — the
        // body-`Result` → `WorkflowStatus` projection invariant. Wired into
        // the default catalogue in GREEN of step 02-01 once the evaluator
        // body landed in `crate::invariants::evaluators`. Pins the engine's
        // `Err(TerminalError)` → `WorkflowStatus::Failed { terminal }`
        // projection AND the lossless terminal round-trip through both the
        // journal `Terminal` command and the `WorkflowTerminal` obs row.
        Self::WorkflowTerminalStatusProjection,
    ];

    /// The canonical kebab-case spelling of this invariant, as a static
    /// string. `Display` renders the same text; having a `&'static str`
    /// view lets callers embed the name in logs without allocating.
    #[must_use]
    pub const fn as_canonical(self) -> &'static str {
        match self {
            Self::SingleLeader => "single-leader",
            Self::IntentNeverCrossesIntoObservation => "intent-never-crosses-into-observation",
            Self::SnapshotRoundtripBitIdentical => "snapshot-roundtrip-bit-identical",
            Self::SimObservationLwwConverges => "sim-observation-lww-converges",
            Self::ReplayEquivalenceProvisionRecord => "replay-equivalence-provision-record",
            Self::EntropyDeterminismUnderReseed => "entropy-determinism-under-reseed",
            // SCAFFOLD: true — phase-1-control-plane-core DISTILL per ADR-0013.
            Self::AtLeastOneReconcilerRegistered => "at-least-one-reconciler-registered",
            Self::DuplicateEvaluationsCollapse => "duplicate-evaluations-collapse",
            Self::BrokerDrainOrderIsDeterministic => "broker-drain-order-is-deterministic",
            Self::ReconcilerIsPure => "reconciler-is-pure",
            Self::DispatchRoutingIsNameRestricted => "dispatch-routing-is-name-restricted",
            Self::IntentStoreReturnsCallerBytes => "intent-store-returns-caller-bytes",
            // phase-1-first-workload slice 3 (US-03).
            Self::JobScheduledAfterSubmission => "job-scheduled-after-submission",
            Self::DesiredReplicaCountConverges => "desired-replica-count-converges",
            Self::NoDoubleScheduling => "no-double-scheduling",
            // reconciler-memory-redb step 01-07.
            Self::ViewStoreRoundtripIsLossless => "view-store-roundtrip-is-lossless",
            Self::BulkLoadIsDeterministic => "bulk-load-is-deterministic",
            Self::WriteThroughOrdering => "write-through-ordering",
            Self::BackendSetSwapAtomic => "backend-set-swap-atomic",
            Self::MaglevDistributionEven => "maglev-distribution-even",
            Self::MaglevDeterministic => "maglev-deterministic",
            Self::ReverseNatLockstep => "reverse-nat-lockstep",
            // unconnected-udp-sendmsg4 Slice 02 (US-02 / K3).
            Self::ReplySourceRewriteLockstep => "reply-source-rewrite-lockstep",
            Self::SanityChecksFireBeforeServiceMap => "sanity-checks-fire-before-service-map",
            Self::HydratorEventuallyConverges => "hydrator-eventually-converges",
            Self::HydratorIdempotentSteadyState => "hydrator-idempotent-steady-state",
            Self::ExitEventObservableOutcome => "exit-event-observable-outcome",
            // workload-gc-absent-stale-allocs step 01-03.
            Self::WorkloadGcOrphanConverges => "workload-gc-orphan-converges",
            Self::WorkloadGcResubmitCreatesFresh => "workload-gc-resubmit-creates-fresh",
            // backend-discovery-bridge-service-reachability (#174 + Atlas Q2)
            // DISTILL — RED scaffolds.
            Self::BridgeEventuallyWritesBackendRow => "bridge-eventually-writes-backend-row",
            Self::BridgeIdempotentSteadyState => "bridge-idempotent-steady-state",
            Self::BridgeRecomputesFingerprintOnReplay => "bridge-recomputes-fingerprint-on-replay",
            // backend-discovery-bridge-service-reachability step 02-04 (S-BDB-19).
            Self::BridgeToHydratorHandoff => "bridge-to-hydrator-handoff",
            // workflow-primitive step 01-07.
            Self::WorkflowJournalWriteOrdering => "workflow-journal-write-ordering",
            Self::WorkflowExactlyOnceEffectOnResume => "workflow-exactly-once-effect-on-resume",
            // workflow-result-error-model step 02-01 (RED scaffold; not in ALL).
            Self::WorkflowTerminalStatusProjection => "workflow-terminal-status-projection",
        }
    }
}

impl Display for Invariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_canonical())
    }
}

impl FromStr for Invariant {
    type Err = InvariantParseError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        // Case-insensitive match against the canonical forms. Hyphens
        // are preserved, only alphabetic characters are folded.
        let lowered = raw.to_ascii_lowercase();
        for candidate in Self::ALL {
            if candidate.as_canonical() == lowered {
                return Ok(*candidate);
            }
        }
        Err(InvariantParseError { raw: raw.to_owned() })
    }
}

/// Error returned when `--only <NAME>` cannot be resolved to an
/// [`Invariant`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown invariant name: {raw:?}")]
pub struct InvariantParseError {
    /// The caller-provided string that did not match any variant.
    pub raw: String,
}
