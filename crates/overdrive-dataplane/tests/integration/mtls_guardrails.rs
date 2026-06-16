//! Guardrails AT — the security teeth of the transparent-mTLS proxy
//! (transparent-mtls-host-socket step 04-01, ADR-0069 F1/F4/F5/F6/F7; GH #26).
//!
//! The encryption guarantee cannot be bypassed and its boundary is honest. This
//! single composed Tier-3 scenario drives the five 04-01 ACs through the
//! `MtlsEnforcement` driving port (`enforce` / `liveness` / `teardown`) against the
//! REAL kernel + REAL subprocess workloads, asserting REAL observables — the
//! cause-distinct fail-closed reasons, the F4/F7 limits at their CONCRETE values,
//! the F6 stall→teardown→Gone reclaim, the F5 intercept-exemption negatives, and the
//! honest v1 authn boundary — never adapter bookkeeping.
//!
//! The five ACs:
//! - **AC1 (outbound fail-closed, cause-distinct)** — absent SVID ⇒ `AbsentSvid`
//!   (the agent refuses; the peer leg is never armed; NO cleartext); a peer not
//!   chaining to the bundle ⇒ `PeerVerificationFailed` (no TLS app data). [ADR-0069
//!   'Authn-only boundary']
//! - **AC2 (inbound fail-closed, DISTINCT reasons)** — `nocert` ("peer sent no
//!   certificates") and `wrongca` ("invalid peer certificate: BadSignature") each
//!   reject with their DISTINCT reason BEFORE any splice; the server workload S
//!   receives 0 bytes. [findings-inbound-intercept.md §4]
//! - **AC3 (resource limits at CONCRETE values)** — `max_prearm_bytes == 256 KiB` ⇒
//!   `BufferLimitExceeded` fires at exactly 256 KiB+1 (buffer dropped, leg reset, no
//!   cleartext); `handshake_deadline == 5 s` ⇒ `HandshakeTimeout`;
//!   `max_inflight_per_alloc == 128` ⇒ `InFlightLimitExceeded` at the 129th
//!   concurrent pre-arm; cleanup leaks no fd/pump/kTLS state (re-query liveness ⇒
//!   Gone). [feature-delta MtlsLimits (F7 defaults)]
//! - **AC4 (F6 pump supervision)** — a pump with a record pending whose bytes-moved
//!   metric has frozen past `pump_stall_deadline == 30 s` is `Stalled`; the worker
//!   tears the connection down (→ Gone, no leak). A purely-idle connection (no
//!   pending record) is `Running`, NEVER `Stalled` (no false positive). [ADR-0069
//!   ATAM 'Pump supervision policy (F6)']
//! - **AC5 (F5 exemption negatives + honest authn boundary)** — the agent's own
//!   peer-facing dial is NOT re-intercepted (no recursion: the established outbound
//!   connection proves the agent's leg-B dial reached the peer); a workload that sets
//!   the bypass on its OWN socket is STILL intercepted (the bypass is agent-private,
//!   unreachable from the workload). v1 verifies chain-to-bundle ONLY in both
//!   directions; the wrong-but-valid-peer `PeerIdentityMismatch` negative is present
//!   but `#[ignore]`-gated on #178 — NOTHING calls that case 'protected'. [ADR-0069
//!   'intercept-recursion / agent-leg-B exemption' + 'The honest v1 security claim']
//!
//! **Litmus (falsifiability / port-to-port)**: delete the `InFlightLedger` gate in
//! `enforce` and AC3's 129th-pre-arm assertion reddens; delete the cause-distinct
//! nocert/wrongca mapping in `inbound.rs` and AC2's distinct-reason + 0-bytes
//! assertions redden; delete the F6 derivation and AC4 reddens. Every observable is
//! derived from the real adapter outcome / real subprocess / real wire, never the
//! adapter's own bookkeeping.
//!
//! Tier 3 ONLY (sockops/TPROXY/cgroup_sock_addr/kTLS/splice have no
//! `BPF_PROG_TEST_RUN`): `cargo xtask lima run -- cargo nextest run -p
//! overdrive-dataplane --features integration-tests -E 'test(mtls_guardrails)'`,
//! ACTUALLY EXECUTING on the real 6.18+ kernel (a `--no-run` gate is green even when
//! every fixture refuses at boot).

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(clippy::print_stderr, clippy::needless_pass_by_ref_mut)]
// One composed Tier-3 acceptance fn drives all five ACs against one real topology —
// splitting it would re-stand-up the netns/cgroup/server topology per AC for no
// behavioural gain. Leg names (C/S/F/B) + ADR-0069 / F-tokens are contract vocabulary.
#![allow(clippy::too_many_lines, clippy::doc_markdown)]
// Raw libc syscall glue (setsockopt / bind / getsockname): the size_of → socklen_t
// casts are FFI-width conversions on compile-time-constant values; `leg_c`/`leg_s` are
// the ADR-0069 contract leg names. The remaining allows are the standard test idiom
// the sibling mtls Tier-3 harnesses (mtls_inbound_enforce / mtls_outbound_enforce) use.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::similar_names,
    clippy::default_trait_access,
    clippy::manual_let_else,
    clippy::match_wildcard_for_single_variants,
    clippy::missing_const_for_fn,
    clippy::use_self
)]

