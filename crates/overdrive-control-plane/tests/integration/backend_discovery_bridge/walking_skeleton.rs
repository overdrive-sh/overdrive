//! Walking-skeleton acceptance test for `backend-discovery-bridge-service-reachability`
//! (joint #174 + #175 e2e gate).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-01. Tier 3 — runs through `cargo xtask lima run -- cargo nextest run
//! -p overdrive-control-plane -E 'test(walking_skeleton)' --features integration-tests`
//! per `.claude/rules/testing.md` § "Running tests — Lima VM".
//!
//! RED scaffold per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
//! failing commits": every test body is `#[should_panic(expected = "RED scaffold")]`
//! with `panic!("Not yet implemented -- RED scaffold (<scenario>)")`. This is
//! the ONLY sanctioned RED test shape; bare `panic!()` without the attribute,
//! and `#[ignore]` for "production code doesn't exist yet" are forbidden.
//!
//! GREEN transition: DELIVER Slice 2 (closes #175) replaces each `panic!` with
//! the real test body and drops the `#[should_panic]` attribute. The
//! scaffold body MUST NOT import not-yet-existent production types
//! (`BackendDiscoveryBridgeReconciler`, `Action::WriteServiceBackendRow`,
//! `DataplaneBootError`, etc.) — those compile only once DELIVER lands them.
//!
//! Scenarios covered by this file:
//!
//! - S-BDB-01 — submit Service, TCP round-trip succeeds through VIP (the joint
//!   #174+#175 walking-skeleton)
//! - S-BDB-18 — graceful shutdown: XDP programs detach, bpffs pins removed
//!   (the Drop-RAII counterpart that the walking-skeleton fixture exercises
//!   on teardown)
//! - S-BDB-19 — `ServiceMapHydrator` picks up bridge-written row and emits
//!   `DataplaneUpdateService` (the bridge-to-hydrator handoff in-process; the
//!   walking-skeleton is the real-kernel evidence, this is the in-process
//!   evidence at Tier 3 against `LocalObservationStore` + `EbpfDataplane`)
//!
//! Boot-composition scenarios (S-BDB-11 through S-BDB-17, S-BDB-20) live in
//! the sibling module `boot_composition.rs`.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn submit_service_workload_tcp_round_trip_through_vip_succeeds() {
    // S-BDB-01 — walking-skeleton e2e gate. See
    // docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md
    // for the full GIVEN/WHEN/THEN spec, including:
    //   - K1 bind-readiness wait: 50ms cadence / 2s budget poll-connect loop
    //   - K2 listener choice: Python one-liner echo (Form A per DWD-03)
    //   - K3 echo payload: literal bytes `walking-skeleton-probe\n`
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-01 / walking-skeleton e2e: \
         submit Service -> Running -> bridge -> hydrator -> EbpfDataplane -> \
         BACKEND_MAP + SERVICE_MAP populated -> TCP round-trip succeeds)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn graceful_shutdown_detaches_xdp_and_removes_bpffs_pin() {
    // S-BDB-18 — Drop-RAII teardown:
    //   GIVEN EbpfDataplane successfully booted with XDP attached to both ifaces
    //         AND SERVICE_MAP bpffs pin exists at /sys/fs/bpf/overdrive/SERVICE_MAP
    //   WHEN  the test fixture drops the EbpfDataplane (or graceful shutdown fires)
    //   THEN  ip link show lb_veth_a shows NO XDP attachment
    //         AND ip link show lb_veth_b shows NO XDP attachment
    //         AND /sys/fs/bpf/overdrive/SERVICE_MAP does NOT exist
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-18 / graceful shutdown: \
         XDP detach from both ifaces + bpffs SERVICE_MAP pin removed on Drop)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "RED scaffold")]
async fn bridge_to_hydrator_handoff_dispatches_dataplane_update_service() {
    // S-BDB-19 — in-process bridge-to-hydrator handoff at Tier 3 (real
    // LocalObservationStore + real EbpfDataplane):
    //   GIVEN the bridge has written a ServiceBackendRow for a Service workload
    //   WHEN  the convergence loop ticks
    //   THEN  ServiceMapHydrator emits Action::DataplaneUpdateService with the
    //         row's vip + backends
    //         AND the action shim dispatches into EbpfDataplane::update_service
    //         AND a service_hydration_results row with Completed status is written
    //
    // Note: the DST equivalent runs every PR via cargo dst against
    // SimDataplane; this Tier 3 variant proves the same property against
    // the real kernel adapter.
    panic!(
        "Not yet implemented -- RED scaffold (S-BDB-19 / bridge-to-hydrator handoff: \
         ServiceMapHydrator picks up bridge-written row -> DataplaneUpdateService \
         dispatched -> service_hydration_results.Completed observable)"
    );
}
