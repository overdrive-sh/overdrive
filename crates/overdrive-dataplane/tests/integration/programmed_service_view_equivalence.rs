//! Host/Sim `ProgrammedServiceView` equivalence and the restart case (D-20,
//! ADR-0299; DISTILL obligation 3). Tier 3.
//!
//! `EbpfDataplane` (the real `SERVICE_MAP` outer slot, its inner map, and the
//! fingerprint in inner slots 16,381 and 16,382) and `SimDataplane` are driven
//! through one `update_service` call sequence. After each step the port reads
//! the same value from both, and each equals the literal oracle
//! `fingerprint(vip, applied backends)`, so two adapters wrong in the same way
//! cannot pass. The restart case builds a second `EbpfDataplane` over the same
//! pin directory (`new_with_pin_dir` recreates the maps): every frontend reads
//! `None`, and re-programming reads the fingerprint again. This is the Tier-3
//! pairing for any Sim read in a conformance driver.
//!
//! The kernel observation is also made directly, without the port: after an
//! update the test opens the run's pinned `SERVICE_MAP`, looks up the frontend
//! to get the inner map id, opens that map with `BPF_MAP_GET_FD_BY_ID`, and
//! asserts that inner slots 16,381 and 16,382 hold the low and high words of
//! the fingerprint and that a Maglev slot still holds a backend id present in
//! `BACKEND_MAP`. A read made after the update returns cannot tell whether the
//! fingerprint was written before or after the outer swap; that ordering is a
//! mutation obligation the apply-commit guard makes unobservable here. The IPv6
//! refusal is the real `BackendEntryPod` conversion.
//!
//! A failed host read (`ENOENT` on the inner map id) has no safe injection
//! point on a real kernel and is not reachable on the appliance; the Sim side
//! of that clause is covered in
//! `crates/overdrive-sim/tests/sim_dataplane_programmed_fingerprint_contract.rs`.
//!
//! Both reads are implemented: the tests run as written.
//!
//! CONTRACT_SHAPE: pure-function (the read mutates nothing).

#![cfg(target_os = "linux")]
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::too_many_lines)]

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU16;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::path::Path;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::traits::ProgrammedServiceView;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_dataplane::EbpfDataplane;
use overdrive_dataplane::maps::wire::ServiceKey;
use overdrive_dataplane::sys::bpf::{bpf_map_lookup_elem, bpf_obj_get};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_testing::overdrive_test;

use super::helpers::gateway_connect_rig::{HostRig, backend, frontend};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

/// The same frontend on another protocol: an independent key on the same VIP.
fn on_udp(frontend: ServiceFrontend) -> ServiceFrontend {
    ServiceFrontend::new(
        frontend.vip(),
        NonZeroU16::new(frontend.port().get()).expect("non-zero port"),
        Proto::Udp,
    )
    .expect("IPv4 frontend")
}

/// First inner slot that carries the fingerprint (the Maglev table has 16,381).
const FINGERPRINT_LOW_SLOT: u32 = 16_381;
const FINGERPRINT_HIGH_SLOT: u32 = 16_382;
/// `bpf(2)` command `BPF_MAP_GET_FD_BY_ID`.
const BPF_MAP_GET_FD_BY_ID: libc::c_long = 14;

/// The inner map of `fe`'s outer `SERVICE_MAP` slot, opened by id.
fn inner_map_of(pin_dir: &Path, fe: &ServiceFrontend) -> OwnedFd {
    let outer = bpf_obj_get(&pin_dir.join("SERVICE_MAP")).expect("open the pinned SERVICE_MAP");
    let key = ServiceKey {
        vip_host: u32::from(fe.vip_v4()),
        port_host: fe.port().get(),
        proto: fe.proto().as_u8(),
        _pad: 0,
    };
    // SAFETY: `ServiceKey` is `repr(C)`, 8 bytes, with the pad byte zeroed.
    let key_bytes = unsafe {
        core::slice::from_raw_parts(
            (&raw const key).cast::<u8>(),
            core::mem::size_of::<ServiceKey>(),
        )
    };
    let id_bytes = bpf_map_lookup_elem(outer.as_fd(), key_bytes, 4)
        .expect("outer SERVICE_MAP lookup")
        .expect("the frontend has an outer SERVICE_MAP slot");
    let inner_id = u32::from_ne_bytes(id_bytes.try_into().expect("an outer value is a 4-byte id"));
    let mut attr = [0u8; 128];
    attr[0..4].copy_from_slice(&inner_id.to_ne_bytes());
    // SAFETY: `attr` is a zeroed 128-byte `bpf_attr` with `map_id` set.
    let fd =
        unsafe { libc::syscall(libc::SYS_bpf, BPF_MAP_GET_FD_BY_ID, attr.as_mut_ptr(), 128usize) };
    assert!(fd >= 0, "BPF_MAP_GET_FD_BY_ID({inner_id}): {}", std::io::Error::last_os_error());
    // SAFETY: the kernel returned a new file descriptor that nothing else owns.
    unsafe { OwnedFd::from_raw_fd(i32::try_from(fd).expect("a descriptor fits in i32")) }
}

