//! Composed bidirectional transparent-mTLS proxy walking skeleton — the FIRST,
//! BLOCKING DELIVER slice (ADR-0069, GH #26; step 01-01, F2).
//!
//! This is an INTEGRATION / walking-skeleton gate, NOT a prove-the-mechanism
//! gate: the 6 committed Tier-3 spikes settled the mechanism (proxy, not in-band
//! kTLS) and proved the composed INBOUND flow end-to-end (increment-i). This test
//! COMPOSES the spike-proven primitives into ONE bidirectional walking skeleton on
//! the REAL netns/veth topology with cgroup-isolated workloads, closing three
//! narrow gaps:
//!   GAP 1 — OUTBOUND composed in ONE flow (increment-e intercept+capture+flush +
//!           increment-f kTLS-TX splice, wired together for the first time);
//!   GAP 2 — bidirectional steady-state round-trip (the response legs the spikes
//!           never composed);
//!   GAP 3 — real netns/veth + cgroup-isolated workloads (all spikes were loopback
//!           + sibling processes).
//!
//! Port-to-port: the test drives the scenario THROUGH the `MtlsEnforcement`
//! driving port — `HostMtlsEnforcement::probe`/`enforce`/`liveness`/`teardown`,
//! and ONLY those four pinned methods. The intercept setup (cgroup_connect4 attach
//! / nft-TPROXY) + the leg-F/leg-C listener + the `accept()` are the WORKER's
//! composition-root role (step 07-01), which the test harness (`MtlsTopology` +
//! the role helpers) stands in for here — they are NOT adapter API. The worker
//! hands the adapter an already-`accept()`ed `InterceptedConnection`.
//!
//! The observables are the real `tcpdump`-shape TLS 1.3 records (`0x17`
//! ciphertext) on the peer-facing leg, the plaintext appearing ONLY on the
//! host-internal leg, NO RST post-arm, under BOTH normal AND traced/delayed
//! timing.
//!
//! Tier 3 ONLY: sockops/cgroup_connect4/TPROXY/kTLS have NO meaningful
//! `BPF_PROG_TEST_RUN`, so there is no Tier-2 backstop — `cargo xtask lima run --
//! cargo nextest run -p overdrive-dataplane --features integration-tests`,
//! ACTUALLY EXECUTING (a `--no-run` gate is green even when every fixture refuses
//! at boot).
//!
//! The agent reads SVID + bundle ONLY via the shipped `IdentityRead` port (#35) —
//! NO #26-local issuance/cache; kTLS arms on the AGENT's leg (leg B / leg C),
//! NEVER the workload's socket. `expected_peer` is `None` in v1 (authn-only).

#![cfg(target_os = "linux")]
// `unwrap`/`expect` are the standard test idiom — a panic with a message is
// exactly the right failure for a precondition.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The SKIP path prints via `eprintln!` (nextest captures it); the role helpers
// take `&mut self` for the shape they will need once their GREEN bodies land
// (they mutate the spawned-child / listener state). Allowed for the RED scaffold.
#![allow(clippy::print_stderr, clippy::needless_pass_by_ref_mut)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    Direction, InterceptedConnection, MtlsEnforcement, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_dataplane::mtls::HostMtlsEnforcement;

use super::helpers::mtls_netns_topology::{MtlsTopology, TopologyError};

mod pki;
mod roles;

use pki::TestPki;
use roles::{InboundServer, OutboundPeer, OutboundWorkload};

