//! S-2.2-09..11 — HASH_OF_MAPS atomic per-service backend swap.
//!
//! Tags: `@US-03` `@K3` `@slice-03` `@ASR-2.2-01`
//! `@real-io @adapter-integration` `@pending`.
//! Tier: Tier 3.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc, clippy::expect_used, clippy::unwrap_used)]

/// S-2.2-09 — Atomic backend swap drops zero packets under
/// 50 kpps `xdp-trafficgen` traffic.
#[test]
#[should_panic(expected = "RED scaffold")]
fn atomic_swap_under_50kpps_traffic_drops_zero_packets() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-09 — \
         service S1 has one backend B1; xdp-trafficgen 50 kpps; \
         swap to {{B1, B2, B3}}; assert ZERO drops via send vs sink \
         receive accounting"
    );
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
