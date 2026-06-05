//! GH #211 — `deregister_local_backend` retry-safety regression.
//!
//! Feature: `unconnected-udp-sendmsg4` follow-up (ADR-0053 rev
//! 2026-06-05, Decisions 2 & 3 / Amendment 3 reversal). Tier 3 (real
//! kernel — real `LOCAL_BACKEND_MAP` + `REVERSE_LOCAL_MAP` over
//! kernel-created BPF maps via the production `EbpfDataplane` load path).
//!
//! # The bug this pins
//!
//! The pre-fix `EbpfDataplane::deregister_local_backend` derived the
//! `REVERSE_LOCAL_MAP` key by reading the forward `LOCAL_BACKEND_MAP`
//! entry FIRST, then removed forward, then removed reverse only
//! `if let Some(entry) = forward_read`. If a prior deregister attempt
//! removed forward but errored on the reverse removal, the caller is
//! expected to retry. On retry the forward read now returns `None`
//! (forward already gone) → the reverse-removal branch is skipped → the
//! function returns `Ok(())`, MASKING the failure and permanently
//! stranding the stale reverse entry. That stale
//! `REVERSE_LOCAL_MAP[(backend_ip, backend_port, proto)] → vip` entry
//! then mis-rewrites the reply source of any datagram from that backend
//! address, presenting a deregistered VIP to the receiver.
//!
//! The fix makes `backend` a caller-supplied parameter (mirroring
//! `register_local_backend`), so the reverse removal is unconditional
//! and keyed without a forward read-back — retries complete the reverse
//! removal regardless of forward state.
//!
//! # Why Tier 3 (no Tier-1 / Tier-2 backstop)
//!
//! `SimDataplane` uses a single atomic `remove`-and-return, so it CANNOT
//! exhibit the forward-removed-then-reverse-survives split state — the
//! faithful repro requires the real two-map adapter. There is no Tier-2
//! `BPF_PROG_TEST_RUN` backstop for this cgroup surface. This test is
//! the structural defense.
//!
//! Tags: `@US-211 @tier3 @real-io @adapter-integration @regression`.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::expect_used, clippy::print_stderr)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;

use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_dataplane::EbpfDataplane;
use overdrive_dataplane::maps::reverse_local_map_handle::ReverseLocalKeyPod;

use super::helpers::veth::{VethError, VethPair};

/// DNS-shape service VIP, distinct from any host-assigned address.
const VIP: Ipv4Addr = Ipv4Addr::new(10, 96, 0, 11);
/// VIP port — the DNS port 53 (nothing binds it; the cgroup path rewrites
/// VIP:53 → backend, so the privileged-port constraint never applies).
const VIP_PORT: u16 = 53;

/// Per-test bpffs pin dir, cleaned on construction + on drop.
struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bring up the production `EbpfDataplane` (real veth for the XDP attach +
/// a per-test bpffs pin dir + cgroup hooks at `/sys/fs/cgroup`). Returns
/// `None` (with a skip message) when `CAP_NET_ADMIN` is absent.
fn bring_up(host: &str, peer: &str) -> Option<(EbpfDataplane, VethPair, PinDirGuard)> {
    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: deregister retry-safety needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return None;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-drs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let pin_guard = PinDirGuard(pin_dir.clone());

    let dataplane = EbpfDataplane::new_with_pin_dir(
        &veth.host,
        &veth.peer,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir with cgroup sendmsg4+recvmsg4 attach");

    Some((dataplane, veth, pin_guard))
}

/// Does `REVERSE_LOCAL_MAP` carry the reverse entry for `backend`?
fn reverse_entry_present(dataplane: &EbpfDataplane, backend: SocketAddrV4, proto: Proto) -> bool {
    let want_key =
        ReverseLocalKeyPod::from_typed(BackendKey::new(*backend.ip(), backend.port(), proto));
    dataplane
        .reverse_local_map_entries()
        .expect("dump REVERSE_LOCAL_MAP entries")
        .iter()
        .any(|(key, _vip)| *key == want_key)
}

/// GH #211 — after a partial-failure (forward removed, reverse survived)
/// the retried `deregister_local_backend` MUST purge the stale reverse
/// entry. With the pre-fix forward-read-back derivation the retry sees a
/// missing forward entry, skips the reverse removal, and the stale
/// reverse entry leaks — mis-rewriting later replies from that backend
/// address to the deregistered VIP.
///
/// Steps:
/// 1. Register `(VIP, VIP_PORT, backend, UDP)` — both forward + reverse
///    present.
/// 2. Remove ONLY the forward `LOCAL_BACKEND_MAP` entry (models the
///    post-partial-failure state).
/// 3. `deregister_local_backend(VIP, VIP_PORT, backend, UDP)` (the retry).
/// 4. Assert `REVERSE_LOCAL_MAP` no longer holds the backend's reverse
///    entry.
#[test]
fn deregister_purges_reverse_entry_when_forward_already_removed() {
    let Some((dataplane, _veth, _pin_guard)) = bring_up("ovd-drs0a", "ovd-drs0b") else {
        return;
    };

    let backend = SocketAddrV4::new(Ipv4Addr::new(10, 244, 0, 7), 8053);
    let proto = Proto::Udp;

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");

    // 1. Register — reverse-first dual-write installs BOTH maps.
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, backend, proto)
            .await
            .expect("register UDP local backend (reverse-first dual-write)");
    });
    assert!(
        reverse_entry_present(&dataplane, backend, proto),
        "precondition: after register the reverse entry for the backend must be present",
    );

    // 2. Model the post-partial-failure state: forward removed, reverse
    //    survives (a prior deregister whose reverse removal errored).
    dataplane
        .remove_local_backend_forward_only(VIP, VIP_PORT, proto)
        .expect("forward-only removal (models partial deregister failure)");
    assert!(
        reverse_entry_present(&dataplane, backend, proto),
        "precondition: the reverse entry must still be present after forward-only removal — \
         this is the stale state the retry must clean up",
    );

    // 3. The retry. With the pre-fix forward-read-back derivation this
    //    sees a missing forward entry and skips the reverse removal,
    //    returning Ok(()) while the stale reverse entry leaks.
    rt.block_on(async {
        dataplane
            .deregister_local_backend(VIP, VIP_PORT, backend, proto)
            .await
            .expect("deregister retry must succeed");
    });

    // 4. The load-bearing assertion: the stale reverse entry is gone.
    //    RED against the forward-read-back body (entry leaks);
    //    GREEN against the caller-supplied-backend body.
    assert!(
        !reverse_entry_present(&dataplane, backend, proto),
        "GH #211: after a deregister retry the stale REVERSE_LOCAL_MAP entry for the backend \
         MUST be purged — a leaked reverse entry mis-rewrites the reply source of any datagram \
         from {backend} to the deregistered VIP {VIP}",
    );
}
