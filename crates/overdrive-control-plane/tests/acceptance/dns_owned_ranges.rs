//! Guest public DNS resolution — the owned-range list (US-GPD-2, AC-6, AC-7,
//! AC-8; ADR-0359). Default lane, pure: `OwnedAddressRanges::platform` and
//! `owning_range` are driven with no socket and no clock.
//!
//! A scenario that still carries a `should_panic(expected = ...)` marker is
//! pending: its body is complete and fails with the semantic RED the marker
//! names until the behaviour lands.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use ipnet::Ipv4Net;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::WORKLOAD_FRONTEND_BASE;
use overdrive_control_plane::dns_responder::owned_ranges::{OwnedAddressRanges, OwnedRangeKind};
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_dataplane::allocators::VipRange;
use overdrive_testing::placement;
use proptest::prelude::*;

placement!(host_pure);

fn vip_net() -> Ipv4Net {
    Ipv4Net::new(Ipv4Addr::new(10, 200, 0, 0), 24).expect("net")
}

fn owned() -> OwnedAddressRanges {
    let vip = VipRange::new(vec![vip_net()], BTreeSet::from([Ipv4Addr::new(10, 200, 0, 1)]))
        .expect("a valid service VIP pool");
    OwnedAddressRanges::platform(&vip)
}

proptest! {
    /// Contract shape: pure-function. AC-6, AC-8: every address in the mesh
    /// frontend range, the configured VIP pool (reserved addresses included)
    /// or the guest prefix is owned, and is reported against the range the
    /// defining const names; the ranges are read from `WORKLOAD_FRONTEND_BASE`
    /// and `GUEST_NODE_PREFIX` themselves, so changing a const changes the guard.
    #[test]
    fn every_address_in_an_owned_range_is_owned_and_names_its_range(
        a in any::<u32>(), b in any::<u32>(), c in any::<u32>(),
    ) {
        let list = owned();
        let frontend = pick(WORKLOAD_FRONTEND_BASE, a);
        let guest = pick(GUEST_NODE_PREFIX, b);
        let vip = pick(vip_net(), c);
        prop_assert_eq!(list.owning_range(frontend), Some(OwnedRangeKind::MeshFrontend));
        prop_assert_eq!(list.owning_range(guest), Some(OwnedRangeKind::GuestNode));
        prop_assert_eq!(list.owning_range(vip), Some(OwnedRangeKind::ServiceVip));
    }

    /// Contract shape: pure-function. AC-7: an address in no owned range,
    /// including the rest of the private space and `10.99.0.0/16`, is not owned.
    #[test]
    fn an_address_outside_every_owned_range_is_not_owned(raw in any::<u32>()) {
        let address = Ipv4Addr::from(raw);
        let in_owned = WORKLOAD_FRONTEND_BASE.contains(&address)
            || GUEST_NODE_PREFIX.contains(&address)
            || vip_net().contains(&address);
        prop_assume!(!in_owned);
        prop_assert_eq!(owned().owning_range(address), None);
    }
}

fn pick(net: Ipv4Net, entropy: u32) -> Ipv4Addr {
    let host_bits = 32 - u32::from(net.prefix_len());
    let offset = if host_bits == 0 { 0 } else { entropy % (1u32 << host_bits) };
    Ipv4Addr::from(u32::from(net.network()) + offset)
}

/// Contract shape: pure-function. The boundaries of every owned range, pinned
/// with concrete addresses: first and last address of each range are owned, the
/// address just outside each is not (AC-6, AC-7); `10.99.0.0/16` is not owned
/// because no production path steers a guest connection to it as mesh traffic.
#[test]
fn the_edges_of_each_owned_range_are_owned_and_the_next_address_is_not() {
    let list = owned();
    for (inside, kind) in [
        ("10.98.0.0", OwnedRangeKind::MeshFrontend),
        ("10.98.255.255", OwnedRangeKind::MeshFrontend),
        ("100.95.0.0", OwnedRangeKind::GuestNode),
        ("100.95.255.255", OwnedRangeKind::GuestNode),
        ("10.200.0.0", OwnedRangeKind::ServiceVip),
        ("10.200.0.1", OwnedRangeKind::ServiceVip),
        ("10.200.0.255", OwnedRangeKind::ServiceVip),
    ] {
        assert_eq!(list.owning_range(inside.parse().unwrap()), Some(kind), "{inside}");
    }
    for outside in [
        "10.97.255.255",
        "10.99.0.0",
        "10.99.255.255",
        "100.94.255.255",
        "100.96.0.0",
        "10.199.255.255",
        "10.200.1.0",
        "127.0.0.1",
        "8.8.8.8",
    ] {
        assert_eq!(list.owning_range(outside.parse().unwrap()), None, "{outside}");
    }
}

// ---------------------------------------------------------------------------
// Source scan: every owned-range constant is in the guard's list
// ---------------------------------------------------------------------------