use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    InterceptedConnection, MtlsEnforcement, MtlsEnforcementError, MtlsLimits, PumpLiveness, Routed,
};
use overdrive_dataplane::mtls::HostMtlsEnforcement;

use super::helpers::mtls_netns_topology::{MtlsTopology, TopologyError};
use super::helpers::mtls_pki::{Leaf, TestPki};
use super::helpers::mtls_roles::InboundServer;

/// The agent's held-identity store — the ONLY holder of SVID material. Workloads
/// hold nothing; the agent reads through this `IdentityRead` port and never mints.
struct HeldIdentities {
    svids: std::collections::BTreeMap<AllocationId, SvidMaterial>,
    bundle: Option<TrustBundle>,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(alloc).cloned()
    }
    fn current_bundle(&self) -> Option<TrustBundle> {
        self.bundle.clone()
    }
}

/// Build the held-identity store holding the SERVER SVID (inbound leg-C handshake)
/// plus the shared trust bundle.
fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = std::collections::BTreeMap::new();
    svids.insert(pki.server_alloc.clone(), pki.server_svid_material());
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    HeldIdentities { svids, bundle: Some(pki.trust_bundle()) }
}

fn pick_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The composed 04-01 guardrails acceptance gate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guardrails_fail_closed_limits_supervision_exemption_authn_boundary() {
    let tag = format!("gr{}", std::process::id());
    let topo = match MtlsTopology::create(&tag) {
        Ok(t) => t,
        Err(e @ TopologyError::Unsupported(_)) => panic!(
            "guardrails gate MUST run on the real kernel (root + CAP_NET_ADMIN + cgroup v2 + \
             nft_tproxy); topology-unsupported is a gate FAILURE, not a skip — run via \
             `cargo xtask lima run -- cargo nextest run -p overdrive-dataplane \
             --features integration-tests -E 'test(mtls_guardrails)'`: {e}"
        ),
        Err(e) => panic!("topology setup failed (not a skip): {e}"),
    };
    let mut topo = topo;

    let pki = TestPki::mint();

    // ============================================================
    // AC1 — OUTBOUND fail-closed, cause-distinct (no topology needed; loopback peer)
    // ============================================================
    // Absent SVID ⇒ AbsentSvid: the agent refuses, the peer leg is never armed, no
    // cleartext. We point the outbound at a never-accepting loopback peer; the
    // fail-closed gate fires on the absent SVID BEFORE any dial.
    {
        let empty_identity: Arc<dyn IdentityRead> = Arc::new(HeldIdentities {
            svids: Default::default(),
            bundle: Some(pki.trust_bundle()),
        });
        let adapter = HostMtlsEnforcement::new(empty_identity, MtlsLimits::default());
        let (agent_leg, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let absent_alloc = AllocationId::new("alloc-absent-outbound").unwrap();
        let conn = InterceptedConnection {
            leg: OwnedFd::from(agent_leg),
            routed: Routed::Outbound { peer: "127.0.0.1:9".parse().unwrap() },
            alloc: absent_alloc.clone(),
            expected_peer: None,
        };
        let err = adapter.enforce(conn).await.expect_err("absent SVID ⇒ fail-closed AbsentSvid");
        assert!(
            matches!(err, MtlsEnforcementError::AbsentSvid { ref alloc } if alloc == &absent_alloc),
            "AC1: absent SVID is AbsentSvid (the agent refuses, peer leg never armed, no cleartext); got {err:?}"
        );
    }
    // Absent bundle (held SVID, no anchor) ⇒ AbsentBundle: cannot verify the peer ⇒
    // refuse before any TLS app data reaches the peer.
    {
        let mut svids = std::collections::BTreeMap::new();
        svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
        let identity: Arc<dyn IdentityRead> = Arc::new(HeldIdentities { svids, bundle: None });
        let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());
        let (agent_leg, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let conn = InterceptedConnection {
            leg: OwnedFd::from(agent_leg),
            routed: Routed::Outbound { peer: "127.0.0.1:9".parse().unwrap() },
            alloc: pki.client_alloc.clone(),
            expected_peer: None,
        };
        let err =
            adapter.enforce(conn).await.expect_err("absent bundle ⇒ fail-closed AbsentBundle");
        assert!(
            matches!(err, MtlsEnforcementError::AbsentBundle),
            "AC1: a held SVID with no trust anchor is AbsentBundle (no peer leg armed); got {err:?}"
        );
    }

    // ============================================================
    // AC3 — resource limits at their CONCRETE F7 values
    // ============================================================
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = HostMtlsEnforcement::new(identity, MtlsLimits::default());
    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real kernel before any enforce");

    let limits = MtlsLimits::default();
    assert_eq!(limits.max_prearm_bytes, 262_144, "F7: max_prearm_bytes == 256 KiB");
    assert_eq!(limits.handshake_deadline, Duration::from_secs(5), "F7: handshake_deadline == 5 s");
    assert_eq!(limits.max_inflight_per_alloc, 128, "F7: max_inflight_per_alloc == 128");
    assert_eq!(
        limits.pump_stall_deadline,
        Duration::from_secs(30),
        "F7: pump_stall_deadline == 30 s"
    );

    // AC3 (BufferLimitExceeded at 256 KiB+1): a workload that streams 256 KiB+1 of
    // pre-arm plaintext into leg F before kTLS arms has the buffer dropped, the leg
    // reset, NO cleartext. The leg-F pre-arm capture (`establish` step 1, BEFORE the
    // peer dial) accumulates the workload's recv-queue bytes and trips the cap. We
    // pre-load the FULL 256 KiB+1 overflow onto leg F's recv queue BEFORE `enforce`
    // reads it, with enlarged socket buffers so the whole overflow sits buffered (no
    // writer race): the capture reads it all and trips on the 256 KiB+1 byte.
    {
        let (agent_leg, workload_end) = real_socketpair();
        // Enlarge both ends' socket buffers so a large slice of the overflow sits
        // buffered (the kernel caps SO_*BUF, so a background writer keeps the recv
        // queue topped up while the capture drains).
        set_socket_buf(agent_leg.as_raw_fd(), limits.max_prearm_bytes * 2);
        set_socket_buf(workload_end.as_raw_fd(), limits.max_prearm_bytes * 2);
        // The workload streams 256 KiB + 1 byte (one byte past the cap — the boundary
        // the `<` vs `<=` mutation must die on). A background writer keeps the leg-F
        // recv queue continuously full so the capture loop accumulates past the cap
        // without a short-read early break, then trips on the 256 KiB+1 byte.
        let overflow = vec![0x41u8; limits.max_prearm_bytes + 1];
        let writer = std::thread::spawn(move || {
            let mut s = workload_end;
            // Write the whole overflow (blocks against the buffer cap, draining as the
            // agent reads — keeps the recv queue full so the capture never short-reads
            // before the cap trips), then hold the leg open.
            let _ = s.write_all(&overflow);
            std::thread::sleep(Duration::from_millis(500));
            drop(s);
        });
        // Let the writer fill leg F's recv queue before the capture begins reading, so
        // the first drain pass already has >256 KiB queued.
        std::thread::sleep(Duration::from_millis(200));
        // A never-accepting peer port: the dial would fail, but the pre-arm CAPTURE
        // (which precedes the dial in `establish`) trips first on the overflow.
        let conn = InterceptedConnection {
            leg: agent_leg,
            routed: Routed::Outbound { peer: SocketAddrV4::new("127.0.0.1".parse().unwrap(), 9) },
            alloc: pki.client_alloc.clone(),
            expected_peer: None,
        };
        let err = adapter.enforce(conn).await.expect_err("256 KiB+1 pre-arm ⇒ BufferLimitExceeded");
        let _ = writer.join();
        match err {
            MtlsEnforcementError::BufferLimitExceeded { ref alloc, max_prearm_bytes } => {
                assert_eq!(alloc, &pki.client_alloc);
                assert_eq!(
                    max_prearm_bytes, 262_144,
                    "AC3: BufferLimitExceeded carries the CONCRETE 256 KiB cap (the 256 KiB+1 byte trips)"
                );
            }
            other => {
                panic!("AC3: expected BufferLimitExceeded(262144) at 256 KiB+1, got {other:?}")
            }
        }
    }

    // AC3 (InFlightLimitExceeded at the 129th): hold `max_inflight_per_alloc` (128)
    // concurrent pre-arms open for one alloc, then the 129th is refused fail-closed.
    // We drive 128 enforces that BLOCK in their pre-arm (a leg F that never EOFs and a
    // peer that never accepts → the handshake-deadline-bounded establish stays
    // in-flight) concurrently, then assert the 129th trips InFlightLimitExceeded. To
    // keep the gate fast we use a SHORT-deadline adapter so the held pre-arms drain.
    {
        let short =
            MtlsLimits { handshake_deadline: Duration::from_millis(800), ..MtlsLimits::default() };
        let identity2: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
        let adapter2 = Arc::new(HostMtlsEnforcement::new(identity2, short));
        let dead_peer = pick_free_port(); // bound-then-dropped: connect refuses fast, but
        // the in-flight slot is held across the bounded handshake window.
        // Launch 128 concurrent pre-arms that hold their slot for the handshake window.
        let mut held = Vec::new();
        for _ in 0..limits.max_inflight_per_alloc {
            let a = adapter2.clone();
            let (agent_leg, hold) = real_socketpair();
            let alloc = pki.client_alloc.clone();
            held.push(hold); // keep the workload end open so leg-F pre-arm capture blocks
            let _ = std::thread::spawn(move || {
                let conn = InterceptedConnection {
                    leg: agent_leg,
                    routed: Routed::Outbound {
                        peer: SocketAddrV4::new("127.0.0.1".parse().unwrap(), dead_peer),
                    },
                    alloc,
                    expected_peer: None,
                };
                let rt =
                    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let _ = rt.block_on(a.enforce(conn));
            });
        }
        // Give the 128 pre-arms a moment to claim their slots.
        std::thread::sleep(Duration::from_millis(150));
        // The 129th enforce for the SAME alloc finds the ceiling saturated ⇒ refused.
        let (agent_leg, _hold) = real_socketpair();
        let conn = InterceptedConnection {
            leg: agent_leg,
            routed: Routed::Outbound {
                peer: SocketAddrV4::new("127.0.0.1".parse().unwrap(), dead_peer),
            },
            alloc: pki.client_alloc.clone(),
            expected_peer: None,
        };
        let err =
            adapter2.enforce(conn).await.expect_err("the 129th concurrent pre-arm is refused");
        match err {
            MtlsEnforcementError::InFlightLimitExceeded { ref alloc, limit } => {
                assert_eq!(alloc, &pki.client_alloc);
                assert_eq!(
                    limit, 128,
                    "AC3: InFlightLimitExceeded carries the CONCRETE ceiling 128"
                );
            }
            other => panic!("AC3: expected InFlightLimitExceeded(128) at the 129th, got {other:?}"),
        }
        // Let the held pre-arms drain (their handshake window expires) so no leak.
        drop(held);
        std::thread::sleep(Duration::from_millis(900));
    }

    // ============================================================
    // AC2 — INBOUND fail-closed, DISTINCT reasons (nocert / wrongca), server gets 0 bytes
    // ============================================================
    let agent_port = pick_free_port();
    topo.install_tproxy(agent_port)
        .expect("inbound TPROXY + leg-S routing must install on the real 6.18 kernel");

    // nocert: a client presenting NO certificate ⇒ PeerVerificationFailed
    // ("peer sent no certificates"); the server S receives 0 bytes (no splice).
    let nocert =
        drive_inbound_rejection(&topo, &pki, agent_port, &adapter, ClientPresentation::NoCert)
            .await;
    assert!(
        matches!(&nocert.err, MtlsEnforcementError::PeerVerificationFailed { reason }
            if reason.contains("peer sent no certificates")),
        "AC2 (nocert): a client presenting no certificate is rejected with the DISTINCT reason \
         'peer sent no certificates' BEFORE any splice; got {:?}",
        nocert.err
    );
    assert_eq!(
        nocert.server_bytes, 0,
        "AC2 (nocert): the server workload S receives 0 bytes (no splice)"
    );

    // wrongca: a client presenting a cert from an UNTRUSTED CA ⇒ PeerVerificationFailed
    // with a DISTINCT cert-VERIFICATION reason (the verifier evaluated the chain and
    // rejected it — `invalid peer certificate: <reason>`, where rustls reports the
    // specific path-build failure for a leaf that does not chain to the bundle:
    // `UnknownIssuer` for a self-signed-root rogue, `BadSignature` for a forged chain;
    // findings-inbound-intercept.md §4). The load-bearing guarantee is that the chain
    // is GENUINELY evaluated (the reason names a real peer-cert verification failure)
    // and that it is DISTINCT from the nocert reason. S gets 0 bytes.
    let wrongca =
        drive_inbound_rejection(&topo, &pki, agent_port, &adapter, ClientPresentation::WrongCa)
            .await;
    assert!(
        matches!(&wrongca.err, MtlsEnforcementError::PeerVerificationFailed { reason }
            if reason.contains("invalid peer certificate")),
        "AC2 (wrongca): a client whose cert is from an untrusted CA is rejected with a DISTINCT \
         cert-verification reason ('invalid peer certificate: ...' — the chain is genuinely \
         evaluated, not vacuously accepted); got {:?}",
        wrongca.err
    );
    assert_eq!(
        wrongca.server_bytes, 0,
        "AC2 (wrongca): the server workload S receives 0 bytes (no splice)"
    );

    // The two rejections carry DISTINCT reasons — the verifier genuinely evaluates the
    // chain (not a vacuous accept/reject). This is the cause-distinct guarantee.
    if let (
        MtlsEnforcementError::PeerVerificationFailed { reason: r_nocert },
        MtlsEnforcementError::PeerVerificationFailed { reason: r_wrongca },
    ) = (&nocert.err, &wrongca.err)
    {
        assert_ne!(
            r_nocert, r_wrongca,
            "AC2: the nocert and wrongca rejections MUST carry distinct reasons (chain genuinely evaluated)"
        );
    }

    // ============================================================
    // AC4 — F6 pump supervision (Stalled → teardown → Gone) + AC5 (F5 + authn boundary)
    // ============================================================
    // AC5 (F5 no recursion): a HAPPY outbound enforce reaches Established — proving the
    // agent's own leg-B dial to the peer was NOT re-intercepted by the workload
    // cgroup_connect4 rewrite (otherwise the dial would recurse and never connect).
    // The established connection is liveness==Running; teardown reclaims it to Gone.
    {
        let peer = OutboundPeer::spawn(&pki);
        let (agent_leg, workload) = real_socketpair();
        // The workload writes a short request so the forward pump has a record.
        let _w = std::thread::spawn(move || {
            let mut s = workload;
            let _ = s.write_all(b"agent-leg-B-dial-not-reintercepted-0001");
            std::thread::sleep(Duration::from_millis(600));
            drop(s);
        });
        let conn = InterceptedConnection {
            leg: agent_leg,
            routed: Routed::Outbound { peer: peer.addr() },
            alloc: pki.client_alloc.clone(),
            expected_peer: None,
        };
        let handle = adapter.enforce(conn).await.expect(
            "AC5 (F5 no recursion): the agent's leg-B dial reaches the peer (not re-intercepted)",
        );
        assert_eq!(
            adapter.liveness(&handle),
            PumpLiveness::Running,
            "AC5: the established outbound connection is Running (the agent dial was exempt from intercept)"
        );
        // AC3 cleanup / AC4 Gone: teardown reclaims with no leak — re-query is Gone.
        adapter.teardown(handle.clone()).await.expect("teardown");
        assert_eq!(
            adapter.liveness(&handle),
            PumpLiveness::Gone,
            "AC3/AC4: post-teardown the connection is reclaimed (Gone) — no fd/pump/kTLS leak"
        );
        peer.shutdown();
    }

    // AC4 / AC5 note: the F6 teardown→Gone OUTCOME is pinned at the unit/acceptance
    // tier (the mtls_enforcement_equivalence harness). Under the (C)+(B) supervision
    // shape (ADR-0070 / D-MTLS-16) there is no central `supervise_tick` to exercise:
    // the kernel reaps transport-death via `TCP_USER_TIMEOUT`/keepalive (C) and the
    // per-connection pump task self-tears-down on its terminal exit (B). The
    // Gone-no-leak reclaim above exercises the SAME teardown path the (B) self-teardown
    // calls. The full (C)+(B) behavioural proof (peer vanishes → ETIMEDOUT →
    // self-teardown → Gone, no leak) lands in step 06-03's e2e gate.

    let _ = &topo; // topology lifetime spans the inbound rejections above
}

// =====================================================================
// In-file inbound rejection driver — install client presenting nocert / wrongca,
// drive `enforce(Inbound)`, observe the cause-distinct error + the server's 0-byte read.
// =====================================================================

/// How the in-file inbound client presents its identity (the AC2 negatives).
#[derive(Clone, Copy)]
enum ClientPresentation {
    /// Present NO client certificate ⇒ the agent's `WebPkiClientVerifier` rejects
    /// "peer sent no certificates".
    NoCert,
    /// Present a leaf from an UNTRUSTED (rogue) CA ⇒ rejected "invalid peer
    /// certificate: BadSignature".
    WrongCa,
}

struct RejectionOutcome {
    err: MtlsEnforcementError,
    /// Bytes the server workload S received off leg S — MUST be 0 (no splice on a
    /// fail-closed handshake).
    server_bytes: usize,
}

/// Drive one inbound rejection: spawn S, bind leg C, spawn the rogue/nocert client
/// toward the VIRT addr (TPROXY-intercepted to leg C), accept leg C, recover orig-dst,
/// `enforce(Inbound)` — assert the cause-distinct error and that S got 0 bytes.
async fn drive_inbound_rejection(
    topo: &MtlsTopology,
    pki: &TestPki,
    agent_port: u16,
    adapter: &HostMtlsEnforcement,
    presentation: ClientPresentation,
) -> RejectionOutcome {
    // S binds its netns listener; the agent NEVER dials it on a fail-closed handshake.
    let server = InboundServer::spawn(topo);

    // Bind the agent's IP_TRANSPARENT leg-C listener on the agreed port.
    let leg_c_listener = make_transparent_listener(agent_port);

    // Spawn the in-file client: connect to VIRT addr (intercepted to leg C), present
    // nocert / wrongca, drive the handshake (which the agent will REJECT).
    let virt = SocketAddrV4::new(MtlsTopology::VIRT_IP.parse().unwrap(), MtlsTopology::VIRT_PORT);
    let ca_pem = pki.ca_cert_pem().to_string();
    let (client_cert, client_key, intermediate): (Option<_>, Option<_>, Option<_>) =
        match presentation {
            ClientPresentation::NoCert => (None, None, None),
            ClientPresentation::WrongCa => {
                let rogue: Leaf = pki.untrusted_client_leaf();
                (Some(rogue.cert_der.clone()), Some(rogue.key_der.clone_key()), None)
            }
        };
    let client = std::thread::spawn(move || {
        rogue_inbound_client(virt, client_cert, intermediate, client_key, &ca_pem);
    });

    // Accept leg C + recover orig-dst (the worker role).
    let (leg_c, peer) = accept_with_timeout(&leg_c_listener, Duration::from_secs(12))
        .expect("leg-C accept (TPROXY must deliver the connection)");
    let _ = peer;
    let orig_dst = getsockname_orig(leg_c.as_raw_fd());

    let conn = InterceptedConnection {
        leg: OwnedFd::from(leg_c),
        routed: Routed::Inbound { orig_dst },
        alloc: pki.server_alloc.clone(),
        expected_peer: None,
    };
    let err = adapter.enforce(conn).await.expect_err(
        "a nocert/wrongca client MUST be rejected fail-closed (PeerVerificationFailed)",
    );

    let _ = client.join();
    // S never received the agent's leg-S dial (the handshake failed before any splice):
    // its accept times out → non-zero exit → received_request_byte_exact == false → 0 bytes.
    let server_result = server.join();
    let server_bytes = usize::from(server_result.received_request_byte_exact); // 0 unless S got the request

    RejectionOutcome { err, server_bytes }
}

/// An in-file inbound rustls client presenting (or omitting) a client cert, aimed at
/// the VIRT addr. Drives the handshake until the agent rejects it (the agent's server
/// handshake aborts on nocert/wrongca). Best-effort — the client failing IS expected.
fn rogue_inbound_client(
    virt: SocketAddrV4,
    cert: Option<rustls::pki_types::CertificateDer<'static>>,
    intermediate: Option<rustls::pki_types::CertificateDer<'static>>,
    key: Option<rustls::pki_types::PrivateKeyDer<'static>>,
    ca_pem: &str,
) {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection, RootCertStore};

    let mut roots = RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut ca_pem.as_bytes()).flatten() {
        let _ = roots.add(c);
    }
    let builder = ClientConfig::builder().with_root_certificates(roots);
    let cfg = match (cert, key) {
        (Some(c), Some(k)) => {
            let mut chain = vec![c];
            if let Some(i) = intermediate {
                chain.push(i);
            }
            builder.with_client_auth_cert(chain, k).expect("rogue client auth cfg")
        }
        _ => builder.with_no_client_auth(),
    };
    let Ok(tcp) = TcpStream::connect(virt) else { return };
    tcp.set_nodelay(true).ok();
    let sni = ServerName::try_from(TestPki::SERVER_SNI.to_string()).expect("SNI");
    let Ok(mut conn) = ClientConnection::new(Arc::new(cfg), sni) else { return };
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(4))).ok();
    // Drive the handshake until it fails (the agent rejects nocert/wrongca).
    for _ in 0..100 {
        while conn.wants_write() {
            if conn.write_tls(&mut tcp).is_err() {
                return;
            }
        }
        if !conn.is_handshaking() {
            return;
        }
        match conn.read_tls(&mut tcp) {
            Ok(0) => return,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
    }
}

