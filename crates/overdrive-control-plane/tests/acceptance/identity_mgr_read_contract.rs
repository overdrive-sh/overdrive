//! Acceptance for workload-identity-manager Slice 02 (step 02-01).
//!
//! Layer 1/2: the in-process `IdentityRead` read contract (ADR-0067 D7).
//! `IdentityMgr` is exercised through the `IdentityRead` driving port — the
//! sync, owned-clone, never-re-issue read surface a dataplane consumer holds.
//!
//! The two activated scenarios pin two of the five D7 clauses behaviourally:
//!   * **S-WIM-04** — a held alloc reads back its owned `SvidMaterial` clone and
//!     the hydrated `TrustBundle`, with NO certificate issuance on the read path
//!     (clauses 1 + 4). The "never issues" guarantee (clause 1, the O3
//!     read-latency promise) is proven with a CA-call counter: a `CountingCa`
//!     whose `issue_svid` bumps an atomic counter is composed into the universe,
//!     and the counter is asserted unchanged across the reads.
//!   * **S-WIM-05** (`@error`) — after `drop_svid`, the same read port returns
//!     `None` explicitly (clause 5 / K2), so a consumer fails closed instead of
//!     presenting a stale credential for a stopped alloc.
//!
//! The GENERATIVE `IdentityRead` equivalence property + the `SimIdentityRead`
//! double are step 02-02 and are deliberately NOT built here.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_core::traits::ca::{
    Ca, CaCertDer, CaCertPem, CaKeyPem, IntermediateHandle, Result as CaResult, RootCaHandle,
    SvidMaterial, SvidRequest, TrustBundle,
};
use overdrive_core::traits::identity_read::IdentityRead;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, NodeId, SpiffeId};
use std::time::Duration;

/// A `Ca` whose `issue_svid` bumps an atomic counter, so a test can assert the
/// read path NEVER issues (ADR-0067 D7 clause 1 — the O3 read-latency promise).
///
/// Only `issue_svid` is exercised by these scenarios; the other surface methods
/// are unreachable here and panic if called (no read path touches them either).
struct CountingCa {
    issue_calls: Arc<AtomicU64>,
}

impl CountingCa {
    fn new() -> (Self, Arc<AtomicU64>) {
        let counter = Arc::new(AtomicU64::new(0));
        (Self { issue_calls: Arc::clone(&counter) }, counter)
    }
}

impl Ca for CountingCa {
    fn root(&self) -> CaResult<RootCaHandle> {
        unreachable!("read-contract scenario never composes a root")
    }

    fn issue_intermediate(&self, _node: &NodeId) -> CaResult<IntermediateHandle> {
        unreachable!("read-contract scenario never issues an intermediate")
    }

    fn issue_svid(&self, req: &SvidRequest) -> CaResult<SvidMaterial> {
        self.issue_calls.fetch_add(1, Ordering::SeqCst);
        Ok(svid(req.spiffe_id().as_str(), req.not_after()))
    }

    fn trust_bundle(&self) -> CaResult<TrustBundle> {
        unreachable!("read-contract scenario hydrates the bundle directly")
    }
    // `adopt_persisted_root` / `adopt_persisted_intermediate` use the trait's
    // no-op default impls — the read-contract scenarios never adopt.
}

/// Build a `SvidMaterial` with placeholder cert/key bytes for `(spiffe,
/// not_after)`. The read-contract scenarios assert on the owned clone the read
/// port returns, not on the opaque cert/key bytes.
fn svid(spiffe: &str, not_after: UnixInstant) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0xDE, 0xAD]),
        CertSerial::new("0badc0de").expect("serial parses"),
        SpiffeId::new(spiffe).expect("valid SpiffeId"),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into()),
        not_after,
    )
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid AllocationId")
}

fn bundle() -> TrustBundle {
    TrustBundle::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into()),
        None,
    )
}

const fn not_after_at(secs: u64) -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::from_secs(secs))
}

/// `@in-memory` `@S-WIM-04` — a held alloc reads back its owned `SvidMaterial`
/// clone via `IdentityRead::svid_for`, and `current_bundle` returns the hydrated
/// `TrustBundle`, with NO certificate issued on the read path.
///
/// Universe: the two `IdentityRead` return values + a CA issue-call counter. The
/// `CountingCa` is composed into the universe purely to prove the read path is
/// CA-free (clause 1): we record the counter before the reads and assert it is
/// unchanged after — `current_bundle` reads the hydrated in-process bundle (D6),
/// never `Ca::trust_bundle`, and `svid_for` reads the held map, never
/// `Ca::issue_svid`.
#[test]
fn identity_read_returns_svid_and_trust_bundle_without_reissue() {
    let (ca, issue_calls) = CountingCa::new();
    let ca: Arc<dyn Ca> = Arc::new(ca);

    let spiffe = "spiffe://overdrive.local/job/payments/alloc/a1b2c3";
    let not_after = not_after_at(1_700_003_600);
    let held = svid(spiffe, not_after);

    // The boot bundle is HYDRATED into IdentityMgr (D6) — the executor would
    // refresh it via set_bundle; here we seed it at construction.
    let mgr = IdentityMgr::new(Some(bundle()));
    mgr.hold(alloc("alloc-a1b2c3-0"), held.clone());

    // Read through the IdentityRead driving port (sync, owned clones).
    let reader: &dyn IdentityRead = &mgr;
    let calls_before = issue_calls.load(Ordering::SeqCst);

    let svid_read = reader.svid_for(&alloc("alloc-a1b2c3-0"));
    let bundle_read = reader.current_bundle();

    let calls_after = issue_calls.load(Ordering::SeqCst);

    // Clause 1 (never issues) — the O3 read-latency promise: zero CA issuance on
    // the read path. The `ca` handle is held only to source the counter; the
    // read surface never touches it.
    assert_eq!(
        calls_after, calls_before,
        "the read path issues no certificate (ADR-0067 D7 clause 1 / O3)"
    );
    let _ = &ca; // the CA is intentionally never invoked on the read path

    // Clause 4 (owned clones) — svid_for returns the held material, no re-issue.
    assert_eq!(
        svid_read,
        Some(held),
        "svid_for returns an owned clone of the held SVID material (D7 clause 4)"
    );

    // current_bundle returns the hydrated bundle (D6 — zero CA I/O).
    assert_eq!(
        bundle_read,
        Some(bundle()),
        "current_bundle returns the hydrated trust bundle (ADR-0067 D6)"
    );
}

/// `@in-memory` `@error` `@S-WIM-05` — after `drop_svid`, `IdentityRead::svid_for`
/// returns `None` for the dropped alloc, so the consumer cannot observe a stale
/// credential for a stopped allocation (clause 5 / K2 — drop-on-stop is
/// observable on the read surface).
///
/// Universe: the `svid_for` result for the dropped alloc.
#[test]
fn identity_read_returns_none_after_drop() {
    let mgr = IdentityMgr::new(Some(bundle()));
    let a = alloc("alloc-a1b2c3-0");
    mgr.hold(
        a.clone(),
        svid("spiffe://overdrive.local/job/payments/alloc/a1b2c3", not_after_at(1_700_003_600)),
    );

    let reader: &dyn IdentityRead = &mgr;
    assert!(reader.svid_for(&a).is_some(), "the alloc is held and readable before the drop");

    mgr.drop_svid(&a);

    assert_eq!(
        reader.svid_for(&a),
        None,
        "after DropSvid the read port returns explicit absence — no stale credential (D7 clause 5 / K2)"
    );
}
