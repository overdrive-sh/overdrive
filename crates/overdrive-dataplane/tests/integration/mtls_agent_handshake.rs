//! Agent mutual-TLS handshake presents the held SVID — server leg C
//! (transparent-mtls-host-socket step 02-02, ADR-0069 F1/F3; GH #26).
//!
//! Step 01-01 (the composed walking skeleton) already proved the full
//! bidirectional handshake end-to-end on the real netns/veth topology. THIS test
//! is the focused **handshake-identity** acceptance test that isolates the
//! identity properties the step's acceptance criteria name for the INBOUND
//! (server-role) direction, through the `MtlsEnforcement` driving port:
//!
//! - **AC2 (inbound, server role)**: the agent completes a TLS 1.3 SERVER
//!   handshake on leg C presenting `server_alloc`'s HELD SVID and REQUIRE+VERIFYs
//!   the client's SVID chains to the bundle; a VALID client cert ⇒ the handshake
//!   succeeds (the inbound client reads S's byte-exact response over leg C).
//! - **AC3 (server role)**: the presented SERVER leaf chains to the root AND its
//!   sole URI SAN is the server workload's SPIFFE id — proven from the captured
//!   handshake at the test tier (`InboundClientResult::presented_server_spiffe`,
//!   read from chain position 0 of the inbound client's verified certificate chain).
//! - **AC4 (server role)**: the server workload holds NO cert and NO key — the
//!   agent reads the leaf material through the `IdentityRead` port; nothing crosses
//!   into the workload. Expressed structurally: the inbound plaintext server is
//!   handed no SVID material — only the agent's `HeldIdentities` double carries it.
//! - **AC5**: `HostMtlsEnforcement::new` takes the `IdentityRead` read-port as a
//!   REQUIRED constructor parameter (no builder, no default) — proven by
//!   construction (the adapter cannot be built without injecting the port).
//!
//! **The outbound (client-role) half (AC1) was dropped at step 04-01.** It
//! exercised the now-deleted `cgroup_connect4_mtls` outbound mechanism (via the
//! removed `OutboundPeer`/`OutboundWorkload` harness); per the deletion discipline
//! (delete tests of deleted code), the outbound scenario is removed here and fresh
//! client-role coverage on the nft-TPROXY outbound path is re-established in steps
//! 05-01/05-03 (AC10 pattern). The surviving inbound half exercises the leg-C
//! handshake-identity path, which the worker rewire did NOT touch.
//!
//! **Litmus (falsifiability)**: if the call-site that wires the identity-read cert
//! resolver were deleted (the agent presented no server leaf, or the wrong leaf),
//! the inbound client's server-cert REQUIRE+VERIFY / the SPIFFE-SAN assertion would
//! go RED — the handshake-identity property is asserted from the CAPTURED
//! handshake, not from the agent's own bookkeeping.
//!
//! Tier 3 ONLY (sockops/TPROXY/kTLS have no `BPF_PROG_TEST_RUN`):
//! `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features
//! integration-tests -E 'test(agent_handshake_presents_held_svid_server_role)'`,
//! ACTUALLY EXECUTING on the real 6.18 kernel (a `--no-run` gate is green even when
//! every fixture refuses at boot).

#![cfg(target_os = "linux")]
// `unwrap`/`expect` are the standard test idiom — a panic with a message is exactly
// the right failure for a precondition.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
// The role helpers take `&mut self` because they mutate the spawned-child / listener
// state they own.
#![allow(clippy::needless_pass_by_ref_mut)]

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
use super::helpers::mtls_pki::TestPki;
use super::helpers::mtls_roles::{InboundServer, InboundWorker};

/// The agent's held-identity store — the ONLY holder of SVID material (AC4). The
/// workloads hold nothing; the agent reads through THIS `IdentityRead` port and
/// NEVER mints (#26 is a reader). `None` is explicit absence (`identity_read.rs`
/// clause 3).
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

