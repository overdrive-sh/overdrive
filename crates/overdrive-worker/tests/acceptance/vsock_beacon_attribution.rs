//! P2 — beacon intake: peer-CID + live-launch-token attribution (DISTILL,
//! netns-density-295).
//!
//! Translates `docs/feature/netns-density-295/distill/test-scenarios-p2-owner-flows.md`
//! P2-S14 into seeded-`overdrive-sim` `#[tokio::test]` bodies that drive the
//! **production** `VmDriver` beacon accept path through its seam
//! (feature-delta §"[REF] Beacon intake — overdrive-worker::VmDriver (D17)",
//! ATTR-TOKEN/ADR-0177, ATTR-LIVE-CLAIM). Example-based (layer 2–3); the oracle
//! is a before/after observable snapshot (beacon `ClaimSet` holder, the
//! `vm_beacon.connection_closed_unattributed { cause }` counters, and whether a
//! READY was released) — never a private driver field.
//!
//! RED scaffold, gated on the DELIVER roadmap-reconciliation beacon step. The
//! bodies call only design-pinned surface (`BeaconMessage::Ready { launch }`,
//! `GuestCid`, `LaunchToken`) plus a DISTILL-shaped test-support beacon accept
//! seam (`BeaconHarness`), per `test-scenarios-p2-owner-flows.md` §"Testability
//! gaps" #2. CLAUDE.md §"Implement to the design": no production API is invented.
//! Registered in `tests/acceptance.rs` by the author when the beacon scaffold
//! co-arrives; unregistered (dangling) until then.

#![allow(dead_code, unused_imports)]

use overdrive_core::guest_transport::{GuestCid, LaunchToken};
use overdrive_core::id::AllocationId;

// =============================================================================
// DISTILL-shaped test-support seam (gap #2) — provided by the DELIVER beacon
// step. The driver exposes ONLY this observable surface.
// =============================================================================

/// One accepted beacon connection, as the host kernel delivers it: a peer CID,
/// the first line it buffered (a dead launch's queued connection still carries
/// its READY), and whether any bytes were buffered.
#[derive(Debug, Clone)]
struct InjectedBeacon {
    peer_cid: GuestCid,
    first_line: BeaconLine,
    has_buffered_bytes: bool,
}

#[derive(Debug, Clone)]
enum BeaconLine {
    /// A `READY ... launch=<hex>` line carrying a launch token.
    Ready { launch: LaunchToken },
    /// Any non-READY first message (closed without effect).
    Other(String),
    /// No line (a queued connection with no buffered bytes; fails its read).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BeaconCloseCause {
    StaleLaunch,
    NoLiveLaunch,
    SessionHeld,
    Malformed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BeaconCounters {
    stale_launch: u64,
    no_live_launch: u64,
    session_held: u64,
    malformed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BeaconObservable {
    /// The connection id holding the CID's beacon `ClaimSet<GuestCid>` claim.
    claim_holder: Option<u64>,
    /// `true` once a READY of the live launch has been released (claim taken).
    ready_released: bool,
    counters: BeaconCounters,
}

/// Harness over the production `VmDriver` beacon listener wired with seeded
/// `Sim*` ports. Each method forwards to the DELIVER beacon seam; it holds no
/// business logic.
struct BeaconHarness;

impl BeaconHarness {
    fn seeded(_seed: u64) -> Self {
        unimplemented!("DELIVER beacon step: production VmDriver + Sim* ports")
    }

    /// Starts a launch of `alloc` on `cid` with `launch` as its live token (the
    /// driver's launch is live: started, not stopped).
    async fn start_launch(&self, _alloc: &AllocationId, _cid: GuestCid, _launch: LaunchToken) {
        unimplemented!("DELIVER beacon step: start a live launch on the CID")
    }

    /// `driver.stop`: ends the launch's liveness in the step that closes its
    /// beacon session and releases its claim.
    async fn stop_launch(&self, _alloc: &AllocationId) {
        unimplemented!("DELIVER beacon step: driver.stop seam")
    }

    /// Delivers one accepted beacon connection through the seam (unread).
    async fn deliver(&self, _beacon: InjectedBeacon) {
        unimplemented!("DELIVER beacon step: beacon accept injection seam")
    }

    /// Drives the beacon accept loop one logical step (reads the first line,
    /// runs the live-launch + token compare and the claim in one per-CID step).
    async fn pump(&self) {
        unimplemented!("DELIVER beacon step: single-step pump of the accept loop")
    }

    fn observe(&self, _cid: GuestCid) -> BeaconObservable {
        unimplemented!("DELIVER beacon step: observable surface accessor")
    }
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("valid allocation id")
}

fn token(seed: u8) -> LaunchToken {
    LaunchToken::from_bytes([seed.max(1); 16]).expect("non-zero token")
}

fn cid(raw: u32) -> GuestCid {
    GuestCid::new(raw).expect("CID in the leased range")
}

// =============================================================================
// P2-S14 — beacon keyed by peer CID + live launch token (D17, ATTR-LIVE-CLAIM)
// =============================================================================

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn ready_of_the_live_launch_takes_the_claim_exactly_once() {
    let h = BeaconHarness::seeded(1);
    let a = alloc("payments");
    let c = cid(0x0100_0005);
    let launch = token(0x61);
    h.start_launch(&a, c, launch).await;

    let before = h.observe(c);
    // The live launch's own READY.
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::Ready { launch },
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let after = h.observe(c);

    // The READY took the beacon claim exactly once; nothing counted unattributed.
    assert!(after.ready_released, "the live launch's READY is released");
    assert!(after.claim_holder.is_some(), "the claim is held by the READY connection");
    assert_eq!(after.counters, before.counters, "no unattributed close");
}

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn queued_previous_launch_beacon_is_closed_unattributed_and_releases_nothing() {
    // A beacon connection the CID's PREVIOUS launch left queued (buffered READY
    // with the stale token), delivered before the current launch's own.
    let h = BeaconHarness::seeded(2);
    let a = alloc("payments");
    let c = cid(0x0100_0006);
    let previous = token(0x62);
    let current = token(0x63);
    // The CID is reused by the current launch.
    h.start_launch(&a, c, current).await;

    let before = h.observe(c);
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::Ready { launch: previous }, // buffered stale READY
        has_buffered_bytes: true,
    })
    .await;
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::Ready { launch: current },
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let after = h.observe(c);