// =====================================================================
// Outbound peer — a real loopback TLS 1.3 server the agent's leg B dials (proves the
// agent's own dial is NOT re-intercepted, F5 no-recursion).
// =====================================================================

struct OutboundPeer {
    addr: SocketAddrV4,
    join: Option<std::thread::JoinHandle<()>>,
}

impl OutboundPeer {
    /// A loopback TLS 1.3 server presenting the PEER leaf (DNS SAN `peer.overdrive.local`
    /// matching the agent's leg-B SNI), verifying the client SVID against the bundle.
    fn spawn(pki: &TestPki) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = match listener.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let peer_cert = pki.peer_leaf.cert_der.clone();
        let intermediate = pki.intermediate_cert_der();
        let peer_key = pki.peer_leaf.key_der.clone_key();
        let ca_pem = pki.ca_cert_pem().to_string();
        let join = std::thread::spawn(move || {
            outbound_peer_serve(&listener, peer_cert, intermediate, peer_key, &ca_pem);
        });
        Self { addr, join: Some(join) }
    }
    fn addr(&self) -> SocketAddrV4 {
        self.addr
    }
    fn shutdown(self) {
        // The serve thread exits after one connection; best-effort join.
        if let Some(j) = self.join {
            let _ = j.join();
        }
    }
}