fn inner_slot(inner: &OwnedFd, slot: u32, what: &str) -> u32 {
    let bytes = bpf_map_lookup_elem(inner.as_fd(), &slot.to_ne_bytes(), 4)
        .unwrap_or_else(|e| panic!("inner slot {slot} ({what}): {e}"))
        .unwrap_or_else(|| panic!("inner slot {slot} ({what}) is absent"));
    u32::from_ne_bytes(bytes.try_into().expect("a slot value is 4 bytes"))
}

/// Read the kernel's `SERVICE_MAP` directly: the inner map of `fe` carries
/// `expected` as its low and high words, and slot 0 still names a backend that
/// `BACKEND_MAP` holds.
fn assert_raw_slots_carry(
    rig: &HostRig,
    fe: &ServiceFrontend,
    expected: BackendSetFingerprint,
    step: &str,
) {
    let inner = inner_map_of(&rig.pins.0, fe);
    let low = inner_slot(&inner, FINGERPRINT_LOW_SLOT, "fingerprint low word");
    let high = inner_slot(&inner, FINGERPRINT_HIGH_SLOT, "fingerprint high word");
    let want_low = u32::try_from(expected & u64::from(u32::MAX)).expect("masked to 32 bits");
    let want_high =
        u32::try_from(expected >> 32).expect("a u64 shifted right by 32 fits in 32 bits");
    assert_eq!(low, want_low, "raw inner slot {FINGERPRINT_LOW_SLOT} after {step}");
    assert_eq!(high, want_high, "raw inner slot {FINGERPRINT_HIGH_SLOT} after {step}");
    let maglev = inner_slot(&inner, 0, "a Maglev slot");
    let owned: Vec<u32> = rig
        .dataplane
        .backend_map_entries()
        .expect("BACKEND_MAP entries")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        owned.contains(&maglev),
        "Maglev slot 0 holds {maglev}, a backend id BACKEND_MAP does not hold ({owned:?}) after {step}"
    );
}

/// Assert both adapters read `expected` for `fe`, naming `step` in the failure.
async fn assert_both_read(
    host: &EbpfDataplane,
    sim: &SimDataplane,
    fe: &ServiceFrontend,
    expected: Option<BackendSetFingerprint>,
    step: &str,
) {
    let from_host = host.programmed_fingerprint(fe).await.expect("host read");
    assert_eq!(from_host, expected, "host programmed_fingerprint after {step}");
    let from_sim = sim.programmed_fingerprint(fe).await.expect("sim read");
    assert_eq!(from_sim, expected, "sim programmed_fingerprint after {step}");
}

/// Apply `set` to both adapters and require the same outcome.
async fn apply_to_both(
    host: &EbpfDataplane,
    sim: &SimDataplane,
    fe: &ServiceFrontend,
    set: &[Backend],
) -> Result<(), ()> {
    let from_host = host.update_service(*fe, set.to_vec()).await.is_ok();
    let from_sim = sim.update_service(*fe, set.to_vec()).await.is_ok();
    assert_eq!(from_host, from_sim, "host and sim agree on whether the update is accepted");
    if from_host { Ok(()) } else { Err(()) }
}

