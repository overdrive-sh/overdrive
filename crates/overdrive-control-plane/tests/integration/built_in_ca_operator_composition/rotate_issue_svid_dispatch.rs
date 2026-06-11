//! Integration — the rotate-correlation `Action::IssueSvid` dispatches through
//! the EXISTING action-shim executor per `built-in-ca-operator-composition`
//! Slice ① (folds GH #40). DELIVER (step 02-03) — scaffold ACTIVATED.
//!
//! Layer 3 (real `RcgenCa` + real `LocalObservationStore`; the `IssueSvid`
//! action-shim executor `dispatch_issue` is the driving port). Per Mandate 11
//! example-only, one example; no PBT machinery.
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
//! Direct executor-dispatch coverage (NOT a Phase-01 reconcile-branch flip, NO
//! Phase-01 dependency): the rotate-correlation `IssueSvid` is CONSTRUCTED in
//! the test (the same shape `SvidLifecycle`'s near-expiry branch emits) and
//! dispatched through `dispatch_issue` UNCHANGED. The executor (`issue_svid`) is
//! reused wholesale — the assertions are on the audit row + held snapshot, never
//! executor internals.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::action_shim::issue_svid::dispatch_issue;
use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, ContentHash, CorrelationKey, NodeId, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

const WORKLOAD_NAME: &str = "ws-rotate";
const NODE_NAME: &str = "host-0";
const ALLOC_NAME: &str = "alloc-rotate-0";

/// Trust-domain subject the test CA is minted for.
fn trust_domain_subject() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/overdrive/ca").expect("trust-domain SpiffeId parses")
}

/// A host `RcgenCa` over real OS entropy — the `Ca` port the executor dispatches
/// through (real P-256 crypto).
fn host_ca() -> RcgenCa {
    RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject())
}

/// A real `LocalObservationStore` over redb at `obs.redb` — the production
/// `ObservationStore` the executor writes the `issued_certificates` audit row
/// through. Returned as `Arc<dyn ObservationStore>` so the executor receives the
/// PORT.
fn audit_store(dir: &TempDir) -> Arc<dyn ObservationStore> {
    Arc::new(
        LocalObservationStore::open(dir.path().join("obs.redb"))
            .expect("LocalObservationStore::open"),
    )
}

/// A fixed-time `Clock` test double — `unix_now` returns a constant so the audit
/// row's validity window is deterministic. Mirrors `ca_boot_and_audit::FixedClock`.
struct FixedClock {
    unix: Duration,
    monotonic: Instant,
}

impl FixedClock {
    fn at_unix_secs(secs: u64) -> Self {
        Self { unix: Duration::from_secs(secs), monotonic: Instant::now() }
    }
}

#[async_trait]
impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.monotonic
    }
    fn unix_now(&self) -> Duration {
        self.unix
    }
    async fn sleep(&self, _duration: Duration) {}
}

/// Build the rotate-correlation `Action::IssueSvid` — the SAME shape the
/// `SvidLifecycle` near-expiry branch emits (ADR-0067 D2): `"rotate-svid"`
/// purpose, the HELD identity, the running node. NO new variant / field / flag.
fn rotate_issue_svid_action(alloc: &AllocationId, identity: &SpiffeId, node: &NodeId) -> Action {
    // `identity_correlation` shape from `svid_lifecycle.rs`: target =
    // "svid-lifecycle/<alloc>", spec_hash = ContentHash::of(<spiffe uri>),
    // purpose = "rotate-svid".
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(identity.as_str().as_bytes());
    let correlation = CorrelationKey::derive(&target, &spec_hash, "rotate-svid");
    Action::IssueSvid {
        alloc_id: alloc.clone(),
        spiffe_id: identity.clone(),
        node_id: node.clone(),
        correlation,
    }
}