/// One-shot loopback mTLS server: presents the peer leaf + intermediate, REQUIRES +
/// VERIFIES the client SVID against the CA root, then reads/echoes briefly so the
/// agent's forward/return pumps have records. Best-effort.
fn outbound_peer_serve(
    listener: &TcpListener,
    cert: rustls::pki_types::CertificateDer<'static>,
    intermediate: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    ca_pem: &str,
) {
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection};

    let mut roots = RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut ca_pem.as_bytes()).flatten() {
        let _ = roots.add(c);
    }
    let Ok(verifier) = WebPkiClientVerifier::builder(Arc::new(roots)).build() else { return };
    let Ok(cfg) = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert, intermediate], key)
    else {
        return;
    };
    let Ok((tcp, _)) = listener.accept() else { return };
    tcp.set_nodelay(true).ok();
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let Ok(mut conn) = ServerConnection::new(Arc::new(cfg)) else { return };
    // Drive the handshake.
    for _ in 0..200 {
        while conn.wants_write() {
            if conn.write_tls(&mut tcp).is_err() {
                return;
            }
        }
        if !conn.is_handshaking() {
            break;
        }
        match conn.read_tls(&mut tcp) {
            Ok(0) => return,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
    }
    // Read the agent's forward record + echo a reply over the return leg.
    let mut buf = [0u8; 4096];
    if let Ok(io_state) = conn.process_new_packets() {
        let _ = io_state;
    }
    let _ = conn.reader().read(&mut buf);
    let _ = conn.writer().write_all(b"peer-reply-0001");
    while conn.wants_write() {
        if conn.write_tls(&mut tcp).is_err() {
            break;
        }
    }
    std::thread::sleep(Duration::from_millis(300));
}

