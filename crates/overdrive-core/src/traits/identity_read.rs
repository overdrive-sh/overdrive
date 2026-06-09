//! [`IdentityRead`] — the in-process held-identity read port (ADR-0067 D7).
//!
//! A dataplane consumer (the sockops/kTLS mTLS layer #26, the gateway, the
//! telemetry path) reads the current SVID for an allocation and the current
//! trust bundle through this port — **sync, in-process, owned-clone, never
//! re-issuing**. The whole point is O3 (whitepaper §7 "no gRPC, no IPC"): the
//! read hot path touches no CA and no network. The SVID is served from the
//! held set `IdentityMgr` (ADR-0067 D4) holds; the bundle is served from the
//! HYDRATED bundle `IdentityMgr` holds (ADR-0067 D6 — set at boot, refreshed by
//! the issue executor, pushed by the #40 rotation via `set_bundle`), never
//! pulled from the CA per read.
//!
//! Per ADR-0063 / `.claude/rules/development.md` § "Port-trait dependencies",
//! consumers take `Arc<dyn IdentityRead>` as a **required constructor
//! parameter** — never defaulted to a production binding. The host adapter is
//! `IdentityMgr` (`overdrive-control-plane`); the `SimIdentityRead` double
//! (`overdrive-sim`) and the `identity_read_equivalence` DST test that drives
//! both adapters through the same call sequence are step 02-02.

use crate::AllocationId;
use crate::traits::ca::{SvidMaterial, TrustBundle};

/// The in-process held-identity read port (ADR-0067 D7).
///
/// Two sync, owned-clone getters: [`svid_for`](IdentityRead::svid_for) and
/// [`current_bundle`](IdentityRead::current_bundle). These rustdoc blocks are
/// the SSOT the `identity_read_equivalence` DST test (step 02-02) enforces
/// against every adapter (`.claude/rules/development.md` § "Trait definitions
/// specify behavior, not just signature"). Five observable clauses every
/// adapter MUST honor:
///
/// 1. **A read never issues.** Neither getter calls `Ca::issue_svid` (nor any
///    CA method) — the SVID is served from the held set and the bundle from the
///    hydrated bundle. This is the O3 read-latency guarantee (whitepaper §7);
///    an adapter that issued (or pulled the bundle from the CA) on a read would
///    violate it.
/// 2. **A read never mutates.** Neither getter mutates the held set or the
///    bundle as a side effect of being read.
/// 3. **`None` is explicit absence** — not an error, not an empty-but-present
///    credential. A consumer reading an absent allocation refuses the handshake
///    rather than presenting a stale credential.
/// 4. **Returns owned clones.** The caller holds no lock after the read — the
///    read-lock is dropped within the read expression (the host adapter clones
///    out and drops the guard before returning, ADR-0067 D4 / A7).
/// 5. **Post-drop absence is observable.** After the held entry for `alloc` is
///    dropped (the `DropSvid` executor calls `IdentityMgr::drop_svid`),
///    `svid_for(alloc)` returns `None` — drop-on-stop is observable through the
///    read surface (ADR-0067 O2 / K2 — leak resistance on stop).
pub trait IdentityRead: Send + Sync {
    /// The current SVID material held for `alloc`, or `None` when no SVID is
    /// held for it.
    ///
    /// # Preconditions
    /// None. Any [`AllocationId`] is a valid query — an allocation that was
    /// never issued an SVID, or whose SVID was dropped, reads as `None`.
    ///
    /// # Postconditions
    /// On `Some`, returns an **owned clone** of the [`SvidMaterial`] currently
    /// held for `alloc` (the cert + node-held leaf private key + validity end),
    /// with NO certificate issued or re-issued (clauses 1 + 4). The caller holds
    /// no lock after the call returns (clause 4). The held set is unchanged
    /// (clause 2).
    ///
    /// # Edge cases
    /// `None` is **explicit absence** (clause 3) — never an error, never an
    /// empty-but-present credential. A re-issue overwrites the held material, so
    /// a subsequent `svid_for` returns the FRESH material under the same
    /// `alloc`; a drop removes it, so a subsequent `svid_for` returns `None`
    /// (clause 5).
    ///
    /// # Observable invariants
    /// `svid_for(alloc)` reflects exactly the current held entry for `alloc`:
    /// `Some(material)` after a hold, `None` after a drop (clause 5). It never
    /// calls the CA (clause 1) and never mutates (clause 2).
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial>;

    /// The current hydrated trust bundle, or `None` when no bundle is held.
    ///
    /// # Preconditions
    /// None.
    ///
    /// # Postconditions
    /// Returns an **owned clone** of the [`TrustBundle`] held in-process
    /// (clause 4), served from the HYDRATED bundle (ADR-0067 D6) — set at boot,
    /// refreshed by the issue executor, pushed by the #40 rotation seam — with
    /// **zero CA I/O on the read path** (clause 1). The caller holds no lock
    /// after the call returns (clause 4); the bundle is unchanged (clause 2).
    ///
    /// # Edge cases
    /// `None` is **explicit absence** (clause 3) — no bundle has been installed
    /// (a CA that refused to start never seeds one; the identity layer inherits
    /// ADR-0063's `health.startup.refused` posture). It is not an error and not
    /// an empty-but-present bundle.
    ///
    /// # Observable invariants
    /// `current_bundle()` reflects the most recently installed bundle and never
    /// pulls from the CA (clause 1) — the read touches no network and no CA.
    fn current_bundle(&self) -> Option<TrustBundle>;
}
