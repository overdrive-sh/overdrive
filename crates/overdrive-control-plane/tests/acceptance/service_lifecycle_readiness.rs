//! Tier 1 acceptance — readiness probe → `Backend.healthy` flip.
//!
//! Slice 04 (US-04). Step 03-01 GREEN landing.
//!
//! KPI K2: readiness probe Pass → Fail flips `Backend.healthy =
//! false` within 1 reconciler tick. The dataplane fingerprint
//! changes as a consequence (asserted via the
//! `readiness_health_flip_changes_fingerprint` proptest at
//! `crates/overdrive-core/src/dataplane/fingerprint.rs`).
//!
//! PBT paradigm per the standing mandate: the RECON-07 / 08b / 08c
//! / no-restart scenarios are property-based with declared universes;
//! RECON-08 (recovery) is a single representative transition (the
//! property is "Fail→Pass restores within one tick" — a directional
//! transition, expressed once).
//!
//! Per `.claude/rules/development.md` § "Reconciler I/O": `reconcile`
//! is pure sync `(desired, actual, view, tick) → (Vec<Action>,
//! View)`. No `.await` in test bodies. Universe = observable Action
//! emissions (the `WriteServiceBackendRow` row's `backends[*].healthy`
//! flags) — never internal View fields.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names
)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRow, ProbeRole, ProbeRunSeq, ProbeStatus, SameStatusRuns,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use proptest::prelude::*;

fn tick_at(now_unix_ms: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_millis(now_unix_ms)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid alloc id")
}

fn spiffe(i: usize) -> SpiffeId {
    SpiffeId::new(&format!("spiffe://overdrive.local/workload/svc/alloc/a{i}"))
        .expect("valid spiffe")
}

fn dataplane_identity() -> ServiceDataplaneIdentity {
    ServiceDataplaneIdentity {
        port: std::num::NonZeroU16::new(8080).expect("listener port"),
        protocol: overdrive_core::dataplane::backend_key::Proto::Tcp,
        vip: ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, 1))).expect("valid vip"),
        writer: NodeId::new("node-1").expect("valid node id"),
    }
}

/// Build a Service alloc fact carrying a readiness probe with the given
/// latest status. `started_at` is `Some` (the alloc reached Running).
fn fact_with_readiness(
    index: usize,
    latest_readiness: Option<ProbeStatus>,
    success_threshold: u32,
) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: alloc(&format!("svc-{index}")),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        // Startup already passed — these allocs are Stable backends.
        latest_startup_probe: Some(ProbeStatus::Pass),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: latest_readiness,
        has_readiness_probe: true,
        readiness_success_threshold: success_threshold,
        backend_spiffe: spiffe(index),
        backend_ip: Ipv4Addr::new(192, 168, 1, u8::try_from(10 + index).unwrap_or(u8::MAX)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

/// Build a Service alloc fact with NO readiness probe (backward-compat
/// default — `healthy = true` post-Stable).
fn fact_without_readiness(index: usize) -> ServiceAllocFact {
    ServiceAllocFact {
        alloc_id: alloc(&format!("svc-{index}")),
        state: AllocState::Running,
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1))),
        exit_code: None,
        latest_startup_probe: Some(ProbeStatus::Pass),
        latest_startup_probe_observed_at: Some(UnixInstant::from_unix_duration(
            Duration::from_millis(1),
        )),
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: spiffe(index),
        backend_ip: Ipv4Addr::new(192, 168, 1, u8::try_from(10 + index).unwrap_or(u8::MAX)),
        latest_liveness_probe: None,
        has_liveness_probe: false,
        liveness_failure_threshold: 3,
        probe_rows: BTreeMap::new(),
    }
}

fn state_with(facts: Vec<ServiceAllocFact>) -> ServiceLifecycleState {
    let mut state = ServiceLifecycleState {
        service_dataplane: BTreeMap::from([(
            ServiceId::new(42).expect("service id"),
            dataplane_identity(),
        )]),
        ..ServiceLifecycleState::default()
    };
    for fact in facts {
        let alloc_id = fact.alloc_id.clone();
        let startup = fact.latest_startup_probe.clone();
        let readiness = fact.latest_readiness_probe.clone();
        state.allocs.insert(alloc_id.clone(), fact);
        if let Some(status) = startup {
            put_startup_row(&mut state, &alloc_id, status);
        }
        if let Some(status) = readiness {
            put_readiness_row(&mut state, &alloc_id, status, 1, 1);
        }
    }
    state
}

fn put_readiness_row(
    state: &mut ServiceLifecycleState,
    alloc_id: &AllocationId,
    status: ProbeStatus,
    run_seq: u64,
    same_status_runs: u32,
) {
    let probe_idx = ProbeIdx::new(0);
    let fact = state.allocs.get_mut(alloc_id).expect("fixture allocation");
    fact.latest_readiness_probe = Some(status.clone());
    fact.probe_rows.insert(
        (ProbeRole::Readiness, probe_idx),
        ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx,
            role: ProbeRole::Readiness,
            status,
            last_observed_at_unix_ms: run_seq,
            inferred: false,
            run_seq: ProbeRunSeq::new(run_seq).expect("fixture run sequence is non-zero"),
            same_status_runs: SameStatusRuns::new(same_status_runs)
                .expect("fixture streak is non-zero"),
        },
    );
}