/// Test `IdentityRead` double — holds the minted SVIDs (keyed by `AllocationId`)
/// plus the trust bundle, served as owned clones (the contract: a read never
/// issues, never mutates; `None` is explicit absence). The proxy reads through
/// THIS, never mints (#26 is a reader).
struct HeldIdentities {
    svids: std::collections::BTreeMap<AllocationId, SvidMaterial>,
    bundle: TrustBundle,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

/// Build the held-identity store from the test PKI: one SVID per allocation
/// (`client_alloc` for the outbound client leg, `server_alloc` for the inbound
/// server leg), plus the shared trust bundle.
fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    svids.insert(pki.server_alloc.clone(), pki.server_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The composed bidirectional walking-skeleton gate. Drives BOTH the outbound and
/// inbound composed flows through `HostMtlsEnforcement` on the real netns/veth +
/// cgroup topology, under BOTH normal AND traced/delayed timing, asserting NO RST
/// and TLS 1.3 ciphertext on the peer wire with plaintext only host-internal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn composed_bidirectional_proxy_walking_skeleton_no_rst() {
    let tag = format!("{}", std::process::id());
    // The canonical gate runs `cargo xtask lima run -- …` as root on the real 6.18
    // kernel, where the topology is ALWAYS supported (root + CAP_NET_ADMIN + cgroup
    // v2 + nft_tproxy). A `TopologyError::Unsupported` here is therefore NOT a
    // legitimate skip — it is the same "green without executing" hole a `--no-run`
    // gate has (the criterion is "ACTUALLY EXECUTING ... on the real 6.18 kernel").
    // Fail loud so a degraded environment cannot pass the BLOCKING gate by skipping.
    let topo = match MtlsTopology::create(&tag) {
        Ok(t) => t,
        Err(e @ TopologyError::Unsupported(_)) => panic!(
            "composed gate MUST run on the real kernel (root + CAP_NET_ADMIN + cgroup v2 + \
             nft_tproxy); a topology-unsupported here is a gate FAILURE, not a skip — run via \
             `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
             --features integration-tests`: {e}"
        ),
        Err(e) => panic!("topology setup failed (not a skip): {e}"),
    };

    let mut topo = topo;

    let pki = TestPki::mint();
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());

    // Earned-Trust probe (wire → probe → use). The composed gate requires the
    // substrate honours its contract before any connection is enforced.
    adapter.probe().await.expect("Earned-Trust probe must pass on the real 6.18 kernel");

    // Install the inbound nft-TPROXY intercept + the GAP-3 leg-S routing ONCE, via
    // the topology (the single source of truth — `install_tproxy` is RAII-cleaned and
    // FAILURE-PROPAGATING; a setup failure is a hard gate failure, not silent
    // best-effort). A FIXED agent_port lets both timing regimes re-bind the leg-C
    // listener (SO_REUSEADDR) against the one installed rule. `expect` here is a
    // gate-failure precondition (the real kernel always supports it).
    let inbound_agent_port = pick_free_inbound_agent_port();
    topo.install_tproxy(inbound_agent_port)
        .expect("inbound TPROXY + leg-S routing must install on the real 6.18 kernel");

    // Run each direction under BOTH timing regimes (normal + a deliberate
    // handshake-window delay — the increment-e harness intercept-lifecycle RST is
    // the artifact to defeat).
    for handshake_delay in [Duration::ZERO, Duration::from_millis(400)] {
        drive_outbound(&adapter, &pki, &topo, handshake_delay).await;
        drive_inbound(&adapter, &pki, &topo, inbound_agent_port, handshake_delay).await;
    }
}

/// Pick a free ephemeral port for the agent's `IP_TRANSPARENT` leg-C listener — the
/// `tproxy to 127.0.0.1:<port>` target installed once in the topology and re-bound by
/// both timing regimes.
fn pick_free_inbound_agent_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("free agent port");
    l.local_addr().expect("agent port addr").port()
}

/// GAP 1 + GAP 2 (outbound half) + GAP 3: the composed OUTBOUND flow. A
/// cgroup-isolated workload's `connect()` is `cgroup_connect4_mtls`-rewritten to
/// the agent's leg-F listener; the worker accepts leg F and hands the adapter an
/// `InterceptedConnection`. `enforce` drains the pre-arm plaintext losslessly,
/// completes a rustls CLIENT handshake on leg B presenting the held client SVID,
/// arms kTLS, installs the forward egress-redirect, and runs the return splice —
/// bidirectional multi-record transfer, NO RST, 0x17 on leg B.
async fn drive_outbound(
    adapter: &HostMtlsEnforcement,
    pki: &TestPki,
    topo: &MtlsTopology,
    handshake_delay: Duration,
) {
    // The real peer (the outbound mTLS server the agent's leg B dials). It arms
    // kTLS-RX to decrypt the workload's request and replies (the B→F response leg
    // GAP 2 requires).
    let peer = OutboundPeer::spawn(pki);

    // WORKER role (test harness): the worker owns the leg-F listener + the
    // cgroup_connect4 intercept. `OutboundWorkload::run` loads/attaches
    // cgroup_connect4_mtls to the workload cgroup, programs MTLS_REDIRECT_DEST
    // [real_peer → leg-F listener], spawns the cgroup-isolated workload, and
    // `accept()`s leg F. The accepted leg + the request/reply round-trip outcome
    // come back here; ONLY the accepted leg crosses into the adapter.
    let mut workload = OutboundWorkload::run(topo, peer.addr(), handshake_delay);
    let leg_f = workload.accept_leg_f();

    let conn = InterceptedConnection {
        leg: leg_f,
        routed: Routed::Outbound { peer: peer.addr() },
        alloc: pki.client_alloc.clone(),
        expected_peer: None,
    };
    assert_eq!(conn.routed.direction(), Direction::Outbound);

    let handle = adapter
        .enforce(conn)
        .await
        .expect("outbound enforce must reach steady-state-established (NO RST)");
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Running);

    // GAP 2: bidirectional steady-state round-trip — forward F→B AND return B→F,
    // both multi-record. The workload sends a multi-record request and reads the
    // peer's byte-exact reply (proving the return splice).
    let round_trip = workload.join();
    assert!(
        round_trip.forward_delivered_byte_exact,
        "outbound forward F→B must deliver the workload's request byte-exact to the peer"
    );
    assert!(
        round_trip.return_delivered_byte_exact,
        "outbound return B→F must deliver the peer's reply byte-exact to the workload (GAP 2)"
    );
    assert!(
        !round_trip.observed_rst,
        "outbound post-arm transfer must NOT RST in either timing regime"
    );

    // Confidentiality: 0x17 TLS 1.3 records on the peer-facing leg B; the
    // workload's plaintext NEVER on the peer wire.
    let wire = peer.wire_observations();
    assert!(
        wire.app_data_records >= 1,
        "leg B (peer-facing) must carry TLS 1.3 application_data (0x17) records"
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "the workload's plaintext must NEVER appear on the peer-facing leg B"
    );

    adapter.teardown(handle.clone()).await.expect("outbound teardown");
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Gone);
    peer.shutdown();
}

/// GAP 2 (inbound half) + GAP 3: the composed INBOUND flow (increment-i),
/// extended with the S→C response leg. A client connects to the server workload's
/// virtual address; nft-TPROXY redirects to the worker's `IP_TRANSPARENT` leg-C
/// listener; the worker accepts leg C, recovers orig-dst via `getsockname`, and
/// hands the adapter an `InterceptedConnection`. `enforce` server-mTLS-verifies the
/// client SVID, arms kTLS-RX, and splices the decrypted plaintext to leg S —
/// byte-exact at S, NO RST, 0x17 on leg C.
async fn drive_inbound(
    adapter: &HostMtlsEnforcement,
    pki: &TestPki,
    topo: &MtlsTopology,
    agent_port: u16,
    handshake_delay: Duration,
) {
    // The identity-unaware server workload S — a CGROUP-ISOLATED NETNS SUBPROCESS
    // (GAP 3), binding the netns veth IP; holds nothing. The agent's leg-S dial
    // reaches it over the veth via the topology's DNAT of the verbatim orig-dst.
    let server = InboundServer::spawn(topo);

    // WORKER role (test harness): owns the IP_TRANSPARENT leg-C listener. The
    // nft-TPROXY intercept + leg-S routing was installed once by the test via the
    // topology (the single source of truth — F4). `InboundWorker::run` binds leg C on
    // the agreed `agent_port`, spawns the client (presenting a valid client SVID
    // toward the virtual addr), and `accept()`s leg C + recovers orig-dst.
    let mut worker =
        roles::InboundWorker::run(topo, server.addr(), pki, agent_port, handshake_delay);
    let (leg_c, orig_dst) = worker.accept_leg_c_and_orig_dst();

    let conn = InterceptedConnection {
        leg: leg_c,
        routed: Routed::Inbound { orig_dst },
        alloc: pki.server_alloc.clone(),
        expected_peer: None,
    };
    assert_eq!(conn.routed.direction(), Direction::Inbound);

    let handle = adapter
        .enforce(conn)
        .await
        .expect("inbound enforce must reach steady-state-established (server-mTLS OK, NO RST)");
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Running);

    // Confidentiality: 0x17 on the client-facing leg C; plaintext only on leg S.
    // Captured BEFORE join_client consumes the worker.
    let wire = worker.client_wire_observations();

    // S receives the byte-exact decrypted plaintext; the response leg S→C carries
    // S's reply back to the client over leg C's kTLS (GAP 2 inbound half).
    let server_result = server.join();
    let client_result = worker.join_client();
    assert!(
        server_result.received_request_byte_exact,
        "the server workload S must receive the byte-exact decrypted plaintext request"
    );
    assert!(
        client_result.received_response_byte_exact,
        "the client must receive S's response byte-exact over leg C (GAP 2 inbound response leg)"
    );
    assert!(
        !server_result.observed_rst && !client_result.observed_rst,
        "inbound transfer must NOT RST in either timing regime"
    );

    assert!(
        wire.app_data_records >= 1,
        "leg C (client-facing) must carry TLS 1.3 application_data (0x17) records"
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "the request plaintext must NEVER appear on the client-facing leg C"
    );

    adapter.teardown(handle.clone()).await.expect("inbound teardown");
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Gone);
}
