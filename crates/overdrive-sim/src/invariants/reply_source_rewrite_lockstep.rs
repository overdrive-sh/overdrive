//! `ReplySourceRewriteLockstep` — unconnected-udp-sendmsg4 Slice 02
//! (US-02; J-PLAT-004 / K3). GH #200, ADR-0053 rev 2026-06-05.
//!
//! **Always invariant**: after `register_local_backend(vip, vip_port,
//! backend, proto)`, the `SimDataplane` reply mirror carries
//! `BackendKey(backend_ip, backend_port, proto) → vip` — i.e. the reply
//! source the unconnected-UDP recvmsg4 path would present for that
//! backend identity is the **VIP**, never the backend. Every forward
//! `local_backend` entry has a matching reply-mirror entry; deregister
//! purges both in lockstep.
//!
//! This is the **structural defense BELOW Tier-3** for the reply-source
//! identity. There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for
//! `cgroup_sock_addr` (ENOTSUPP ≤ 6.8), so the kernel recvmsg4 reply
//! rewrite is a Tier-3-only gate; this Tier-1 invariant pins the SAME
//! observable contract on the Sim adapter, meeting Tier-3 at the shared
//! backend identity (the two-pronged pin). A forward-only / asymmetric
//! regression — register writes the forward entry but NOT the reply
//! mirror — turns this RED (the #163-class mutation this slice kills).
//!
//! Mirrors `ReverseNatLockstep`'s shape (the
//! `submit-a-udp-service.yaml` step-4 template), retargeted from the XDP
//! `update_service` / `reverse_nat` wire path to the cgroup
//! `register_local_backend` / `reply_mirror` same-host reply path.

// SPIFFE / SocketAddr literals in this file are structurally total —
// every input is a hand-picked constant the test author can prove
// parses. `expect` here is documentation, not error suppression in an
// unbounded code path.
#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddrV4};

use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::id::NodeId;
use overdrive_core::traits::dataplane::Dataplane;

use crate::adapters::dataplane::SimDataplane;
use crate::harness::{InvariantResult, InvariantStatus};

