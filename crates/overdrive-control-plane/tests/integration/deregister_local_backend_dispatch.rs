//! Action-shim `deregister_local_backend::dispatch` — mutation kill
//! for the body-replaced-with-`Ok(())` mutant at
//! `crates/overdrive-control-plane/src/action_shim/deregister_local_backend.rs:51:5`.
//!
//! The shim's contract per ADR-0053 § 3: a successful
//! `Action::DeregisterLocalBackend` dispatch invokes
//! [`Dataplane::deregister_local_backend`], which removes the
//! `(vip, vip_port)` entry from `LOCAL_BACKEND_MAP`. Without this
//! test, the mutation "replace dispatch body with `Ok(())`" passes —
//! every assertion downstream of dispatch sees a no-op rather than a
//! real removal, and the cgroup_sock_addr path silently keeps the
//! stale entry. The test asserts the observable post-state: after
//! dispatch, `SimDataplane::local_backend_for((vip, vip_port))`
//! returns `None`.
//!
//! Tier 1 — calls the typed per-arm dispatch fn directly against
//! `SimDataplane`. The matching `register_local_backend::dispatch`
//! shim is intentionally NOT covered here — the walking-skeleton
//! integration test (`backend_discovery_bridge::walking_skeleton`)
//! covers the production EbpfDataplane path end-to-end. This test
//! pins the deregister contract that has no end-to-end coverage in
//! Phase 1 (the walking skeleton tests the registration emission,
//! never the deregistration teardown).

use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_control_plane::action_shim::deregister_local_backend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{ContentHash, CorrelationKey, ServiceId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_sim::adapters::dataplane::SimDataplane;

#[tokio::test]
async fn deregister_local_backend_dispatch_removes_entry_from_dataplane() {
    let dataplane = SimDataplane::new();
    let vip = Ipv4Addr::new(10, 96, 0, 1);
    let vip_port: u16 = 8080;
    let backend = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9090);

    // Precondition — register the backend through the trait so the
    // deregister has something to remove. The trait method is the same
    // surface the matching `register_local_backend::dispatch` shim
    // invokes.
    dataplane
        .register_local_backend(vip, vip_port, backend, Proto::Tcp)
        .await
        .expect("register precondition");
    assert_eq!(
        dataplane.local_backend_for(vip, vip_port, Proto::Tcp),
        Some(backend),
        "fixture sanity: backend must be registered before dispatch",
    );

    // Construct the action shape the runtime would emit. The
    // correlation key derivation mirrors the hydrator's
    // (target=service-map-hydrator/<sid>, purpose=deregister-local-backend)
    // shape per the doc-comment on `Action::DeregisterLocalBackend`.
    let service_id = ServiceId::new(1).expect("ServiceId");
    let target = format!("service-map-hydrator/{service_id}");
    // The fingerprint value is irrelevant to the shim — it only
    // forwards `(vip, vip_port)`. Use a stable byte sequence so the
    // ContentHash construction is deterministic.
    let spec_hash = ContentHash::of(0_u64.to_le_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "deregister-local-backend");
    let action = Action::DeregisterLocalBackend {
        service_id,
        vip,
        vip_port,
        proto: Proto::Tcp,
        backend,
        correlation,
    };

    // Drive the dispatch fn directly — this is the function whose
    // body is mutated. With the mutation "body replaced with Ok(())"
    // the trait method below is never called and the post-state
    // assertion fails.
    deregister_local_backend::dispatch(&action, &dataplane).await.expect("dispatch must succeed");

    // Observable post-state assertion — the local_backends mirror no
    // longer carries the entry. This is the assertion the mutation
    // kills.
    assert_eq!(
        dataplane.local_backend_for(vip, vip_port, Proto::Tcp),
        None,
        "deregister dispatch MUST remove the (vip, vip_port) entry; \
         observed entry still present, indicating the dispatch body \
         did not invoke `Dataplane::deregister_local_backend`",
    );
}