/// One call sequence: apply `B1`, apply `B2`, a weight-only change, withdraw,
/// apply on a second protocol of the same VIP, and a mixed IPv4/IPv6 set that
/// is refused. After each step host and sim read the same value, equal to the
/// fingerprint of the applied slice.
#[overdrive_test(guest = "mtls-relay")]
#[test]
fn host_and_sim_read_the_same_programmed_fingerprint_after_every_call() {
    let rig = HostRig::new("pf1");
    let host = &rig.dataplane;
    let sim = SimDataplane::new();
    let fe = frontend(1);
    let vip = fe.vip();

    runtime().block_on(async {
        // Never programmed.
        assert_both_read(host, &sim, &fe, None, "construction").await;

        let b1 = vec![backend(1, 1, true)];
        apply_to_both(host, &sim, &fe, &b1).await.expect("apply B1");
        assert_both_read(host, &sim, &fe, Some(fingerprint(&vip, &b1)), "applying B1").await;

        let b2 = vec![backend(1, 1, true), backend(2, 2, true)];
        apply_to_both(host, &sim, &fe, &b2).await.expect("apply B2");
        assert_both_read(host, &sim, &fe, Some(fingerprint(&vip, &b2)), "applying B2").await;
        assert_raw_slots_carry(&rig, &fe, fingerprint(&vip, &b2), "applying B2");

        // The same backends in a non-canonical order are a different applied
        // slice: the fingerprint is computed in the order given, so an
        // implementation that sorts before fingerprinting fails here.
        let b2_reversed: Vec<Backend> = b2.iter().rev().cloned().collect();
        let reversed_fp = fingerprint(&vip, &b2_reversed);
        assert_ne!(reversed_fp, fingerprint(&vip, &b2), "the order of the slice is covered");
        apply_to_both(host, &sim, &fe, &b2_reversed).await.expect("reordered B2");
        assert_both_read(host, &sim, &fe, Some(reversed_fp), "a reordered B2").await;
        assert_raw_slots_carry(&rig, &fe, reversed_fp, "a reordered B2");
        apply_to_both(host, &sim, &fe, &b2).await.expect("back to B2");

        let mut reweighted = b2.clone();
        reweighted[1].weight = 7;
        apply_to_both(host, &sim, &fe, &reweighted).await.expect("weight-only change");
        let reweighted_fp = fingerprint(&vip, &reweighted);
        assert_ne!(reweighted_fp, fingerprint(&vip, &b2), "a weight change is a different set");
        assert_both_read(host, &sim, &fe, Some(reweighted_fp), "a weight-only change").await;

        apply_to_both(host, &sim, &fe, &[]).await.expect("withdraw");
        assert_both_read(host, &sim, &fe, None, "the withdrawal").await;

        // A second protocol of the same VIP is an independent key.
        let udp = on_udp(fe);
        let udp_set = vec![backend(3, 3, true)];
        apply_to_both(host, &sim, &udp, &udp_set).await.expect("apply on udp");
        assert_both_read(host, &sim, &udp, Some(fingerprint(&vip, &udp_set)), "applying on udp")
            .await;
        assert_both_read(host, &sim, &fe, None, "applying on udp (the tcp key)").await;

        // A mixed IPv4/IPv6 set is refused before any write: the read is unchanged.
        let mut mixed = udp_set.clone();
        mixed.push(Backend {
            alloc: backend(4, 4, true).alloc,
            addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 9000),
            weight: 1,
            healthy: true,
        });
        assert!(
            apply_to_both(host, &sim, &udp, &mixed).await.is_err(),
            "a set with an IPv6 backend is refused by both adapters"
        );
        assert_both_read(host, &sim, &udp, Some(fingerprint(&vip, &udp_set)), "a refused IPv6 set")
            .await;
    });
}

/// The restart case: a second `EbpfDataplane` over the same pin directory
/// recreates the maps, so every frontend reads `None`; re-programming reads the
/// fingerprint again. A freshly constructed `SimDataplane` is the sim's restart.
#[overdrive_test(guest = "mtls-relay")]
#[test]
fn a_restarted_host_dataplane_reads_none_for_every_frontend_then_reprograms() {
    let rig = HostRig::new("pf2");
    let fe_one = frontend(1);
    let fe_two = frontend(2);
    let vip_one = fe_one.vip();
    let set_one = vec![backend(1, 1, true)];
    let set_two = vec![backend(2, 2, true), backend(3, 3, true)];

    let runtime = runtime();
    let sim = SimDataplane::new();
    runtime.block_on(async {
        for (fe, set) in [(&fe_one, &set_one), (&fe_two, &set_two)] {
            apply_to_both(&rig.dataplane, &sim, fe, set).await.expect("program");
        }
        assert_both_read(
            &rig.dataplane,
            &sim,
            &fe_one,
            Some(fingerprint(&vip_one, &set_one)),
            "programming before the restart",
        )
        .await;
    });

    // Serve restarts: the maps are recreated empty over the same pin directory.
    let rig = rig.recreate();
    let sim = SimDataplane::new();
    runtime.block_on(async {
        for fe in [&fe_one, &fe_two] {
            assert_both_read(&rig.dataplane, &sim, fe, None, "the restart").await;
        }
        apply_to_both(&rig.dataplane, &sim, &fe_one, &set_one).await.expect("re-program");
        assert_both_read(
            &rig.dataplane,
            &sim,
            &fe_one,
            Some(fingerprint(&vip_one, &set_one)),
            "re-programming after the restart",
        )
        .await;
        assert_both_read(&rig.dataplane, &sim, &fe_two, None, "re-programming one frontend").await;
    });
}