/// Build the held-identity store from the test PKI: the client SVID (outbound leg)
/// and the server SVID (inbound leg), plus the shared trust bundle. The leaf
/// material lives HERE, with the agent — never with the workloads (AC4).
fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    svids.insert(pki.server_alloc.clone(), pki.server_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The focused handshake-identity acceptance gate (step 02-02, inbound half). Drives
/// the inbound (server role) handshake through `HostMtlsEnforcement::enforce` on the
/// real netns/veth + nft-TPROXY topology, and asserts the handshake-identity
/// properties (AC2–AC5) from the captured handshake. The outbound (client-role) half
/// was dropped at step 04-01 — see the module docstring.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_handshake_presents_held_svid_server_role() {
    let tag = format!("hs{}", std::process::id());
    // The canonical gate runs `cargo xtask lima run -- …` as root on the real 6.18
    // kernel, where the topology is ALWAYS supported. A `TopologyError::Unsupported`
    // here is NOT a legitimate skip — it is the same "green without executing" hole a
    // `--no-run` gate has. Fail loud so a degraded environment cannot pass by skipping.
    let mut topo = match MtlsTopology::create(&tag) {
        Ok(t) => t,
        Err(e @ TopologyError::Unsupported(_)) => panic!(
            "handshake-identity gate MUST run on the real kernel (root + CAP_NET_ADMIN + \
             cgroup v2 + nft_tproxy); a topology-unsupported here is a gate FAILURE, not a \
             skip — run via `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
             --features integration-tests`: {e}"
        ),
        Err(e) => panic!("topology setup failed (not a skip): {e}"),
    };

    let pki = TestPki::mint();
    // AC4/AC5: the agent reads the held SVID + bundle THROUGH the required-param
    // `IdentityRead` port. `HostMtlsEnforcement::new` cannot be constructed without
    // injecting it — the port is a REQUIRED constructor parameter, no builder, no
    // default (`.claude/rules/development.md` § "Port-trait dependencies"). The
    // workloads below are handed NO SVID material.
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());

    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real 6.18 kernel before any enforce");

    let inbound_agent_port = pick_free_inbound_agent_port();
    topo.install_tproxy(inbound_agent_port)
        .expect("inbound TPROXY + leg-S routing must install on the real 6.18 kernel");

    assert_inbound_handshake_presents_held_server_svid(&adapter, &pki, &topo, inbound_agent_port)
        .await;
}

/// AC2 + AC3 + AC4 (inbound, server role): drive `enforce` on leg C; the agent's
/// leg-C SERVER handshake must present `server_alloc`'s held SVID AND REQUIRE+VERIFY
/// the client's SVID chains to the bundle. A VALID client cert ⇒ the handshake
/// succeeds (the inbound client verifies the agent's server cert chains to the
/// bundle root and extracts its SPIFFE-SAN).
async fn assert_inbound_handshake_presents_held_server_svid(
    adapter: &HostMtlsEnforcement,
    pki: &TestPki,
    topo: &MtlsTopology,
    agent_port: u16,
) {
    // The identity-unaware server workload S — holds NOTHING (AC4). The agent's
    // leg-S dial reaches it over the veth.
    let server = InboundServer::spawn(topo);

    // WORKER role (test harness): owns the IP_TRANSPARENT leg-C listener and spawns
    // the inbound client (which presents a valid client SVID and verifies the agent's
    // server cert). Only the accepted leg C crosses into the adapter.
    let mut worker = InboundWorker::run(topo, server.addr(), pki, agent_port, Duration::ZERO);
    let (leg_c, orig_dst) = worker.accept_leg_c_and_orig_dst();

    let conn = InterceptedConnection {
        leg: leg_c,
        routed: Routed::Inbound { orig_dst },
        alloc: pki.server_alloc.clone(),
        expected_peer: None,
    };
    assert_eq!(conn.routed.direction(), Direction::Inbound);

    let handle = adapter.enforce(conn).await.expect(
        "inbound enforce must complete the server handshake REQUIRE+VERIFYing the valid client \
         SVID (steady-state-established)",
    );
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Running);

    // S received the request; the client read S's response back over leg C — proving
    // the mutual handshake (server presents held SVID AND the client SVID was
    // REQUIRED+VERIFIED) completed for a VALID client cert (AC2).
    let server_result = server.join();
    let (client_result, _wire) = worker.join_client();
    assert!(
        server_result.received_request_byte_exact,
        "AC2: a valid client SVID ⇒ the inbound server handshake succeeds and the byte-exact \
         decrypted request reaches the server workload S"
    );
    assert!(
        client_result.received_response_byte_exact,
        "AC2: the inbound client (which REQUIRE+VERIFYs the agent's server cert chains to the \
         bundle) reads S's response byte-exact over leg C — the mutual handshake completed"
    );

    // AC2 + AC3 (the load-bearing inbound handshake-identity assertion): the inbound
    // client verified the agent's presented SERVER cert chains to the bundle root,
    // and the SPIFFE-SAN it extracted from chain position 0 IS the held server SVID's
    // SPIFFE. If the identity-read cert-resolver wiring were deleted, the agent would
    // present no server leaf and the client's server-cert verification would fail —
    // this goes RED.
    assert_eq!(
        client_result.presented_server_spiffe.as_ref(),
        Some(&pki.server_leaf.spiffe),
        "AC2/AC3: the agent's leg-C SERVER handshake must present server_alloc's HELD SVID; \
         the inbound client must verify it chains to the bundle root and the verified leaf's \
         URI SAN must be the server workload's SPIFFE id"
    );

    adapter.teardown(handle.clone()).await.expect("inbound teardown");
    assert_eq!(adapter.liveness(&handle), PumpLiveness::Gone);
}

/// Pick a free ephemeral port for the agent's `IP_TRANSPARENT` leg-C listener — the
/// `tproxy to 127.0.0.1:<port>` target installed in the topology and bound by the
/// inbound worker.
fn pick_free_inbound_agent_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("free agent port");
    l.local_addr().expect("agent port addr").port()
}
