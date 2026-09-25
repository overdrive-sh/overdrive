//! GH #295 D-295-R22 read-back — the TAP debug message mask through
//! `overdrive_netlink::ethtool::{debug_msg_mask, debug_msg_masks}`
//! (FD 1294-1354) against real kernel TAPs.
//!
//! The level is changed with a real `TUNSETDEBUG` on the scratch TAP's queue:
//! the one mutation the launch filter denies, performed here as the test's
//! fault stimulus from a root process that holds the queue.

use std::os::fd::AsRawFd;

use overdrive_netlink::ethtool::{debug_msg_mask, debug_msg_masks};
use overdrive_netlink::{Client, NetlinkError, attach_tap_queue, block_on_host_netlink};

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

    let queue = attach_tap_queue(tap.name()).expect("the scratch TAP hands over a queue");
    // SAFETY: `queue` is an open `/dev/net/tun` descriptor attached to the
    // scratch TAP for the duration of the call; `TUNSETDEBUG` takes its
    // argument by value and writes no memory.
    let rc = unsafe {
        libc::ioctl(queue.as_fd().as_raw_fd(), libc::TUNSETDEBUG, libc::c_ulong::from(CHANGED_LEVEL))
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
