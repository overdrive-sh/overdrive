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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use overdrive_core::id::{AllocationId, NodeId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeStatus};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::service_lifecycle::{
    ServiceAllocFact, ServiceDataplaneIdentity, ServiceLifecycleReconciler, ServiceLifecycleState,
    ServiceLifecycleView,
};
use overdrive_core::traits::observation_store::AllocState;
use overdrive_core::wall_clock::UnixInstant;
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
    SpiffeId::new(&format!("spiffe://overdrive.local/job/svc/alloc/a{i}")).expect("valid spiffe")
}

fn dataplane_identity() -> ServiceDataplaneIdentity {
    ServiceDataplaneIdentity {
        service_id: ServiceId::new(42).expect("valid service id"),
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
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: latest_readiness,
        has_readiness_probe: true,
        readiness_success_threshold: success_threshold,
        backend_spiffe: spiffe(index),
        backend_addr: SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, u8::try_from(10 + index).unwrap_or(u8::MAX))),
            8080,
        ),
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
        max_attempts: 30,
        startup_deadline: Duration::from_secs(60),
        mechanic_summary: "tcp 127.0.0.1:8080".to_string(),
        inferred: true,
        startup_probes_empty: false,
        latest_readiness_probe: None,
        has_readiness_probe: false,
        readiness_success_threshold: 1,
        backend_spiffe: spiffe(index),
        backend_addr: SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, u8::try_from(10 + index).unwrap_or(u8::MAX))),
            8080,
        ),
    }
}

fn state_with(facts: Vec<ServiceAllocFact>) -> ServiceLifecycleState {
    let mut allocs = BTreeMap::new();
    for fact in facts {
        allocs.insert(fact.alloc_id.clone(), fact);
    }
    ServiceLifecycleState { allocs, service_dataplane: Some(dataplane_identity()) }
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
// (per-backend readiness ∈ {Pass, Fail}) × (seed consecutive_successes
// 0..=10), reconcile emits one WriteServiceBackendRow whose every
// backend carries `healthy = (latest == Pass AND
// consecutive_successes_after_this_tick >= success_threshold)`.
//
// success_threshold = 1 (default), so a single Pass this tick is
// sufficient. Universe = the emitted row's per-backend `healthy`
// flags. The seed counter exercises the persisted-input read path.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
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
        // Seed the persisted consecutive-Pass counter for every alloc
        // BEFORE moving `facts` into the state (avoids a redundant clone).
        let mut view = ServiceLifecycleView::default();
        for fact in &facts {
            view
                .readiness_consecutive_successes
                .insert((fact.alloc_id.clone(), ProbeIdx::new(0)), seed_counter);
        }
        let actual = state_with(facts);
        let desired = actual.clone();
        let tick = tick_at(2000);

        let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);

        prop_assert_eq!(backends.len(), statuses.len(), "one backend per alloc");
        // Backends are emitted in BTreeMap iteration order; facts were
        // keyed `svc-0 .. svc-N` so index aligns with `statuses`.
        for (i, &passes) in statuses.iter().enumerate() {
            // success_threshold = 1; on Pass the counter becomes
            // seed+1 >= 1, so Pass ⇒ healthy. Fail ⇒ not healthy.
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
// THIS TICK iff the post-increment consecutive-Pass count reaches the
// threshold: `healthy == (seed_counter + 1 >= success_threshold)`.
// Universe = (seed consecutive_successes 0..=10) × (success_threshold
// 2..=6). This pins the `>= success_threshold` comparison AND the
// `latest == Pass AND counter >= threshold` conjunction (both operands
// vary independently here, unlike the threshold-1 RECON-07 case where
// Pass alone always satisfies the gate).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn readiness_pass_below_threshold_stays_unhealthy(
        seed_counter in 0u32..=10,
        success_threshold in 2u32..=6,
    ) {
        let reconciler = ServiceLifecycleReconciler::new();
        let fact = fact_with_readiness(0, Some(ProbeStatus::Pass), success_threshold);
        let key = (fact.alloc_id.clone(), ProbeIdx::new(0));
        let mut view = ServiceLifecycleView::default();
        view.readiness_consecutive_successes.insert(key, seed_counter);
        let actual = state_with(vec![fact]);
        let desired = actual.clone();
        let tick = tick_at(7000);

        let (actions, _next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        assert_no_restart(&actions);
        let backends = backend_row_backends(&actions);
        prop_assert_eq!(backends.len(), 1);
        let expected_healthy = seed_counter.saturating_add(1) >= success_threshold;
        prop_assert_eq!(
            backends[0].healthy,
            expected_healthy,
            "Pass with threshold {}: healthy iff (seed {} + 1) >= threshold",
            success_threshold,
            seed_counter
        );
    }
}

/// S-SHCP-RECON-08 (US-04 / K2 recovery) — a backend whose prior
/// readiness was Fail (counter 0) recovers to `healthy = true` on the
/// next-tick Pass within one reconcile (success_threshold = 1).
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
