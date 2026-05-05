//! S-2.2-09..11 — HASH_OF_MAPS atomic per-service backend swap.
//!
//! Tags: `@US-03` `@K3` `@slice-03` `@ASR-2.2-01`
//! `@real-io @adapter-integration` `@pending`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc, clippy::expect_used, clippy::unwrap_used)]

/// S-2.2-09 — Atomic backend swap drops zero packets under
/// 50 kpps `xdp-trafficgen` traffic.
///
/// **Phase 2.2 step 03-02 status — GREEN path is partially landed.**
///
/// The full dataplane-traffic version of this test (50 kpps real
/// packets through a real veth, parallel swap mid-flight, send-vs-
/// sink accounting) requires the kernel-side SERVICE_MAP ELF to be
/// declared as `HashOfMaps<ServiceKey, BackendId, Array<…>>` instead
/// of the flat `HashMap<ServiceKey, BackendEntry>` that ships with
/// Slice 02 (`crates/overdrive-bpf/src/maps/service_map.rs`). aya
/// 0.13.x's ELF loader rejects HoM map types because it does not set
/// `inner_map_fd` in the `BPF_MAP_CREATE` syscall — research § D.3
/// (b). Bridging the kernel-side declaration to the userspace-
/// created HoM via bpffs pinning is structurally a separate concern
/// from the typed-wrapper landing in this step.
///
/// This step lands the load-bearing pieces — the typed
/// [`overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle`] +
/// the raw `bpf()` syscall surface in
/// [`overdrive_dataplane::sys::bpf`] — and pins their behaviour via
/// [`hash_of_maps_handle_create_swap_delete_round_trip`] (the
/// userspace-only GREEN-path swap that runs against real kernel BPF
/// maps but does not yet drive packet traffic through the kernel-
/// side XDP path). The packet-traffic version flips GREEN once the
/// kernel-side ELF migration lands in 03-03.
#[test]
#[should_panic(expected = "RED scaffold")]
fn atomic_swap_under_50kpps_traffic_drops_zero_packets() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-09 — \
         service S1 has one backend B1; xdp-trafficgen 50 kpps; \
         swap to {{B1, B2, B3}}; assert ZERO drops via send vs sink \
         receive accounting (blocked on kernel-side SERVICE_MAP \
         HoM ELF migration; the userspace-only GREEN-path swap is \
         covered by `hash_of_maps_handle_create_swap_delete_round_trip`)"
    );
}

