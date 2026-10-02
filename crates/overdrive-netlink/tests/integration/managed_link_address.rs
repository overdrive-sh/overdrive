//! GH #295 S-ND295-72 — a managed link is correct whatever the host's link
//! configuration: the bridge creation contract of
//! `Client::ensure_bridge(name, mac)` against real kernel links (E22 (a) and
//! (b); feature delta § "[REF] Managed-link identity independent of host link
//! configuration (fresh-host RCA) — pinned 2026-09-26; user rulings of
//! 2026-09-28" (the bridge creation contract)).
//!
//! - (a) A bridge `ensure_bridge` creates carries `mac` from creation: the
//!   first observation after the call reads `addr_assign_type` 3
//!   (`NET_ADDR_SET`) and the address `mac`, with the link down and no set
//!   issued in between. The oracle is deterministic and holds whatever the
//!   host's link configuration, because the kernel marks the address as set
//!   before it emits the add uevent. A create-then-set control in the same
//!   run is recorded as supporting evidence only (it shows the race the fix
//!   removes, where a host link manager rewrites a kernel-random address; it
//!   is never a gate).
//! - (b) `ensure_bridge` on a present link of any kind writes nothing: its
//!   ifindex, address, `addr_assign_type`, kind, and administrative state are
//!   unchanged.
//!
//! Scratch links carry names outside every managed-link glob (`ovd-gbr*`,
//! `ovd-tp-*`), are created and deleted by this file, and are removed on drop
//! even when an assertion fails. `addr_assign_type` is read from sysfs, a
//! diagnostic read of kernel state; every other fact is read through the
//! crate's own netlink observation.

use std::process::Command;
use std::time::Duration;

use overdrive_netlink::{
    Client, NetlinkError, ObservedLinkIdentity, ObservedLinkKind, block_on_host_netlink,
    create_persistent_tap,
};

use super::tap_queue_attach::{AbsentNameGuard, require_root, scratch_name};

/// The address the created bridge must carry from creation: the production
/// node bridge address (`GUEST_BRIDGE_MAC`), a locally administered unicast.
const CREATE_MAC: [u8; 6] = [0x02, 0x01, 0x00, 0x00, 0x00, 0x01];

/// The address passed to `ensure_bridge` for a present link; it must never be
/// written.
const OTHER_MAC: [u8; 6] = [0x02, 0x95, 0x72, 0x00, 0x00, 0x0b];

/// The address a present link carries before `ensure_bridge` is called.
const PRESENT_MAC: [u8; 6] = [0x02, 0x95, 0x72, 0x00, 0x00, 0x0a];

/// `NET_ADDR_SET`: the address was set by userspace.
const NET_ADDR_SET: u8 = 3;

fn ensure_bridge(name: &str, mac: [u8; 6]) -> Result<(), NetlinkError> {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.ensure_bridge(&owned, mac).await })
}

fn identity(name: &str) -> ObservedLinkIdentity {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.observe_link_identity(&owned).await })
        .unwrap_or_else(|error| panic!("reading the identity of {name}: {error}"))
        .unwrap_or_else(|| panic!("link {name} exists"))
}

fn absent(name: &str) -> bool {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.observe_link_identity(&owned).await })
        .unwrap_or_else(|error| panic!("observing {name}: {error}"))
        .is_none()
}

/// The link's `addr_assign_type` as the kernel reports it in sysfs.
fn addr_assign_type(name: &str) -> u8 {
    let path = format!("/sys/class/net/{name}/addr_assign_type");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {path}: {error}"))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("{path} holds an integer: {error}"))
}

fn format_mac(mac: Option<[u8; 6]>) -> String {
    mac.map_or_else(
        || "none".to_owned(),
        |mac| mac.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":"),
    )
}

