//! GH #295 D-295-R22 read-back — the TAP debug message mask through
//! `overdrive_netlink::ethtool::{debug_msg_mask, debug_msg_masks}`
//! (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the audit read-back of the TAP debug message mask)) against real kernel TAPs.
//!
//! The level is changed with a real `TUNSETDEBUG` on the scratch TAP's queue:
//! the one mutation the launch filter denies, performed here as the test's
//! fault stimulus from a root process that holds the queue. The queue is
//! attached with a raw `TUNSETIFF` test helper, not the production
//! `attach_tap_queue` (DELIVER 05-03, with its own contract, S-ND295-38), so
//! these 06-01 bodies depend on no 05-03 code (DISTILL review DR-01).

use std::os::fd::{AsRawFd, OwnedFd};

use overdrive_netlink::ethtool::{debug_msg_mask, debug_msg_masks};
use overdrive_netlink::{Client, NetlinkError, block_on_host_netlink};

use super::tap_queue_attach::{AbsentNameGuard, ScratchTap, require_root, scratch_name};

/// A level inside the `NETIF_MSG_CLASS_COUNT` classes the ethtool bitset
/// reports (bits 0-14), with distinct bits set so a byte-order or word-offset
/// decode error cannot read back the same value.
const CHANGED_LEVEL: u32 = 0x0951;

fn read_mask(iface: &str) -> Result<u32, NetlinkError> {
    let owned = iface.to_owned();
    block_on_host_netlink(|| async move { debug_msg_mask(&owned).await })
}

fn read_masks() -> std::collections::BTreeMap<u32, u32> {
    block_on_host_netlink(debug_msg_masks).expect("the debug-mask dump succeeds")
}

/// Attach one queue to the scratch TAP with a raw `TUNSETIFF`, requesting
/// the flags the production attach requests (`IFF_TAP | IFF_NO_PI |
/// IFF_VNET_HDR`). Test support only: it lets the `TUNSETDEBUG` stimulus run
/// without the 05-03 production `attach_tap_queue`.
fn attach_raw_queue(tap: &str) -> OwnedFd {
    assert!(tap.len() < libc::IFNAMSIZ, "scratch TAP name {tap} fits IFNAMSIZ");
    let tun = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
        .expect("open /dev/net/tun (close-on-exec) to attach the scratch queue");
    // SAFETY: `ifreq` is plain old data for which all-zero bytes are valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(tap.bytes()) {
        *slot = libc::c_char::from_ne_bytes([byte]);
    }
    request.ifr_ifru.ifru_flags =
        libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI | libc::IFF_VNET_HDR)
            .expect("TAP queue flags fit ifr_flags");
    // SAFETY: `tun` is an open `/dev/net/tun` descriptor and `request` is a
    // live, initialised `ifreq` for the whole call; `TUNSETIFF` reads the name
    // and flags and writes back only into `request`.
    let rc = unsafe { libc::ioctl(tun.as_raw_fd(), libc::TUNSETIFF, &raw mut request) };
    assert_eq!(rc, 0, "attach a queue to scratch TAP {tap}: {}", std::io::Error::last_os_error());
    OwnedFd::from(tun)
}

fn ifindex(iface: &str) -> u32 {
    let owned = iface.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.observe_link_identity(&owned).await })
        .unwrap_or_else(|error| panic!("reading the identity of {iface}: {error}"))
        .unwrap_or_else(|| panic!("scratch TAP {iface} exists"))
        .ifindex
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-49 — The platform can read every TAP's debug message level: a fresh
/// TAP reads zero and a changed level reads back singly and in the dump.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 06-01 (S-ND295-49)"]
fn a_fresh_tap_reads_zero_and_a_changed_level_reads_back_singly_and_in_the_dump() {
    require_root("a_fresh_tap_reads_zero_and_a_changed_level_reads_back_singly_and_in_the_dump");
    let tap = ScratchTap::create("dm");
    let index = ifindex(tap.name());

    assert_eq!(read_mask(tap.name()).expect("a fresh TAP's mask reads"), 0, "a fresh TAP reads 0");
    assert_eq!(read_masks().get(&index), Some(&0), "the dump reports the fresh TAP at 0");

    let queue = attach_raw_queue(tap.name());
    // SAFETY: `queue` is an open `/dev/net/tun` descriptor attached to the
    // scratch TAP for the duration of the call; `TUNSETDEBUG` takes its
    // argument by value and writes no memory.
    let rc = unsafe {
        libc::ioctl(queue.as_raw_fd(), libc::TUNSETDEBUG, libc::c_ulong::from(CHANGED_LEVEL))
    };
    assert_eq!(rc, 0, "TUNSETDEBUG: {}", std::io::Error::last_os_error());
    drop(queue);

    assert_eq!(
        read_mask(tap.name()).expect("the changed mask reads"),
        CHANGED_LEVEL,
        "the single read returns the level TUNSETDEBUG set"
    );
    assert_eq!(
        read_masks().get(&index),
        Some(&CHANGED_LEVEL),
        "the dump keys the changed level by the TAP's ifindex"
    );
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-49 — A vanished device is reported with its original cause, never
/// as a zero mask.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 06-01 (S-ND295-49)"]
fn an_absent_device_is_reported_with_its_original_cause() {
    require_root("an_absent_device_is_reported_with_its_original_cause");
    let absent = scratch_name("dx");
    let _absent_guard = AbsentNameGuard(absent.clone());

    match read_mask(&absent) {
        Err(NetlinkError::Ethtool { op, source }) => {
            assert_eq!(op, "debug-get", "a kernel NACK is the debug-get operation");
            assert_eq!(source.raw_os_error(), Some(libc::ENODEV), "the kernel's ENODEV is kept");
        }
        other => panic!("an absent device must fail debug-get with ENODEV, got {other:?}"),
    }
}
