//! F6 worker supervision policy (transparent-mtls-host-socket step 04-01, ADR-0069
//! D-MTLS-10 / SD-4; GH #26).
//!
//! The dataplane adapter DERIVES `PumpLiveness::Stalled` (SD-2); the WORKER REACTS
//! (D-MTLS-10): `MtlsSupervisor::supervise_tick` point-queries `liveness` for each
//! established connection and tears down the ones that are `Stalled` — teardown +
//! fail-closed reset (→ `Gone`). A `Running` (idle-but-ready) connection is left
//! untouched (no false positive); a `Gone` connection is already reclaimed.
//!
//! Driven through the worker's `MtlsSupervisor` driving port over a scriptable
//! `SimMtlsEnforcement` (the harness flips Running→Stalled via `mark_stalled`, where
//! the host adapter derives it from the real pump's frozen progress). The outcome —
//! the supervisor tears the Stalled connection to Gone and leaves the idle one
//! Running — is read back through the `MtlsEnforcement::liveness` port, never
//! internal bookkeeping.

use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, SpiffeId};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsEnforcement};
use overdrive_worker::mtls_supervisor::MtlsSupervisor;

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid AllocationId")
}

fn svid(spiffe: &str) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0xDE, 0xAD]),
        CertSerial::new("0badc0de").expect("serial"),
        SpiffeId::new(spiffe).expect("spiffe"),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into()),
        UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)),
    )
}

fn bundle() -> TrustBundle {
    TrustBundle::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into()),
        None,
    )
}

fn agent_leg() -> OwnedFd {
    let (agent_end, _peer) = UnixStream::pair().expect("socketpair leg");
    OwnedFd::from(agent_end)
}

fn intercepted(alloc_id: AllocationId) -> InterceptedConnection {
    InterceptedConnection {
        leg: agent_leg(),
        routed: Routed::Inbound { orig_dst: "127.0.0.2:8443".parse().unwrap() },
        alloc: alloc_id,
        expected_peer: None,
    }
}

/// `@in-memory` — `supervise_tick` tears down ONLY the `Stalled` connection
/// (fail-closed reset → `Gone`) and leaves the `Running` connection untouched.
#[tokio::test]
async fn supervise_tick_tears_down_stalled_and_leaves_running_untouched() {
    let held_alloc = alloc("alloc-sup-0");
    let mut held = BTreeMap::new();
    held.insert(held_alloc.clone(), svid("spiffe://overdrive.local/ns/d/sa/s"));
    let identity = Arc::new(SimIdentityRead::new(held, Some(bundle())));
    let enforcement = Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));

    // Two established connections; one stalls, one stays Running (idle-but-ready).
    let stalled = enforcement.enforce(intercepted(held_alloc.clone())).await.unwrap();
    let running = enforcement.enforce(intercepted(held_alloc.clone())).await.unwrap();

    // The pump's progress freezes WHILE a record is pending ⇒ the adapter reports
    // Stalled (the host adapter derives this from the real frozen-progress metric).
    let since = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    enforcement.mark_stalled(&stalled, since);
    assert_eq!(enforcement.liveness(&stalled), PumpLiveness::Stalled { since });
    assert_eq!(enforcement.liveness(&running), PumpLiveness::Running);

    let supervisor = MtlsSupervisor::new(
        enforcement.clone() as Arc<dyn MtlsEnforcement>,
        Arc::new(SimClock::new()),
    );

    // One reconciler tick: the supervisor point-queries liveness and tears down the
    // Stalled connection (and only that one).
    let torn = supervisor.supervise_tick(&[stalled.clone(), running.clone()]).await;
    assert_eq!(torn, vec![stalled.id().clone()], "F6: only the Stalled connection is torn down");

    // The F6 reaction reclaimed the stalled connection (Gone, no leak); the Running
    // connection survives untouched (no false positive).
    assert_eq!(
        enforcement.liveness(&stalled),
        PumpLiveness::Gone,
        "F6: the Stalled connection is torn down → Gone (fail-closed reset, no fd/pump/kTLS leak)"
    );
    assert_eq!(
        enforcement.liveness(&running),
        PumpLiveness::Running,
        "F6 (no false positive): a Running idle connection is NEVER torn down"
    );
}

/// `@in-memory` — a tick over only-`Running`/idle connections tears down NOTHING
/// (the no-false-positive invariant: a quiescent proxy is not disrupted by the
/// supervisor).
#[tokio::test]
async fn supervise_tick_tears_down_nothing_when_all_running() {
    let held_alloc = alloc("alloc-sup-1");
    let mut held = BTreeMap::new();
    held.insert(held_alloc.clone(), svid("spiffe://overdrive.local/ns/d/sa/s"));
    let identity = Arc::new(SimIdentityRead::new(held, Some(bundle())));
    let enforcement = Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));

    let a = enforcement.enforce(intercepted(held_alloc.clone())).await.unwrap();
    let b = enforcement.enforce(intercepted(held_alloc.clone())).await.unwrap();

    let supervisor = MtlsSupervisor::new(
        enforcement.clone() as Arc<dyn MtlsEnforcement>,
        Arc::new(SimClock::new()),
    );
    let torn = supervisor.supervise_tick(&[a.clone(), b.clone()]).await;

    assert!(torn.is_empty(), "no Stalled connection ⇒ nothing torn down");
    assert_eq!(enforcement.liveness(&a), PumpLiveness::Running);
    assert_eq!(enforcement.liveness(&b), PumpLiveness::Running);
}