// S-OC-10 `@integration @real-io @adapter-integration @driving_port @slice-1` —
// an `Action::IssueSvid` carrying a `"rotate-svid"` correlation for a HELD
// running allocation, dispatched through the action shim against a real CA
// adapter (whose `issue_and_audit` mints a fresh leaf + writes an audit row):
// a FRESH `issued_certificates` row (NEW serial, NEW window) is observable;
// `IdentityMgr` holds the freshly-minted `SvidMaterial` for the allocation
// (hold-REPLACE, not a second hold); the held cert serial matches the new
// audit-row serial; and the fresh row's validity window advanced past
// first-issue's and matches the held material. The "NEW window" half is made
// observable by advancing the injected clock by 1h between first-issue and
// rotate — a single fixed instant would re-derive a byte-identical window and
// leave that sub-claim vacuous. Universe: the action-shim result, the
// `IdentityMgr` held snapshot (post-replace), the ObservationStore audit row
// (serial + validity window).
#[tokio::test]
async fn rotate_correlation_issue_svid_mints_replaces_hold_and_audits() {
    // GIVEN a real CA + a real observation store + a fixed clock + an
    // IdentityMgr that ALREADY HOLDS a prior leaf for the allocation (the
    // pre-rotation state — rotation hold-REPLACES this).
    let obs_dir = TempDir::new().expect("obs-store tempdir");
    let ca = host_ca();
    let obs = audit_store(&obs_dir);
    // First issue at T0; rotation at T0 + 1h. Advancing the clock BETWEEN the
    // two dispatches is what makes the rotated leaf carry a genuinely NEW
    // validity window: the issuance seam derives `not_before` / `not_after`
    // from the injected clock (`ca_issuance.rs` — `not_before = now −
    // SKEW_TOLERANCE`, `not_after = not_before + TTL`), so a single fixed
    // instant would re-derive a window byte-identical to first-issue's and
    // S-OC-10's "new window" claim would be vacuous. Two clocks make the
    // window shift observable.
    let t0_secs: u64 = 1_700_000_005;
    let rotate_delta_secs: u64 = 3_600;
    let clock_first = FixedClock::at_unix_secs(t0_secs);
    let clock_rotate = FixedClock::at_unix_secs(t0_secs + rotate_delta_secs);
    let identity_mgr = IdentityMgr::new(None);

    let workload = WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId");
    let alloc = AllocationId::new(ALLOC_NAME).expect("valid AllocationId");
    let node = NodeId::new(NODE_NAME).expect("valid NodeId");
    let identity = SpiffeId::for_allocation(&workload, &alloc);

    // Seed the pre-rotation hold via a first-issue dispatch (the `"issue-svid"`
    // branch) so rotation has a PRIOR entry to replace. Capture its serial.
    let first_issue = Action::IssueSvid {
        alloc_id: alloc.clone(),
        spiffe_id: identity.clone(),
        node_id: node.clone(),
        correlation: {
            let target = format!("svid-lifecycle/{alloc}");
            let spec_hash = ContentHash::of(identity.as_str().as_bytes());
            CorrelationKey::derive(&target, &spec_hash, "issue-svid")
        },
    };
    dispatch_issue(&first_issue, &ca, obs.as_ref(), &clock_first, &identity_mgr)
        .await
        .expect("first-issue dispatch seeds the prior hold");
    let prior_serial = identity_mgr
        .svid_for(&alloc)
        .expect("the prior leaf is held after first-issue")
        .serial()
        .clone();
    // Capture the FIRST-ISSUE validity window from its audit row — the baseline
    // the rotated window must advance past (S-OC-10 "new window").
    let prior_rows = obs.issued_certificate_rows().await.expect("read audit rows");
    let prior_row = prior_rows
        .iter()
        .find(|r| r.serial == prior_serial && r.spiffe_id == identity)
        .expect("the first-issue audit row is present");
    let prior_not_before = prior_row.not_before;
    let prior_not_after = prior_row.not_after;
    let audit_count_before = prior_rows.len();

    // WHEN a rotate-correlation IssueSvid for the SAME held identity dispatches
    // through the EXISTING executor — under an ADVANCED clock (T0 + 1h), so the
    // freshly-minted leaf is signed with a NEW validity window.
    let rotate = rotate_issue_svid_action(&alloc, &identity, &node);
    dispatch_issue(&rotate, &ca, obs.as_ref(), &clock_rotate, &identity_mgr)
        .await
        .expect("rotate-correlation IssueSvid dispatches through the existing executor");

    // THEN a FRESH issued_certificates audit row was written (count strictly
    // increased) — the rotate minted a new leaf, it did not re-read a cached one.
    let rows = obs.issued_certificate_rows().await.expect("read audit rows");
    assert!(
        rows.len() > audit_count_before,
        "rotation must write a FRESH issued_certificates row ({audit_count_before} → {})",
        rows.len()
    );

    // AND IdentityMgr holds the FRESHLY-MINTED material for the alloc — a
    // hold-REPLACE (one held entry, new serial), not a second hold. The held
    // serial is DISTINCT from the prior leaf's.
    let held = identity_mgr.svid_for(&alloc).expect("the alloc is still held after rotation");
    assert_eq!(
        held.spiffe_id(),
        &identity,
        "the held identity is unchanged across rotation (same SpiffeId, fresh leaf)"
    );
    assert_ne!(
        held.serial(),
        &prior_serial,
        "rotation hold-REPLACES with a FRESH leaf — the held serial must differ from the prior"
    );
    assert_eq!(
        identity_mgr.held_snapshot().len(),
        1,
        "rotation hold-REPLACES (one held entry for the alloc), not a second hold"
    );

    // AND the held serial MATCHES the new audit-row serial — the row the rotate
    // wrote is the row for the leaf now held (audit-before-hold, same cert).
    let new_serial = held.serial();
    let new_row = rows
        .iter()
        .find(|r| &r.serial == new_serial && r.spiffe_id == identity)
        .unwrap_or_else(|| {
            panic!(
                "the held leaf's serial must match a fresh issued_certificates row's serial for \
                 the identity; held serial {new_serial}, rows: {:?}",
                rows.iter().map(|r| r.serial.to_string()).collect::<Vec<_>>()
            )
        });

    // AND that fresh row carries a genuinely NEW validity window — both bounds
    // advanced past first-issue's. The issuance seam derives the window from the
    // injected clock, so the +1h advance shifts `not_before` and `not_after`
    // forward; a re-read of a cached leaf would carry the PRIOR window. This is
    // the part of S-OC-10 ("new window") a single fixed clock could not show.
    assert!(
        new_row.not_before > prior_not_before,
        "rotation under an advanced clock must shift not_before forward ({prior_not_before} → {})",
        new_row.not_before
    );
    assert!(
        new_row.not_after > prior_not_after,
        "rotation under an advanced clock must shift not_after forward ({prior_not_after} → {})",
        new_row.not_after
    );

    // AND the fresh window MATCHES the held material — `SvidMaterial::not_after`
    // equals the audit row's `not_after` by construction (`ca_issuance.rs`
    // threads ONE window into both the signed leaf and the row), so the row the
    // rotate wrote is the row for the leaf now held, in its NEW window.
    assert_eq!(
        new_row.not_after,
        held.not_after(),
        "the fresh audit row's not_after must match the held leaf's not_after (same minted window)"
    );
}
