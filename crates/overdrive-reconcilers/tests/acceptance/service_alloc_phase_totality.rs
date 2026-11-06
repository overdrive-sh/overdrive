//! ADR-0276 O4 — `ServiceAllocPhase::from_row` is total over
//! `AllocState x Option<Ipv4Addr>`, and `NotRunningState` is a refinement of
//! `AllocState` that excludes `Running`.
//!
//! Layer 1 (pure, in-process): property-based over the whole input space.
//! Driving port: the pure constructors the hydration uses.
//!
//! NOT YET WIRED: this file references `ServiceAllocPhase`,
//! `ServiceBackendIdentity`, `NotRunningState` and
//! `RunningAllocationWithoutWorkloadAddr`, which the DELIVER change introduces
//! (feature-delta § Interface contracts). The DELIVER change adds
//! `mod service_alloc_phase_totality;` to `tests/acceptance.rs` with the
//! production types; no other edit to this file is needed.

#![allow(clippy::expect_used)]

use std::net::Ipv4Addr;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_reconcilers::service_lifecycle::{
    NotRunningState, ServiceAllocPhase, ServiceBackendIdentity,
};
use proptest::prelude::*;

fn alloc_state() -> impl Strategy<Value = AllocState> {
    prop_oneof![
        Just(AllocState::Pending),
        Just(AllocState::Running),
        Just(AllocState::Draining),
        Just(AllocState::Suspended),
        Just(AllocState::Terminated),
        Just(AllocState::Failed),
    ]
}

fn workload_addr() -> impl Strategy<Value = Option<Ipv4Addr>> {
    prop::option::of(any::<u32>().prop_map(Ipv4Addr::from))
}

fn spiffe() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/workload/payments/alloc/payments-0")
        .expect("static SPIFFE ID is valid")
}

fn row(state: AllocState, workload_addr: Option<Ipv4Addr>) -> AllocStatusRow {
    let node = NodeId::new("local").expect("node id");
    AllocStatusRow {
        alloc_id: AllocationId::new("payments-0").expect("alloc id"),
        workload_id: WorkloadId::new("payments").expect("workload id"),
        node_id: node.clone(),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: node },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: None,
        workload_addr,
        last_terminated: None,
        restart_count: 0,
    }
}

proptest! {
    /// `Running` with an address is a backend identity carrying exactly the row's
    /// address and the given SPIFFE id; `Running` without one is refused naming
    /// the allocation; every other state is `NotRunning` whatever the address.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn from_row_is_total_over_every_state_and_address(
        state in alloc_state(),
        addr in workload_addr(),
    ) {
        let row = row(state, addr);
        let phase = ServiceAllocPhase::from_row(&row, spiffe());
        match (state, addr) {
            (AllocState::Running, Some(ip)) => {
                let phase = phase.expect("Running with an address is a backend");
                prop_assert_eq!(phase.state(), AllocState::Running);
                prop_assert_eq!(
                    phase,
                    ServiceAllocPhase::Running(ServiceBackendIdentity { spiffe: spiffe(), ip })
                );
            }
            (AllocState::Running, None) => {
                let refused = phase.expect_err("Running without an address is refused");
                prop_assert_eq!(refused.alloc_id, row.alloc_id);
            }
            (other, _) => {
                let phase = phase.expect("a non-Running state hydrates with or without an address");
                prop_assert!(
                    matches!(&phase, ServiceAllocPhase::NotRunning(n) if n.get() == other),
                    "{other:?} must be NotRunning({other:?}), got {phase:?}"
                );
                prop_assert_eq!(phase.state(), other);
            }
        }
    }

    /// `NotRunningState::new` rejects exactly `Running` and otherwise returns the
    /// state unchanged.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn not_running_state_excludes_running_and_preserves_every_other_state(
        state in alloc_state(),
    ) {
        let refined = NotRunningState::new(state);
        if state == AllocState::Running {
            prop_assert_eq!(refined, None);
        } else {
            prop_assert_eq!(refined.map(NotRunningState::get), Some(state));
        }
    }
}