fn put_startup_row(
    state: &mut ServiceLifecycleState,
    alloc_id: &AllocationId,
    status: ProbeStatus,
) {
    let probe_idx = ProbeIdx::new(0);
    let fact = state.allocs.get_mut(alloc_id).expect("fixture allocation");
    fact.latest_startup_probe = Some(status.clone());
    fact.latest_startup_probe_observed_at =
        Some(UnixInstant::from_unix_duration(Duration::from_millis(1)));
    fact.probe_rows.insert(
        (ProbeRole::Startup, probe_idx),
        ProbeResultRow {
            alloc_id: alloc_id.clone(),
            probe_idx,
            role: ProbeRole::Startup,
            status,
            last_observed_at_unix_ms: 1,
            inferred: false,
            run_seq: ProbeRunSeq::FIRST,
            same_status_runs: SameStatusRuns::ONE,
        },
    );
}

/// Extract the single `WriteServiceBackendRow` from a reconcile's
/// emitted actions. Panics if absent or duplicated.
fn backend_row_backends(actions: &[Action]) -> Vec<overdrive_core::traits::dataplane::Backend> {
    let rows: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            Action::WriteServiceBackendRow { row, .. } => Some(row.backends.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1, "exactly one WriteServiceBackendRow expected, got {actions:?}");
    rows.into_iter().next().unwrap()
}

fn assert_no_restart(actions: &[Action]) {
    for action in actions {
        assert!(
            !matches!(action, Action::RestartAllocation { .. }),
            "readiness branch must NEVER emit RestartAllocation (that is liveness, step 03-02); got {action:?}"
        );
    }
}

// S-SHCP-RECON-07 (US-04 / K2) — for every (backend count 1..=3) ×
// (per-backend readiness ∈ {Pass, Fail}) × (producer streak
// 0..=10), reconcile emits one WriteServiceBackendRow whose every
// backend carries `healthy = (latest == Pass AND
// same_status_runs >= success_threshold)`.
//
// success_threshold = 1 (default), so a single Pass this tick is
// sufficient. Universe = the emitted row's per-backend `healthy`
// flags. The generated streak exercises the producer-row input path.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_flips_backend_healthy_within_one_tick(
        statuses in prop::collection::vec(any::<bool>(), 1..=3),
        seed_counter in 0u32..=10,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let facts: Vec<ServiceAllocFact> = statuses
            .iter()
            .enumerate()
            .map(|(i, &passes)| {
                let status = if passes {
                    ProbeStatus::Pass
                } else {
                    ProbeStatus::Fail { last_fail_reason: "conn refused".to_string() }
                };
                fact_with_readiness(i, Some(status), 1)
            })
            .collect();
        let mut actual = state_with(facts);
        let streak = seed_counter.saturating_add(1);
        for (i, &passes) in statuses.iter().enumerate() {
            let status = if passes {
                ProbeStatus::Pass
            } else {
                ProbeStatus::Fail { last_fail_reason: "conn refused".to_string() }
            };
            put_readiness_row(
                &mut actual,
                &alloc(&format!("svc-{i}")),
                status,
                u64::from(streak),
                streak,
            );
        }
        let desired = actual.clone();
        let tick = tick_at(2000);

        let (actions, _next_view) = reconciler.reconcile(
            &desired,
            &actual,
            &ServiceLifecycleView::default(),
            &tick,
        );
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);

        prop_assert_eq!(backends.len(), statuses.len(), "one backend per alloc");
        // Backends are emitted in BTreeMap iteration order; facts were
        // keyed `svc-0 .. svc-N` so index aligns with `statuses`.
        for (i, &passes) in statuses.iter().enumerate() {
            // success_threshold = 1; any Pass row's streak is at least one,
            // so Pass ⇒ healthy. Fail ⇒ not healthy.
            prop_assert_eq!(
                backends[i].healthy,
                passes,
                "backend {} healthy must equal (latest_readiness == Pass)",
                i
            );
        }
    }
}