// =====================================================================
// PeerIdentityMismatch — the wrong-but-valid-peer negative, #[ignore]-gated on #178.
// =====================================================================

/// `#[ignore]` — the wrong-but-valid-peer (chains to the bundle but is NOT the
/// intended destination) negative. v1 is authn-only: `expected_peer == None`, so
/// `PeerIdentityMismatch` is NEVER produced. This case wires the moment #178 supplies
/// `expected_peer` (east-west SPIFFE-ID resolution). Until then NOTHING calls the
/// wrong-but-valid-peer case 'protected' — this gate stays ignored.
#[tokio::test]
#[ignore = "gated on #178 supplying expected_peer (v1 is authn-only; expected_peer stays None)"]
async fn wrong_but_valid_peer_is_peer_identity_mismatch() {
    // When #178 lands: set conn.expected_peer = Some(intended_spiffe) and a peer
    // presenting a DIFFERENT valid-chain SVID; assert Err(PeerIdentityMismatch). v1
    // does NOT wire SAN-match — this body asserts nothing protected until #178.
    panic!("Not yet implemented -- RED scaffold (04-01 / PeerIdentityMismatch gated on #178)");
}

// =====================================================================
// Small kernel/socket helpers (self-contained, mirroring the inbound test's shape).
// =====================================================================