/// Run one diagnostic `ip` command that must succeed.
fn ip(args: &[&str]) {
    let output = Command::new("ip")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("spawning `ip {args:?}`: {error}"));
    assert!(
        output.status.success(),
        "`ip {args:?}` failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

/// Wait until systemd-udevd reports `name` initialized, so no host link
/// manager writes it afterwards.
fn udev_wait(name: &str) {
    let output = Command::new("udevadm")
        .args(["wait", "--timeout=10", &format!("/sys/class/net/{name}")])
        .output()
        .unwrap_or_else(|error| panic!("spawning `udevadm wait` for {name}: {error}"));
    assert!(
        output.status.success(),
        "`udevadm wait` for {name} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

/// The facts (b) requires to be unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkFacts {
    ifindex: u32,
    kind: ObservedLinkKind,
    mac: Option<[u8; 6]>,
    addr_assign_type: u8,
    up: bool,
}

fn facts(name: &str) -> LinkFacts {
    let link = identity(name);
    LinkFacts {
        ifindex: link.ifindex,
        kind: link.kind,
        mac: link.mac,
        addr_assign_type: addr_assign_type(name),
        up: link.up,
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-72 — A managed link is correct whatever the host's link configuration.
/// CONTRACT_SHAPE: bounded-change.
///
/// E22 (a): on an absent name, `ensure_bridge(name, mac)` creates a bridge
/// that carries `mac` from creation. The first observation after the call —
/// with no set issued in between — reads kind bridge, address `mac`,
/// `addr_assign_type` 3, and the link down.
#[test]
fn a_created_bridge_carries_its_address_from_creation_and_starts_down() {
    require_root("a_created_bridge_carries_its_address_from_creation_and_starts_down");
    let name = scratch_name("lb");
    let _cleanup = AbsentNameGuard(name.clone());
    assert!(absent(&name), "scratch bridge {name} must not pre-exist");

    ensure_bridge(&name, CREATE_MAC).expect("ensure_bridge creates the absent bridge");
    let first_assign_type = addr_assign_type(&name);
    let first = identity(&name);
    eprintln!(
        "[S-ND295-72 (a)] first observation of {name}: kind={:?} mac={} addr_assign_type={} up={}",
        first.kind,
        format_mac(first.mac),
        first_assign_type,
        first.up
    );

    // Supporting evidence, never a gate: the create-then-set shape the fix
    // replaces, observed after the host's link manager, where one runs, has
    // had time to process the add event; recorded before the assertions so a
    // RED run captures it too.
    let control = scratch_name("lc");
    let _control_cleanup = AbsentNameGuard(control.clone());
    ip(&["link", "add", &control, "type", "bridge"]);
    let control_mac = format_mac(Some(CREATE_MAC));
    ip(&["link", "set", "dev", &control, "address", &control_mac]);
    std::thread::sleep(Duration::from_millis(500));
    eprintln!(
        "[S-ND295-72 (a) control, recorded only] create-then-set {control}: final mac={} \
         addr_assign_type={}",
        format_mac(identity(&control).mac),
        addr_assign_type(&control)
    );

    assert_eq!(first.kind, ObservedLinkKind::Bridge, "the created link is a bridge");
    assert_eq!(
        first_assign_type, NET_ADDR_SET,
        "the address is set by userspace from creation (NET_ADDR_SET), so udev's add event \
         never sees a kernel-random address"
    );
    assert_eq!(first.mac, Some(CREATE_MAC), "the bridge carries the requested address");
    assert!(!first.up, "the bridge is created administratively down");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-72 — A managed link is correct whatever the host's link configuration.
/// CONTRACT_SHAPE: bounded-change.
///
/// E22 (b): on a present link of any kind — a bridge, a dummy, and a
/// persistent TAP, one up and one down among them, each settled (systemd-udevd
/// reports it initialized) — `ensure_bridge` returns `Ok(())` and writes
/// nothing: the ifindex, kind, address, `addr_assign_type`, and administrative
/// state all read back unchanged.
#[test]
fn ensure_bridge_adopts_a_present_link_of_any_kind_without_writing() {
    require_root("ensure_bridge_adopts_a_present_link_of_any_kind_without_writing");
    let present_mac = format_mac(Some(PRESENT_MAC));
    for (row, tag) in [("bridge", "pb"), ("dummy", "pd"), ("persistent TAP", "pt")] {
        let name = scratch_name(tag);
        let _cleanup = AbsentNameGuard(name.clone());
        assert!(absent(&name), "[{row}] scratch link {name} must not pre-exist");
        match row {
            "bridge" => {
                ip(&["link", "add", &name, "address", &present_mac, "type", "bridge"]);
                ip(&["link", "set", "dev", &name, "up"]);
            }
            "dummy" => {
                ip(&["link", "add", &name, "type", "dummy"]);
                ip(&["link", "set", "dev", &name, "address", &present_mac]);
            }
            _ => {
                create_persistent_tap(&name, 0)
                    .unwrap_or_else(|error| panic!("[{row}] creating {name}: {error}"));
            }
        }
        // The host's link manager may still rewrite a new link's address
        // (the fresh-host RCA's race); the precondition is a settled link, so
        // wait until systemd-udevd reports it initialized.
        udev_wait(&name);
        let before = facts(&name);

        ensure_bridge(&name, OTHER_MAC)
            .unwrap_or_else(|error| panic!("[{row}] ensure_bridge adopts {name}: {error}"));
        let after = facts(&name);
        eprintln!("[S-ND295-72 (b)][{row}] before={before:?} after={after:?}");

        assert_eq!(after, before, "[{row}] ensure_bridge on a present link writes nothing");
        assert_ne!(
            after.mac,
            Some(OTHER_MAC),
            "[{row}] the passed address is never written to a present link"
        );
    }
}