// S-SHCP-RECON-07b (US-04 / ADR-0055 §6 success_threshold gate) — for
// a readiness Pass with `success_threshold > 1`, the backend is healthy
// THIS TICK iff the producer row's streak reaches the threshold:
// `healthy == (same_status_runs >= success_threshold)`.
// Universe = (prior written Pass runs 0..=10) × (success_threshold
// 2..=6). This pins the `>= success_threshold` comparison AND the
// `latest == Pass AND counter >= threshold` conjunction (both operands
// vary independently here, unlike the threshold-1 RECON-07 case where
// Pass alone always satisfies the gate).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_pass_below_threshold_stays_unhealthy(
        seed_counter in 0u32..=10,
        success_threshold in 2u32..=6,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let fact = fact_with_readiness(0, Some(ProbeStatus::Pass), success_threshold);
        let alloc_id = fact.alloc_id.clone();
        let mut actual = state_with(vec![fact]);
        let streak = seed_counter.saturating_add(1);
        put_readiness_row(
            &mut actual,
            &alloc_id,
            ProbeStatus::Pass,
            u64::from(streak),
            streak,
        );
        let desired = actual.clone();
        let tick = tick_at(7000);

        let (actions, _next_view) = reconciler.reconcile(
            &desired,
            &actual,
            &ServiceLifecycleView::default(),
            &tick,
        );
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);
        prop_assert_eq!(backends.len(), 1);
        let expected_healthy = streak >= success_threshold;
        prop_assert_eq!(
            backends[0].healthy,
            expected_healthy,
            "Pass with threshold {}: healthy iff producer streak ({} + 1) >= threshold",
            success_threshold,
            seed_counter
        );
    }
}

/// S-SHCP-RECON-08 (US-04 / K2 recovery) — a backend whose prior
/// readiness was Fail (counter 0) recovers to `healthy = true` on the
/// next-tick Pass within one reconcile (success_threshold = 1).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn readiness_fail_to_pass_restores_backend_healthy_within_one_tick() {
    let reconciler = ServiceLifecycleReconciler::new();
    let fact = fact_with_readiness(0, Some(ProbeStatus::Pass), 1);
    let actual = state_with(vec![fact]);
    let desired = actual.clone();

    // Prior tick observed Fail → counter reset to 0 (absent).
    let view = ServiceLifecycleView::default();
    let tick = tick_at(3000);

    let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
    assert_no_restart(&actions);
    let backends = backend_row_backends(&actions);
    assert_eq!(backends.len(), 1);
    assert!(backends[0].healthy, "Fail→Pass restores healthy within one tick");
}

// S-SHCP-RECON-08b (US-04 — no-readiness default) — for an arbitrary
// Service with 1..=3 allocs and ZERO readiness probes, every backend is
// `healthy = true` post-Stable (backward compatibility). Universe = the
// emitted row's per-backend `healthy` flags.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn service_without_readiness_probes_is_healthy_post_stable(
        backend_count in 1usize..=3,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let facts: Vec<ServiceAllocFact> =
            (0..backend_count).map(fact_without_readiness).collect();
        let actual = state_with(facts);
        let desired = actual.clone();
        let view = ServiceLifecycleView::default();
        let tick = tick_at(4000);

        let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);
        prop_assert_eq!(backends.len(), backend_count);
        for (i, b) in backends.iter().enumerate() {
            prop_assert!(b.healthy, "no-readiness backend {} must be healthy post-Stable", i);
        }
    }
}

// S-SHCP-RECON-08c (US-04 — initial state) — for an arbitrary spawned
// alloc carrying a readiness probe but NO Pass row yet (latest = None),
// `Backend.healthy = false` (avoids the inverse race). Universe = the
// emitted row's per-backend `healthy` flag. Threshold is arbitrary
// 1..=5; with no Pass observed the counter is 0 < threshold regardless.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn backend_healthy_false_before_first_readiness_pass(
        backend_count in 1usize..=3,
        success_threshold in 1u32..=5,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let facts: Vec<ServiceAllocFact> = (0..backend_count)
            .map(|i| fact_with_readiness(i, None, success_threshold))
            .collect();
        let actual = state_with(facts);
        let desired = actual.clone();
        let view = ServiceLifecycleView::default();
        let tick = tick_at(5000);

        let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);
        prop_assert_eq!(backends.len(), backend_count);
        for (i, b) in backends.iter().enumerate() {
            prop_assert!(
                !b.healthy,
                "backend {} must be unhealthy before first readiness Pass",
                i
            );
        }
    }
}

// K3 no-restart invariant — readiness flapping (an arbitrary sequence
// of Pass/Fail observations applied tick-by-tick to a single backend)
// NEVER emits `Action::RestartAllocation`. Restart is liveness (step
// 03-02); a readiness Fail only drains the backend. Universe = the
// presence/absence of any RestartAllocation across the whole tick
// sequence (asserted zero).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn readiness_flapping_never_restarts(
        flaps in prop::collection::vec(any::<bool>(), 1..=12),
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let mut view = ServiceLifecycleView::default();

        for (tick_n, &passes) in flaps.iter().enumerate() {
            let status = if passes {
                ProbeStatus::Pass
            } else {
                ProbeStatus::Fail { last_fail_reason: "flap".to_string() }
            };
            let actual = state_with(vec![fact_with_readiness(0, Some(status), 1)]);
            let desired = actual.clone();
            let tick = tick_at(6000 + tick_n as u64 * 1000);
            let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
            assert_no_restart(&actions);
            view = next_view;
        }
    }
}