/// Drive the unconnected-UDP reply-path lockstep scenario and return an
/// `InvariantResult` pinned to the canonical kebab-case name.
///
/// # Scenario
///
/// 1. Build a `SimDataplane` with N same-host UDP services, each one VIP
///    + one local backend, via `register_local_backend(vip, vip_port,
///    backend, Udp)` — the unconnected-UDP shape.
/// 2. After each register, assert
///    `reply_source_for(BackendKey(backend_ip, backend_port, udp)) ==
///    Some((vip, vip_port))` for every registered backend (the reply
///    source the app would read is `(VIP, VIP_PORT)` — IP AND port).
/// 3. Deregister one service; assert its reply-mirror entry is purged
///    (no orphan reverse mapping leaks).
/// 4. Re-register it; assert the reply-mirror entry reappears with the
///    matching VIP.
/// 5. **Per-proto co-resident** — a co-located `(vip, vip_port, Tcp)` +
///    `(vip, vip_port, Udp)` register the same backend address under two
///    distinct reply-mirror keys; deregistering the UDP listener leaves
///    the co-resident TCP reply-mirror entry intact.
///
/// The lockstep guarantee comes from `SimDataplane`: `local_backends`
/// and `reply_mirror` live inside one `Mutex<LocalState>`, and
/// `register_local_backend` writes both under one acquisition (DDD-5d).
/// A forward-only mutation (forward entry written, reply mirror not)
/// fails step 2 — the regression this invariant exists to catch.
pub async fn evaluate_reply_source_rewrite_lockstep() -> InvariantResult {
    const NAME: &str = "reply-source-rewrite-lockstep";
    const SERVICES: u32 = 4;
    const VIP_PORT: u16 = 53;
    const BACKEND_PORT: u16 = 8053;

    let dataplane = SimDataplane::new();

    // Build N same-host UDP services — distinct VIPs and distinct backend
    // addresses so the (vip, backend) cross-product yields unique
    // reply-mirror keys. Each entry is `(vip, vip_port, proto) →
    // backend`, and the expected reply-mirror entry is
    // `BackendKey(backend_ip, backend_port, proto) → vip`.
    let mut layout: BTreeMap<(Ipv4Addr, u16, Proto), (SocketAddrV4, Ipv4Addr)> = BTreeMap::new();
    for s in 0..SERVICES {
        let vip = Ipv4Addr::new(10, 96, 0, u8::try_from(s + 1).expect("s < 256"));
        let backend_ip = Ipv4Addr::new(10, 244, 0, u8::try_from(s + 1).expect("s < 256"));
        let backend = SocketAddrV4::new(backend_ip, BACKEND_PORT);
        layout.insert((vip, VIP_PORT, Proto::Udp), (backend, vip));
    }

    // Install every service. After each register, assert lockstep holds
    // for the cumulative set: every registered backend has a reply-mirror
    // entry pointing to its VIP.
    for (&(vip, vip_port, proto), &(backend, _vip)) in &layout {
        if let Err(e) = dataplane.register_local_backend(vip, vip_port, backend, proto).await {
            return fail(NAME, format!("register_local_backend({vip}:{vip_port}) failed: {e}"));
        }
    }
    if let Some(violation) = check_lockstep(&dataplane, &layout) {
        return fail(NAME, format!("after initial install: {violation}"));
    }

    // Deregister the first service. Its reply-mirror entry MUST be purged.
    let (&(first_vip, first_port, first_proto), &(first_backend, _)) =
        layout.iter().next().expect("SERVICES > 0");
    if let Err(e) =
        dataplane.deregister_local_backend(first_vip, first_port, first_backend, first_proto).await
    {
        return fail(
            NAME,
            format!("deregister_local_backend({first_vip}:{first_port}) failed: {e}"),
        );
    }

    let mut after_deregister = layout.clone();
    after_deregister.remove(&(first_vip, first_port, first_proto));
    if let Some(violation) = check_lockstep(&dataplane, &after_deregister) {
        return fail(NAME, format!("after deregister: {violation}"));
    }
    // The deregistered backend's reply-mirror entry must be ABSENT.
    let removed_key = BackendKey::new(*first_backend.ip(), first_backend.port(), first_proto);
    if let Some(stale_vip) = dataplane.reply_source_for(removed_key) {
        return fail(
            NAME,
            format!(
                "deregistered backend {removed_key} still has reply-mirror entry → \
                 {stale_vip}; expected purged"
            ),
        );
    }

    // Re-register it. The reply-mirror entry must reappear with the VIP.
    if let Err(e) =
        dataplane.register_local_backend(first_vip, first_port, first_backend, first_proto).await
    {
        return fail(NAME, format!("re-register({first_vip}:{first_port}) failed: {e}"));
    }
    if let Some(violation) = check_lockstep(&dataplane, &layout) {
        return fail(NAME, format!("after re-register: {violation}"));
    }

    // Step 5 — per-proto co-resident teardown guard.
    if let Some(violation) = check_coresident_proto_teardown(&dataplane).await {
        return fail(NAME, violation);
    }

    pass(NAME)
}