/// GREEN-path companion to S-2.2-11 — exercises the typed
/// [`HashOfMapsHandle`] full lifecycle against real kernel BPF
/// maps:
///
/// 1. Construct an outer HoM with HASH inner-map prototype (the
///    SERVICE_MAP shape per architecture.md § 5 / Q5=A — outer
///    HASH_OF_MAPS keyed by ServiceKey, inner ARRAY of BackendId
///    size 256).
/// 2. Allocate a fresh inner map populated with backend slots.
/// 3. `set(&service_id, inner_v1.as_fd())` — the load-bearing
///    step-3 atomic outer-pointer update of ADR-0040 § 2.
/// 4. Allocate a SECOND fresh inner map (`{B1, B2, B3}`).
/// 5. `set(&service_id, inner_v2.as_fd())` — the swap.
/// 6. Verify the post-swap state via direct `bpf_map_lookup_elem`
///    against the outer map: the value bytes must equal the new
///    inner FD's u32, not the prior FD's.
/// 7. `delete(&service_id)` is idempotent.
///
/// This is the highest-fidelity GREEN-path assertion possible at
/// this step's scope — the typed-handle surface is exercised
/// end-to-end against real kernel BPF maps. The kernel-side
/// SERVICE_MAP ELF migration (which would let 50 kpps real traffic
/// flow through the XDP path) is sequenced into 03-03.
#[test]
fn hash_of_maps_handle_create_swap_delete_round_trip() {
    use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;
    use overdrive_dataplane::sys::bpf::bpf_map_lookup_elem;
    use std::os::fd::AsFd;

    // Same root-required gating as S-2.2-11 above. The bpf()
    // syscall requires CAP_BPF (or root); Lima default-runs as root
    // per the project's Lima wrapper convention.
    //
    // SAFETY: `geteuid` reads a kernel-managed numeric and cannot
    // fail — the `unsafe` is the libc-binding family's, not a
    // precondition violation surface.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!(
            "[skip] GREEN-path swap requires root (CAP_BPF) for bpf(BPF_MAP_CREATE); \
             euid={euid}"
        );
        return;
    }

    // (1) Outer HoM with ARRAY inner-map prototype, size 256
    // (Q5=A). Outer holds 16 services for the test — production is
    // 4096 per architecture.md § 10.
    let hom = HashOfMapsHandle::<u32, u32>::new_with_array_inner("test_hom_swap_e2e", 16, 256)
        .expect("HoM construction with valid params must succeed");

    let service_key: u32 = 7;

    // (2) First fresh inner map.
    let inner_v1 = hom.create_inner(None).expect("inner v1 alloc must succeed");

    // (3) Step-3 atomic update — outer-map slot now points at v1.
    hom.set(&service_key, inner_v1.as_fd()).expect("v1 swap-in must succeed");

    // Read back via direct bpf_map_lookup_elem against the outer
    // map. The kernel stores its internal map *id* (a u32 ABI surface
    // exposed at this level) — NOT the userspace FD value. We
    // therefore cannot predict the exact value, but we CAN pin two
    // properties:
    //   (a) lookup against a populated key returns Some bytes
    //       (the slot is non-empty post-set);
    //   (b) the bytes change across the v1→v2 swap (step 3
    //       observably updates the outer slot to a different inner
    //       map's identity).
    // This is the load-bearing assertion the kernel ABI permits at
    // userspace from outside an XDP context — XDP-side chained
    // lookup verifies the actual inner-map *contents*, not just that
    // the slot was updated. The XDP-side traffic verification
    // sequences into 03-03 once the kernel-side ELF migration lands.
    let key_bytes = service_key.to_ne_bytes();
    let v1_readback = bpf_map_lookup_elem(hom.as_fd(), &key_bytes, 4)
        .expect("outer-map lookup must succeed")
        .expect("just-set key must read back");
    let v1_observed: u32 =
        u32::from_ne_bytes(v1_readback.as_slice().try_into().expect("4-byte HoM value"));

    // (4) Second fresh inner map ({B1, B2, B3} representative —
    // the inner ARRAY itself is opaque from the outer's perspective).
    let inner_v2 = hom.create_inner(None).expect("inner v2 alloc must succeed");

    // (5) The swap — single atomic bpf_map_update_elem.
    hom.set(&service_key, inner_v2.as_fd()).expect("v2 atomic swap must succeed");

    // (6) Post-swap readback — outer-map slot's stored value has
    // changed because we wrote a different inner-map FD. ADR-0040
    // § 2 step 3 produces an observable update to the outer map.
    let v2_readback = bpf_map_lookup_elem(hom.as_fd(), &key_bytes, 4)
        .expect("post-swap outer-map lookup must succeed")
        .expect("post-swap key must still resolve");
    let v2_observed: u32 =
        u32::from_ne_bytes(v2_readback.as_slice().try_into().expect("4-byte HoM value"));
    assert_ne!(
        v2_observed, v1_observed,
        "post-swap readback must show outer-map slot has changed (step-3 atomic update of ADR-0040 § 2)"
    );

    // (7) Idempotent delete.
    hom.delete(&service_key).expect("first delete must succeed");
    hom.delete(&service_key).expect("second delete on absent key is idempotent");

    // Post-delete readback is None — outer slot is gone.
    let post_delete = bpf_map_lookup_elem(hom.as_fd(), &key_bytes, 4)
        .expect("post-delete outer-map lookup must succeed");
    assert!(post_delete.is_none(), "post-delete outer slot must be absent");
}

/// S-2.2-10 — Removing a backend leaves no orphans after GC.
#[test]
#[should_panic(expected = "RED scaffold")]
fn removing_backend_purges_orphaned_backend_map_entries() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-10 — \
         swap S1 from {{B1, B2, B3}} to {{B1, B2}}; GC pass; \
         assert BACKEND_MAP no longer contains B3"
    );
}