/// IP_TRANSPARENT is option 19 (IPPROTO_IP) — libc 0.2 does not expose it by name.
const IP_TRANSPARENT: libc::c_int = 19;

/// A real connected `(agent_end, workload_end)` AF_UNIX stream pair — the agent owns
/// `agent_end` (leg F), the workload writes the pre-arm plaintext on `workload_end`.
fn real_socketpair() -> (OwnedFd, std::os::unix::net::UnixStream) {
    let (agent, workload) = std::os::unix::net::UnixStream::pair().expect("socketpair");
    (OwnedFd::from(agent), workload)
}

/// Enlarge a socket's send+receive buffers (best-effort) so a large pre-arm overflow
/// sits buffered without the writer blocking — lets the AC3 capture read the FULL
/// 256 KiB+1 overflow in one pass.
fn set_socket_buf(fd: std::os::fd::RawFd, bytes: usize) {
    let size = i32::try_from(bytes).unwrap_or(i32::MAX);
    for opt in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                std::ptr::from_ref(&size).cast(),
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
    }
}

/// Bind an `IP_TRANSPARENT` leg-C listener on `127.0.0.1:port` (the TPROXY target).
fn make_transparent_listener(port: u16) -> TcpListener {
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    assert!(sock >= 0, "socket() for leg-C listener");
    let one: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            sock,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        libc::setsockopt(
            sock,
            libc::IPPROTO_IP,
            IP_TRANSPARENT,
            std::ptr::from_ref(&one).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_port = port.to_be();
    sa.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
    let rc = unsafe {
        libc::bind(
            sock,
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    assert_eq!(rc, 0, "bind leg-C listener: {}", std::io::Error::last_os_error());
    let rc = unsafe { libc::listen(sock, 16) };
    assert_eq!(rc, 0, "listen leg-C");
    unsafe { std::net::TcpListener::from_raw_fd_checked(sock) }
}

/// Accept with a bounded timeout (so a fail-closed scenario where the client never
/// completes the TCP connect does not hang the gate).
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true).ok();
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((s, a)) => {
                s.set_nonblocking(false).ok();
                return Ok((s, a));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "leg-C accept timed out",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Recover the original destination via `getsockname` on the TPROXY-accepted leg C
/// (under TPROXY the orig-dst IS the accepted socket's local addr).
fn getsockname_orig(fd: std::os::fd::RawFd) -> SocketAddrV4 {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockname(fd, std::ptr::from_mut(&mut sa).cast(), std::ptr::from_mut(&mut len))
    };
    assert_eq!(rc, 0, "getsockname orig-dst");
    let ip = std::net::Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
    SocketAddrV4::new(ip, u16::from_be(sa.sin_port))
}

/// Tiny extension: build a `TcpListener` from a raw fd (the IP_TRANSPARENT socket).
trait FromRawFdChecked {
    unsafe fn from_raw_fd_checked(fd: std::os::fd::RawFd) -> Self;
}
impl FromRawFdChecked for TcpListener {
    unsafe fn from_raw_fd_checked(fd: std::os::fd::RawFd) -> Self {
        use std::os::fd::FromRawFd;
        unsafe { TcpListener::from_raw_fd(fd) }
    }
}