/// Address-range constants the guard deliberately does NOT list, each with the
/// ruling that excludes it. A new range constant is neither guarded nor listed
/// here until someone decides which, so this list is the only place a range can
/// be waved through.
const NOT_GUARDED: &[(&str, &str)] = &[(
    "WORKLOAD_SUBNET_BASE",
    "10.99.0.0/16: no production path steers a guest connection to it as mesh traffic \
     (feature-delta, Grounding findings 7; Ruling C, GPD-4)",
)];

/// Every address-range constant declared in production source: a `const` or
/// `static` whose declared type names `Ipv4Net` or `VipRange`, in
/// `crates/*/src/**/*.rs`. Comments are dropped, and each file is cut at its
/// first `#[cfg(test)]` so test-local fixtures are not production ranges.
/// Returns `(constant name, file)`.
fn range_constants(workspace: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for krate in std::fs::read_dir(workspace.join("crates")).expect("read crates/") {
        let src = krate.expect("crates entry").path().join("src");
        if src.is_dir() {
            collect_range_constants(&src, &mut found);
        }
    }
    found
}

fn collect_range_constants(dir: &Path, found: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_range_constants(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let source = std::fs::read_to_string(&path).expect("read source file");
            for line in code_before_test_module(&source).lines() {
                if let Some(name) = range_constant_name(line) {
                    found.push((name, path.clone()));
                }
            }
        }
    }
}

/// The source up to its first `#[cfg(test)]`, with `//` comments removed.
fn code_before_test_module(source: &str) -> String {
    let mut out = String::new();
    for line in source.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        out.push_str(line.split("//").next().unwrap_or_default());
        out.push('\n');
    }
    out
}

/// `Some(NAME)` when `line` declares a `const`/`static` of an address-range type.
fn range_constant_name(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after.trim_start();
        if rest.starts_with('(') {
            rest = rest.split_once(')')?.1.trim_start();
        }
    }
    let rest = rest.strip_prefix("const ").or_else(|| rest.strip_prefix("static "))?;
    let (name, after) = rest.split_once(':')?;
    let declared_type = after.split('=').next()?;
    let is_range = declared_type.contains("Ipv4Net") || declared_type.contains("VipRange");
    let name = name.trim().trim_start_matches("mut ");
    let is_name = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    (is_range && is_name).then(|| name.to_owned())
}

/// The text of `OwnedAddressRanges::platform`: its signature and body.
fn platform_construction(workspace: &Path) -> String {
    let path = workspace.join("crates/overdrive-control-plane/src/dns_responder/owned_ranges.rs");
    let source =
        code_before_test_module(&std::fs::read_to_string(&path).expect("read owned_ranges.rs"));
    let start = source.find("fn platform(").expect("OwnedAddressRanges::platform is declared");
    let open = start + source[start..].find('{').expect("platform has a body");
    let mut depth = 0usize;
    for (offset, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..=open + offset].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("platform's body is not closed");
}

/// Contract shape: pure-function. ADR-0359 ("THE list; adding an owned range is
/// one line"): the owned-range list cannot silently miss a range. The scan finds
/// every address-range constant in production source (selection rule: a `const`
/// or `static` typed `Ipv4Net` or `VipRange`, outside `#[cfg(test)]`) and
/// requires each to be either named in `OwnedAddressRanges::platform`'s
/// construction or listed in `NOT_GUARDED` with its ruling. A newly added
/// allocator range therefore fails here until it is guarded or excluded on the
/// record. The VIP pool, which is configuration and not a constant, is required
/// to arrive through `platform`'s `VipRange` parameter and to be used there.
#[test]
fn every_address_range_constant_is_in_the_owned_list_or_excluded_on_the_record() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let platform = platform_construction(&workspace);
    let constants = range_constants(&workspace);

    // The selection must keep finding the three ranges the design names, or the
    // scan has gone blind.
    for named in ["WORKLOAD_FRONTEND_BASE", "GUEST_NODE_PREFIX", "WORKLOAD_SUBNET_BASE"] {
        assert!(
            constants.iter().any(|(name, _)| name == named),
            "the scan no longer finds {named}; the selection rule needs updating",
        );
    }

    let unaccounted: Vec<String> = constants
        .iter()
        .filter(|(name, _)| !platform.contains(name.as_str()))
        .filter(|(name, _)| !NOT_GUARDED.iter().any(|(excluded, _)| excluded == name))
        .map(|(name, file)| {
            format!("{name} ({})", file.strip_prefix(&workspace).unwrap_or(file).display())
        })
        .collect();
    assert!(
        unaccounted.is_empty(),
        "owned range missing from OwnedAddressRanges::platform and not excluded on the record: {unaccounted:?}",
    );

    for (excluded, reason) in NOT_GUARDED {
        assert!(
            constants.iter().any(|(name, _)| name == excluded),
            "NOT_GUARDED names {excluded}, which no longer exists ({reason})",
        );
        assert!(
            !platform.contains(excluded),
            "{excluded} is both guarded and listed as not guarded ({reason})",
        );
    }

    assert!(
        platform.contains("&VipRange") && platform.matches("vip_range").count() >= 2,
        "owned range missing from OwnedAddressRanges::platform: the service VIP pool is not taken and used: {platform}",
    );
}