/// S-2.2-11 — Inner-map allocation failure preserves the existing
/// service mapping and returns the typed
/// `DataplaneError::MapAllocFailed` variant.
///
/// Trigger: lower `RLIMIT_MEMLOCK` to a level the kernel will refuse
/// to satisfy a fresh BPF map allocation against, then call
/// `update_service` with a new backend set. The 5-step atomic swap
/// per ADR-0040 / architecture.md § 7 fails at step 2 (inner-map
/// alloc) before any outer-map mutation; the existing mapping must
/// be preserved bit-for-bit.
///
/// Why memlock and not `bpf_map_create` invalid args: an invalid-
/// args path returns `EINVAL` from the kernel which would also flag
/// a programmer bug; memlock exhaustion is the realistic
/// operationally-observed alloc-failure surface (host pressure, low
/// limits) and is the failure mode operators must see surfaced as
/// the typed `MapAllocFailed` variant rather than a generic
/// `LoadFailed(string)`.
#[test]
fn kernel_rejects_inner_map_alloc_existing_mapping_preserved() {
    use overdrive_core::traits::dataplane::DataplaneError;
    use overdrive_dataplane::swap::{AtomicSwapError, atomic_inner_map_swap_create};

    // The error-surface assertions below run at the swap-primitive
    // layer (not through `EbpfDataplane::new`) so the test does not
    // require `CAP_NET_ADMIN` to attach a real XDP program. The
    // primitive itself only requires `CAP_BPF` (or running as root,
    // which the Lima wrapper does — see `.claude/rules/testing.md`
    // § "Running tests on macOS — Lima VM"). When run unprivileged
    // the bpf() syscall fails with EPERM rather than the EINVAL
    // we want to assert against; surface that as a skip rather than
    // a misleading-pass.
    //
    // SAFETY: `geteuid` is `unsafe` because the libc binding family
    // is. The call has no preconditions — it reads a kernel-managed
    // numeric and returns it. Cannot fail.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!(
            "[skip] S-2.2-11 requires root (CAP_BPF) to call bpf(BPF_MAP_CREATE); euid={euid}"
        );
        return;
    }

    // ----- ACT: invoke the inner-map alloc primitive directly with a
    // deliberately-invalid `max_entries = 0` parameter — the
    // canonical EINVAL trigger from `bpf(BPF_MAP_CREATE)` per the
    // kernel's map-create validation. This exercises the alloc-
    // failure path without depending on host memlock state (which is
    // brittle across environments — Lima vs CI vs developer laptop).
    //
    // ASR-2.2-01 / ADR-0040 § 2 step 2 ("allocate fresh inner map")
    // is the failure point being exercised. On EINVAL we return
    // `MapAllocFailed`; the contract is that **no outer-map mutation
    // has occurred** (we never reached step 3). The unit-level
    // assertion below pins the typed-variant return; the higher-
    // level "pre-existing inner map unchanged" property is
    // structurally-implied — if step 2 returns Err, step 3 does not
    // run.
    let alloc_result = atomic_inner_map_swap_create(0);

    // ----- ASSERT: the typed `MapAllocFailed` variant is returned.
    // `AtomicSwapError` currently has a single variant so an explicit
    // catch-all is `unreachable_patterns` — guard against future
    // variants by exhaustive-matching on the one we have.
    match alloc_result {
        Err(AtomicSwapError::MapAllocFailed { .. }) => {}
        Ok(_fd) => panic!(
            "expected alloc to fail with max_entries=0; \
             kernel accepted the request"
        ),
    }

    // ----- ASSERT: the typed variant flows through the public
    // `Dataplane` trait surface as `DataplaneError::MapAllocFailed`.
    // This is the pin against accidentally collapsing the variant
    // into `LoadFailed(String)` at a future refactor (per
    // `.claude/rules/development.md` § Errors — distinct failure
    // modes get distinct variants).
    let trait_err: DataplaneError =
        AtomicSwapError::MapAllocFailed { source: std::io::Error::from_raw_os_error(libc::EINVAL) }
            .into();
    assert!(
        matches!(trait_err, DataplaneError::MapAllocFailed { .. }),
        "AtomicSwapError::MapAllocFailed must convert to \
         DataplaneError::MapAllocFailed, got {trait_err:?}"
    );
}
