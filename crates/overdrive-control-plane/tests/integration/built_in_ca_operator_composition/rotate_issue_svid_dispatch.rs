//! Integration — the rotate-correlation `Action::IssueSvid` dispatches through
//! the EXISTING action-shim executor per `built-in-ca-operator-composition`
//! Slice ① (folds GH #40). DISTILL RED scaffold.
//!
//! Layer 3 (real `RcgenCa` + real `LocalObservationStore`; the `IssueSvid`
//! action-shim executor is the driving port). Per Mandate 11 example-only, one
//! example; no PBT machinery.
//!
//! Settled design (feature-delta.md D-OC-1; ADR-0067 D3): the near-expiry
//! rotation branch emits `Action::IssueSvid` with a `"rotate-svid"` correlation
//! — the EXISTING variant, UNCHANGED (no new field/flag/variant; honors
//! CLAUDE.md "never invent API surface"). The rotate `IssueSvid` dispatches
//! through the SAME executor as first-issue and restart-reissue
//! (`action_shim/issue_svid.rs`): `issue_and_audit` mints a fresh leaf (distinct
//! serial, new validity window), writes the `issued_certificates` audit row, and
//! the holder `hold`-replaces the prior entry. This scenario proves the reuse —
//! there is NO new executor surface for rotation.
//!
//! RED scaffold convention: `#[ignore]` — the blocker is that Slice ① has not
//! yet produced a `"rotate-svid"`-correlation `IssueSvid` (the gated
//! `StartWorkflow` path is still live until Slice ① flips it) AND the executor
//! dispatch with real CA + real `ObservationStore` is Lima-gated. DELIVER removes
//! `#[ignore]` and lands real assertions.

#![allow(clippy::expect_used, clippy::unwrap_used)]

// S-OC-10 `@integration @real-io @adapter-integration @driving_port @slice-1` —
// an `Action::IssueSvid` carrying a `"rotate-svid"` correlation for a HELD
// running allocation, dispatched through the action shim against a real CA
// adapter (whose `issue_and_audit` mints a fresh leaf + writes an audit row):
// a FRESH `issued_certificates` row (NEW serial, NEW window) is observable;
// `IdentityMgr` holds the freshly-minted `SvidMaterial` for the allocation
// (hold-REPLACE, not a second hold); the held cert serial matches the new
// audit-row serial. Universe: the action-shim result, the `IdentityMgr` held
// snapshot (post-replace), the ObservationStore audit row.
#[test]
#[ignore = "blocked on Slice 1 — rotate-svid-correlation IssueSvid emit + executor dispatch (Lima; real CA + ObservationStore)"]
fn rotate_correlation_issue_svid_mints_replaces_hold_and_audits() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-10 / a rotate-svid-correlation IssueSvid \
         dispatches through the EXISTING executor: mints a fresh leaf, writes a new audit row, \
         and hold-replaces in IdentityMgr)"
    );
}
