//! US-02 Scenario 2.9 — `CgroupPath` round-trips bit-identical.
//!
//! @in-memory @property — proptest over arbitrary valid alloc-derived
//! paths. PORT-TO-PORT: enters via `CgroupPath::from_str`, asserts
//! `Display` -> `from_str` cycles to an equal value.
//!
//! Relocated from `overdrive-worker/tests/acceptance/` (review remediation,
//! microvm-driver-cloud-hypervisor step 01-01 F6): `CgroupPath` itself now
//! lives in `overdrive_core::cgroup` (ADR-0082 §D2, Amendment 2026-08-12,
//! gap 1, GH #42) — testing it only through the `overdrive_worker`
//! re-export left a scoped `-p overdrive-core` mutation run blind to this
//! roundtrip proof. Tests live beside the code they defend.

use std::str::FromStr;

use overdrive_core::cgroup::CgroupPath;
use proptest::prelude::*;

/// Strategy producing valid alloc IDs (DNS-1123-like: lowercase
/// alphanumerics, `-`, `_`, `.`, length 1..=64).
fn arb_alloc_id() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._-]{0,62}[a-z0-9]".prop_filter("non-empty", |s: &String| !s.is_empty())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn cgroup_path_roundtrips_for_every_valid_input(alloc_label in arb_alloc_id()) {
        let canonical = format!("overdrive.slice/workloads.slice/{alloc_label}.scope");
        let parsed = CgroupPath::from_str(&canonical)
            .expect("valid canonical cgroup path parses");
        let displayed = parsed.to_string();
        prop_assert_eq!(&displayed, &canonical);
        let reparsed = CgroupPath::from_str(&displayed)
            .expect("reparses via Display");
        prop_assert_eq!(parsed, reparsed);
    }
}