/// Step 5 of `evaluate_reply_source_rewrite_lockstep` — the per-proto
/// co-resident teardown guard.
///
/// Registers a co-located `(vip, vip_port, Tcp)` + `(vip, vip_port,
/// Udp)` against the SAME backend address — the two reply-mirror keys
/// differ only in their proto byte. Deregistering the UDP listener MUST
/// purge only that proto's key; the co-resident TCP reply-mirror entry
/// survives (per-proto teardown, mirroring `EbpfDataplane`'s
/// `deregister_local_backend` which keys removal on `(vip, vip_port,
/// proto)`). Returns `None` on success, `Some(reason)` on the first
/// violation.
async fn check_coresident_proto_teardown(dataplane: &SimDataplane) -> Option<String> {
    let vip = Ipv4Addr::new(10, 96, 0, 200);
    let vip_port = 53u16;
    let backend = SocketAddrV4::new(Ipv4Addr::new(10, 244, 0, 200), 8053);

    for proto in [Proto::Tcp, Proto::Udp] {
        if let Err(e) = dataplane.register_local_backend(vip, vip_port, backend, proto).await {
            return Some(format!("co-resident register ({proto:?}) failed: {e}"));
        }
    }

    // Pre-teardown: both protos' reply-mirror keys present → (vip,
    // vip_port).
    let expected_src = SocketAddrV4::new(vip, vip_port);
    for proto in [Proto::Tcp, Proto::Udp] {
        let key = BackendKey::new(*backend.ip(), backend.port(), proto);
        if dataplane.reply_source_for(key) != Some(expected_src) {
            return Some(format!(
                "co-resident pre-teardown: reply_source_for({key}) != {expected_src}"
            ));
        }
    }

    // Deregister the UDP listener only.
    if let Err(e) = dataplane.deregister_local_backend(vip, vip_port, backend, Proto::Udp).await {
        return Some(format!("co-resident udp deregister failed: {e}"));
    }

    // The UDP key is purged; the co-resident TCP key survives.
    let udp_key = BackendKey::new(*backend.ip(), backend.port(), Proto::Udp);
    if let Some(stale) = dataplane.reply_source_for(udp_key) {
        return Some(format!(
            "co-resident: udp reply_source_for({udp_key}) still → {stale}; expected purged"
        ));
    }
    let tcp_key = BackendKey::new(*backend.ip(), backend.port(), Proto::Tcp);
    if dataplane.reply_source_for(tcp_key) != Some(expected_src) {
        return Some(format!(
            "co-resident: tcp reply_source_for({tcp_key}) was purged by a udp deregister; \
             per-proto teardown requires it survive"
        ));
    }

    None
}

/// Walk every `(vip, vip_port, proto) → (backend, vip)` entry in
/// `layout` and assert the dataplane's reply mirror carries the matching
/// `BackendKey(backend_ip, backend_port, proto) → vip` entry, AND that no
/// orphan reply-mirror entries exist (every entry maps back to a backend
/// in the live layout). Returns `None` on success, `Some(reason)` on the
/// first violation.
fn check_lockstep(
    dataplane: &SimDataplane,
    layout: &BTreeMap<(Ipv4Addr, u16, Proto), (SocketAddrV4, Ipv4Addr)>,
) -> Option<String> {
    // Build the expected reply-mirror map from the layout — exactly one
    // key per registered backend, under its declared proto. The expected
    // value is the full `(vip, vip_port)` source the recvmsg4 reply path
    // must restore (both IP and PORT, per §D4), NOT a bare VIP.
    let mut expected: BTreeMap<BackendKey, SocketAddrV4> = BTreeMap::new();
    for (&(vip, vip_port, proto), &(backend, _vip)) in layout {
        expected.insert(
            BackendKey::new(*backend.ip(), backend.port(), proto),
            SocketAddrV4::new(vip, vip_port),
        );
    }

    // Forward direction: every expected entry must exist with the full
    // `(vip, vip_port)` reply source — IP AND port.
    for (key, expected_src) in &expected {
        match dataplane.reply_source_for(*key) {
            Some(actual_src) if actual_src == *expected_src => {}
            Some(actual_src) => {
                return Some(format!(
                    "reply_source_for({key}) = {actual_src}; expected {expected_src}"
                ));
            }
            None => {
                return Some(format!(
                    "reply_source_for({key}) missing; expected {expected_src} \
                     (forward-only regression — the asymmetry this invariant kills)"
                ));
            }
        }
    }

    // Reverse direction: no orphan entries — every reply-mirror entry
    // must correspond to a registered backend in the layout. This catches
    // stale-entry leaks after deregister.
    for (key, src) in dataplane.reply_mirror_entries() {
        if !expected.contains_key(&key) {
            return Some(format!(
                "orphan reply-mirror entry {key} → {src}; not present in current layout"
            ));
        }
    }

    None
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: cluster_host(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: cluster_host(),
        cause: Some(cause),
    }
}

fn cluster_host() -> String {
    NodeId::new("cluster").map_or_else(|_| "cluster".to_owned(), |id| id.to_string())
}