    // The stale queued beacon releases nothing and is counted StaleLaunch; the
    // current launch's READY is released.
    assert_eq!(after.counters.stale_launch, before.counters.stale_launch + 1);
    assert!(after.ready_released, "the current launch's READY is still released");
}

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn ready_read_after_driver_stop_is_no_live_launch() {
    // ATTR-LIVE-CLAIM: a connection accepted while the launch is live and READ
    // ONLY AFTER driver.stop takes nothing (NoLiveLaunch).
    let h = BeaconHarness::seeded(3);
    let a = alloc("payments");
    let c = cid(0x0100_0007);
    let launch = token(0x64);
    h.start_launch(&a, c, launch).await;

    // Deliver the connection (accepted, unread), then stop the launch, then read.
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::Ready { launch },
        has_buffered_bytes: true,
    })
    .await;
    h.stop_launch(&a).await;
    let before = h.observe(c);
    h.pump().await; // read happens after driver.stop
    let after = h.observe(c);

    assert_eq!(after.counters.no_live_launch, before.counters.no_live_launch + 1);
    assert_eq!(after.claim_holder, None, "a stopped launch's connection takes no claim");
    assert!(!after.ready_released, "no READY is released for a stopped launch");
}

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn second_concurrent_connection_of_the_live_launch_is_session_held() {
    let h = BeaconHarness::seeded(4);
    let a = alloc("payments");
    let c = cid(0x0100_0008);
    let launch = token(0x65);
    h.start_launch(&a, c, launch).await;

    for _ in 0..2 {
        h.deliver(InjectedBeacon {
            peer_cid: c,
            first_line: BeaconLine::Ready { launch },
            has_buffered_bytes: true,
        })
        .await;
    }
    h.pump().await;
    let after = h.observe(c);

    // Exactly one claim holder; the second concurrent connection is SessionHeld.
    assert!(after.claim_holder.is_some(), "one connection holds the beacon claim");
    assert_eq!(after.counters.session_held, 1, "the second is SessionHeld");
    assert_eq!(after.counters.stale_launch, 0, "neither carries a stale token");
}

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn queued_connection_with_no_buffered_bytes_fails_its_first_read() {
    let h = BeaconHarness::seeded(5);
    let a = alloc("payments");
    let c = cid(0x0100_0009);
    let launch = token(0x66);
    h.start_launch(&a, c, launch).await;

    let before = h.observe(c);
    // A queued connection with no buffered bytes (fails its first read, closed).
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::None,
        has_buffered_bytes: false,
    })
    .await;
    h.pump().await;
    let after = h.observe(c);

    // Closed like any read failure; takes no claim; releases nothing.
    assert_eq!(after.claim_holder, before.claim_holder, "no claim taken on a read failure");
    assert!(!after.ready_released);
}

#[tokio::test]
#[ignore = "pending DELIVER: VmDriver beacon intake seam + LaunchToken attribution (roadmap-reconciliation beacon step)"]
async fn non_ready_first_message_is_closed_without_effect() {
    let h = BeaconHarness::seeded(6);
    let a = alloc("payments");
    let c = cid(0x0100_000A);
    let launch = token(0x67);
    h.start_launch(&a, c, launch).await;

    let before = h.observe(c);
    h.deliver(InjectedBeacon {
        peer_cid: c,
        first_line: BeaconLine::Other("EXEC".to_string()),
        has_buffered_bytes: true,
    })
    .await;
    h.pump().await;
    let after = h.observe(c);

    assert_eq!(after.claim_holder, None, "a non-READY first message takes no claim");
    assert_eq!(after.counters.malformed, before.counters.malformed + 1);
}
