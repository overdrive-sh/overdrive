//! S-GATE — `ServiceMapHydrator` gates mesh-subnet backends out of BOTH
//! load-balancer paths, leaving the local and remote arms unchanged (DISTILL RED
//! scaffold, GH #241, Tier-1 DST / reconciler-logic, default-lane).
//!
//! D-GATE / D-GATE-PRED / `@us-GATE`. The driving port is
//! `ServiceMapHydrator::reconcile`. A three-way split applied BEFORE the existing
//! LOCAL/REMOTE partition:
//!
//!   - `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` -> emits NEITHER
//!     `RegisterLocalBackend` NOR `DataplaneUpdateService` (mesh -> skip;
//!     nft-TPROXY owns delivery);
//!   - `addr == host_ipv4` -> `RegisterLocalBackend` (UNCHANGED LOCAL arm);
//!   - otherwise -> `DataplaneUpdateService` (UNCHANGED REMOTE arm).
//!
//! The two non-mesh arms are the error/edge coverage — they prove the gate does
//! NOT over-fire (a mutant gating everything, or gating nothing, fails here).
//!
//! Mandate 8 (Universe): the reconcile-returned actions'
//! `register_local_backend_count` + `dataplane_update_service_count` + the
//! `View`'s programmed fingerprint; NEVER the hydrator's private partition state.
//! Mandate 9: Tier-1 -> PBT-eligible over the three address classes;
//! `@example`-pin a representative addr per arm (10.99.0.6 mesh / `host_ipv4` local
//! / 10.96.0.50 remote).
//!
//! Spec: `docs/feature/canonical-workload-address-inbound-tproxy/distill/test-scenarios.md` § S-GATE.
//!
//! DELIVER replaces the panic bodies with `ServiceMapHydrator::reconcile` driven
//! over the three arms (the hydrator gains a `workload_subnet: Ipv4Net` mandatory
//! ctor param = the one `WORKLOAD_SUBNET_BASE` source).

#[test]
#[should_panic(expected = "RED scaffold")]
fn mesh_subnet_backend_programs_neither_local_nor_remote_lb_path() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GATE / a backend with \
         addr.ip() in WORKLOAD_SUBNET_BASE emits NEITHER RegisterLocalBackend \
         NOR DataplaneUpdateService -- nft-TPROXY owns mesh delivery)"
    );
}

#[test]
#[should_panic(expected = "RED scaffold")]
fn host_address_backend_still_registers_as_local_backend() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GATE error/edge / a backend with \
         addr == host_ipv4 still emits RegisterLocalBackend -- the gate must NOT \
         over-fire on the LOCAL arm)"
    );
}

#[test]
#[should_panic(expected = "RED scaffold")]
fn non_mesh_non_host_backend_still_drives_dataplane_service_update() {
    panic!(
        "Not yet implemented -- RED scaffold (S-GATE error/edge / a backend \
         neither host_ipv4 nor in WORKLOAD_SUBNET_BASE still emits \
         DataplaneUpdateService -- the gate must NOT over-fire on the REMOTE arm)"
    );
}
