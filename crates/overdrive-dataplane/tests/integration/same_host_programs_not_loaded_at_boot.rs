//! Slice 02 AC2 (`reconciler-convergence-guarantees`, ADR-0297) — after the
//! same-host local-backend path is deleted, the boot loads and attaches none of
//! its three cgroup socket programs and creates none of its two maps; only the
//! public-ingress gateway's `connect4` program stays attached.
//!
//! Real kernel, native metal guest (`cargo xtask metal run --`): the production
//! `EbpfDataplane` is built on a veth pair with the cgroup attach at the root
//! cgroup, exactly as the other Tier-3 tests in this binary do. The oracle is
//! what the kernel shows, never what the loader says: `bpftool cgroup tree`
//! (the attachment list, whose attach types read `cgroup_inet4_connect`,
//! `cgroup_udp4_sendmsg`, `cgroup_udp4_recvmsg`) and `bpftool map show` (the loaded maps). Kernel
//! program and map names truncate at 15 characters, so the deleted programs
//! cannot be told from the gateway's by name; the attachment list by attach
//! type can (one `connect4`, no `sendmsg4`, no `recvmsg4`).

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    reason = "CONTRACT_SHAPE metadata and fixture preconditions"
)]

use overdrive_testing::overdrive_test;

use super::helpers::gateway_connect_rig::{HostRig, snapshot};

/// Attachment lines of one attach type (`bpftool cgroup tree` column) in its output.
fn attachments(tree: &str, attach_type: &str) -> usize {
    tree.lines().filter(|line| line.contains(attach_type)).count()
}

/// Outcome anchor: OUT-RCG-SAME-HOST-PATH-NOT-LOADED
/// CONTRACT_SHAPE: unbounded-preservation.
///
/// Given the dataplane is booted with the cgroup attach at the root cgroup
/// When the kernel's attachment list and loaded maps are read
/// Then exactly one `connect4` program is attached (the gateway's), none is
///      attached at `sendmsg4` or `recvmsg4`, and neither deleted map exists.
#[overdrive_test(guest = "mtls-relay")]
#[test]
fn the_boot_loads_no_same_host_local_backend_program_or_map() {
    let _rig = HostRig::new("nolocal");

    let tree = snapshot("bpftool", &["cgroup", "tree", "/sys/fs/cgroup"]);
    let maps = snapshot("bpftool", &["map", "show"]);

    assert_eq!(
        (
            attachments(&tree, "cgroup_inet4_connect"),
            attachments(&tree, "cgroup_udp4_sendmsg"),
            attachments(&tree, "cgroup_udp4_recvmsg")
        ),
        (1, 0, 0),
        "the boot attached the deleted same-host cgroup socket programs; only the gateway's \
         connect4 stays: {tree}"
    );
    for deleted in ["LOCAL_BACKEND_M", "REVERSE_LOCAL_M", "reverse_local_mi"] {
        assert!(
            !maps.contains(deleted),
            "the boot created a deleted same-host map ({deleted}): {maps}"
        );
    }
}
