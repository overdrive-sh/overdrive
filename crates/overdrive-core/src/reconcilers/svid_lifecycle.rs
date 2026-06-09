//! `svid-lifecycle` — the workload-SVID reconciler primitives (ADR-0067).
//!
//! This module is the home for the pure `SvidLifecycle` reconciler that
//! converges the in-process held-SVID set against the Running-allocation set
//! (`running ∧ ¬held → IssueSvid`, `running ∧ held(near-expiry) → IssueSvid`
//! rotate, `¬running ∧ held → DropSvid`; ADR-0067 D2, feature-delta D-OC-1).
//! The reconciler, its `State` projection, and its retry-memory `View` land in
//! step 01-04; this step (01-03) defines ONLY the projection the held set
//! yields into the reconciler's `actual`:
//!
//! [`HeldSvidFacts`] — the per-allocation *projection* of a held
//! [`SvidMaterial`](crate::traits::ca::SvidMaterial) the
//! `IdentityMgr::held_snapshot` surface returns. It carries the two facts the
//! reconciler's `running ∧ ¬held` and near-expiry decisions read — the
//! `spiffe_id` and the `not_after` validity end — and DELIBERATELY NOT the leaf
//! private key: the key never leaves `IdentityMgr` (ADR-0067 K2 — leak
//! resistance; the held `SvidMaterial`'s `leaf_key` stays inside the holder).
//!
//! # Why a projection, not the full `SvidMaterial`
//!
//! The reconciler's `actual` must be a pure value the runtime can hold and
//! compare. Projecting to the two non-secret facts (a) keeps the node-held leaf
//! key off the reconciler's input surface entirely, and (b) gives the
//! near-expiry seam (ADR-0067 rev 3 D8) the `not_after` it compares against
//! `tick.now_unix` — a value that, post the ADR-0063 rev 2 amendment, equals
//! the `issued_certificates` audit row's `not_after` and derives from the same
//! injected clock, so the comparison is sound and DST-deterministic.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::SpiffeId;
use crate::ca::WORKLOAD_SVID_TTL;
use crate::id::{AllocationId, ContentHash, CorrelationKey, NodeId, WorkloadId};
use crate::wall_clock::UnixInstant;

use super::{Action, Reconciler, ReconcilerName, TickContext, backoff_for_attempt};

/// The per-allocation projection of a held workload SVID — the `actual` the
/// `SvidLifecycle` reconciler (01-04) reads via `IdentityMgr::held_snapshot`.
///
/// Carries the two non-secret facts the reconciler's decisions consume:
///
/// * `spiffe_id` — the identity the held leaf was minted for (the
///   `running ∧ ¬held` branch compares the desired identity against this).
/// * `not_after` — the held leaf's validity-window end (the near-expiry seam,
///   ADR-0067 rev 3 D8, compares this against `tick.now_unix`). An OBSERVED
///   FACT of the minted credential, equal to the `issued_certificates` row's
///   `not_after` by construction (ADR-0063 rev 2 amendment) — NOT a
///   recompute-from-policy deadline.
///
/// It DELIBERATELY does NOT carry the leaf private key: the
/// [`CaKeyPem`](crate::traits::ca::CaKeyPem) stays inside `IdentityMgr` (K2 —
/// the held secret is never projected into a reconciler input). `HeldSvidFacts`
/// derives `Debug`/`Clone`/`PartialEq`/`Eq` because the reconciler runtime
/// holds, clones, and diffs `actual` values; both fields are non-secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldSvidFacts {
    /// The identity the held leaf was minted for.
    pub spiffe_id: SpiffeId,
    /// The held leaf's validity-window end.
    pub not_after: UnixInstant,
}

/// The per-allocation `desired` fact the `SvidLifecycle` reconciler needs to
/// emit [`Action::IssueSvid`] for a Running allocation — the inputs to the pure
/// `SpiffeId::for_allocation` derivation plus the issuing node.
///
/// The runtime's hydrate-desired projects one of these per Running
/// `alloc_status` observation row (filtered to the target workload), exactly as
/// the `WorkloadLifecycle` / `BackendDiscoveryBridge` arms project the running
/// set. The `AllocationId` is the [`SvidLifecycleState::desired`] map key;
/// `RunningAlloc` carries the remaining two fields the issuance request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAlloc {
    /// The workload the allocation belongs to — the first component of the
    /// derived `spiffe://overdrive.local/job/<workload>/alloc/<alloc>` identity.
    pub workload_id: WorkloadId,
    /// The node the allocation runs on — carried on `IssueSvid` so the action is
    /// self-describing (the `issued_certificates` row's `node_id`, ADR-0067 D2).
    pub node_id: NodeId,
}

/// `desired` / `actual` projection for the `SvidLifecycle` reconciler
/// (ADR-0067 D1/D4).
///
/// As with [`WorkflowLifecycleState`](super::WorkflowLifecycleState), ONE
/// `State` type is instantiated by the runtime in two roles:
///
/// * **`desired`** — `desired` carries the currently-**Running** allocations
///   (`actual` is ignored on this value), keyed by [`AllocationId`].
/// * **`actual`** — `actual` carries the [`IdentityMgr`]-held snapshot
///   (`desired` is ignored on this value), keyed by [`AllocationId`].
///
/// `reconcile` reads `desired.desired` and `actual.actual` / `actual.ever_issued`
/// to converge the two sets. Both maps are [`BTreeMap`] / [`BTreeSet`] for
/// deterministic iteration across DST seeds per `.claude/rules/development.md` §
/// "Ordered-collection choice" — the reconcile body iterates them and the
/// held-set invariant walks the result.
///
/// [`IdentityMgr`]: ../../../overdrive_control_plane/identity_mgr/struct.IdentityMgr.html
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SvidLifecycleState {
    /// The Running-allocation set (the `desired` role). Keyed by
    /// [`AllocationId`]; the value carries the issuance-request fields.
    pub desired: BTreeMap<AllocationId, RunningAlloc>,
    /// The held-SVID snapshot (the `actual` role) — presence of a key means the
    /// allocation is currently held; the value is the non-secret
    /// [`HeldSvidFacts`] projection (`IdentityMgr::held_snapshot`).
    pub actual: BTreeMap<AllocationId, HeldSvidFacts>,
    /// The DURABLE restart-recovery success signal (ADR-0067 rev 5 D10): the set
    /// of `SpiffeId`s observed in the `issued_certificates` audit rows. A running
    /// alloc whose pure-derived [`SpiffeId::for_allocation`] is in this set was
    /// *successfully issued before* (audit-before-hold, ADR-0063 D6 — the row
    /// exists only on a prior successful mint).
    ///
    /// This is the volatile-held-set's durable complement: `¬held ∧ ever_issued`
    /// is the unambiguous *restart marker* (minted-then-lost-hold), distinguished
    /// from `¬held ∧ ¬ever_issued` (genuinely never-succeeded). It is a
    /// recomputed-each-tick projection of the durable audit rows — derived at read
    /// time in the runtime's `hydrate_actual`, NEVER persisted (the audit row is
    /// an existing observation INPUT the reconciler READS; it writes nothing). The
    /// reconciler derives the expected `spiffe_id` per running alloc and tests
    /// membership against this set — keyed on `spiffe_id` because the audit row
    /// carries `spiffe_id`, NOT `alloc_id`.
    ///
    /// Empty on a `desired`-role value (`hydrate_desired` fills only `desired`);
    /// the reconcile body reads `actual.ever_issued` off the `actual`-role value.
    pub ever_issued: BTreeSet<SpiffeId>,
}

/// Per-allocation issue-retry memory — the INPUTS the backoff schedule consumes
/// (ADR-0067 D8; the `development.md` § "Reconciler I/O" `RetryMemory` shape).
///
/// This is retry-policy memory for a FAILED `IssueSvid`, NOT an issuance success
/// fact: there is deliberately NO `serial` (a post-dispatch executor output the
/// pure reconciler cannot know — and the runtime persists `next_view` BEFORE
/// dispatch, so a "success" View could be durably written when the CA / audit
/// write then fails), NO `issued_at`-as-proof, NO `spiffe_id`, and NO derived
/// `expires_at` / `next_renewal_at` deadline. The success fact lives in the
/// `issued_certificates` observation row; "is this alloc held?" is answered by
/// `actual` (the held set), never by the View.
///
/// The backoff DEADLINE is recomputed every tick from these two inputs +
/// [`backoff_for_attempt`] (`last_failure_seen_at + backoff_for_attempt(attempts)`)
/// — never persisted, so a future operator-tunable backoff policy lands without a
/// schema migration (`.claude/rules/development.md` § "Persist inputs, not
/// derived state").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRetry {
    /// Failed-issue attempt count (input to the backoff schedule).
    #[serde(default)]
    pub attempts: u32,
    /// When the last failed issue was observed (input; the backoff DEADLINE is
    /// recomputed each tick from this + the policy, never persisted).
    #[serde(default = "epoch_zero")]
    pub last_failure_seen_at: UnixInstant,
}

/// Default `last_failure_seen_at` for serde — [`UnixInstant`] does not implement
/// `Default`, so we provide an epoch-zero value for new rows where no failure has
/// been observed yet (the `ServiceMapHydrator::RetryMemory` precedent).
const fn epoch_zero() -> UnixInstant {
    UnixInstant::from_unix_duration(Duration::ZERO)
}

impl Default for IssueRetry {
    fn default() -> Self {
        Self { attempts: 0, last_failure_seen_at: epoch_zero() }
    }
}

/// Typed memory for the `SvidLifecycle` reconciler — RETRY MEMORY ONLY
/// (ADR-0067 D8).
///
/// The View's only job is to let a *failed* `IssueSvid` back off instead of
/// re-firing every tick: it persists, per allocation, the retry inputs
/// ([`IssueRetry`]). It carries NO issuance success fact (no `serial` /
/// `issued_at` / `spiffe_id`) and NO derived future-event field (no `expires_at`
/// / `next_renewal_at`) — those are review-rejection smells per ADR-0067 D8 (the
/// success fact lives in the `issued_certificates` observation row; held-ness is
/// `actual`; the near-expiry deadline is recomputed from the held cert's real
/// `not_after`, read off `actual`).
///
/// SIX derives (incl. `Eq`) — NOT the usual four: the runtime's NextView **diff**
/// compares the returned `next_view` against the prior to decide whether to write
/// through, so `Eq` is required (`reconciler_runtime`'s persist-on-change path).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SvidLifecycleView {
    /// Per-allocation issue-retry memory. An absent entry ⇒ no failed issue
    /// attempt recorded; the next `running ∧ ¬held` tick issues immediately.
    #[serde(default)]
    pub retry: BTreeMap<AllocationId, IssueRetry>,
}

/// The near-expiry threshold (seconds) the LIVE near-expiry rotation branch
/// compares the held cert's REAL `not_after` against (`held.not_after <=
/// tick.now_unix + NEAR_EXPIRY_THRESHOLD_SECS` ⇒ near-expiry; ADR-0067 D8,
/// feature-delta D-OC-3).
///
/// DERIVED from [`WORKLOAD_SVID_TTL`] — it is ½ × the workload-SVID TTL (1800s
/// today, since the TTL is 3600s). It is NOT a bare literal: `Duration::as_secs`
/// and `u64 / 2` are both `const`, so the threshold tracks a TTL-policy change
/// at compile time. Re-issuing at the half-life leaves a full ½-TTL window for
/// the rotate `IssueSvid` to complete and the new leaf to be held before the old
/// one expires.
pub const NEAR_EXPIRY_THRESHOLD_SECS: u64 = WORKLOAD_SVID_TTL.as_secs() / 2;

/// The near-expiry predicate: is the held cert's REAL `not_after` within
/// [`NEAR_EXPIRY_THRESHOLD_SECS`] of `now` (ADR-0067 D8)? Inclusive at the
/// boundary (`<=`): a cert expiring at exactly `now + threshold` rotates.
///
/// Extracted into its own function so the threshold-window computation and the
/// `<=` comparison are one named, reviewable predicate whose mutation can be
/// suppressed at function granularity.
// mutants: skip — the `<=` boundary is the LIVE mutation target, but its kill
// test (S-OC-03 `near_expiry_boundary_is_inclusive_at_half_ttl`) and the
// removal of this skip + the `.cargo/mutants.toml` exclude_re entry land
// TOGETHER in step 01-02, so the boundary is never live-but-unkilled. Until
// then this skip stays (step 01-01 flips the branch live; 01-02 un-skips and
// adds the kill test in one commit).
fn near_expiry(not_after: UnixInstant, now: UnixInstant) -> bool {
    not_after <= now + Duration::from_secs(NEAR_EXPIRY_THRESHOLD_SECS)
}

/// The workload-SVID lifecycle reconciler (ADR-0067 D1).
pub struct SvidLifecycle {
    name: ReconcilerName,
}

impl SvidLifecycle {
    /// Construct the canonical `svid-lifecycle` instance.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal satisfying every
    /// `ReconcilerName` validation rule.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'svid-lifecycle' is a valid ReconcilerName by construction");
        Self { name }
    }
}

/// Derive the deterministic [`CorrelationKey`] for an identity action against an
/// allocation: `target = "svid-lifecycle/<alloc>"`, `spec_hash =
/// ContentHash::of(<spiffe-uri bytes>)`, `purpose ∈ {"issue-svid",
/// "rotate-svid", "drop-svid"}` (ADR-0067 D2 — the ADR-0035 reconciler-I/O correlation
/// discipline; the content identity is the SVID's own SPIFFE URI, stable across
/// ticks for the same allocation, NOT a per-attempt request id).
fn identity_correlation(
    alloc: &AllocationId,
    spiffe_id: &SpiffeId,
    purpose: &str,
) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe_id.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, purpose)
}

impl Reconciler for SvidLifecycle {
    /// Canonical kebab-case name; single compile-time anchor.
    const NAME: &'static str = "svid-lifecycle";

    type State = SvidLifecycleState;
    type View = SvidLifecycleView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    /// Pure-sync `reconcile` (ADR-0035 / ADR-0067 D1/D8). Converges
    /// `desired = Running allocs` (`desired.desired`) against
    /// `actual = held set` (`actual.actual`) and maintains the retry-memory View:
    ///
    /// - **`running ∧ ¬held ∧ ever_issued`** (RESTART RECOVERY, ADR-0067 rev 5
    ///   D10) → emit [`Action::IssueSvid`] **IMMEDIATELY, bypassing the backoff
    ///   gate**, and CLEAR the alloc's `IssueRetry` entry. `ever_issued` is the
    ///   pure-derived [`SpiffeId::for_allocation`] identity being present in
    ///   `actual.ever_issued` (the `issued_certificates` audit-row identity set,
    ///   D4/D10) — an unambiguous restart marker (a prior successful mint whose
    ///   hold was lost when the holder was reset). Evaluated BEFORE the backoff
    ///   gate, so a stale retry entry (a record-on-emit artifact from the prior
    ///   successful issue) can no longer suppress recovery. This is the ordinary
    ///   first-issue branch running again — NOT the gated #40 rotation path.
    /// - **`running ∧ ¬held ∧ ¬ever_issued`** (first-issue / genuinely-failing) →
    ///   emit [`Action::IssueSvid`] carrying the pure-derived
    ///   [`SpiffeId::for_allocation`] identity, the issuing `node_id`, and the
    ///   derived `issue-svid` correlation — BUT only when no [`IssueRetry`] entry
    ///   exists for the alloc OR the backoff window has elapsed (`tick.now_unix >=
    ///   last_failure_seen_at + backoff_for_attempt(attempts)`; the deadline is
    ///   recomputed each tick from the persisted inputs + the live policy, NEVER
    ///   persisted). Each emitted `IssueSvid` bumps the alloc's `IssueRetry` in
    ///   `next_view` (`attempts += 1`, `last_failure_seen_at = tick.now_unix`) —
    ///   the `bump_if_dispatched` shape, so a re-issue that then FAILS backs off
    ///   (the pure reconciler infers "still failing" from the alloc remaining
    ///   `¬held ∧ ¬ever_issued` next tick with a retry entry).
    /// - **`¬running ∧ held`** → emit [`Action::DropSvid`] so the executor drops
    ///   the held leaf key (ADR-0067 O2 — leak resistance on stop).
    /// - **`running ∧ held`** → clear the alloc's `IssueRetry` entry (the issue
    ///   succeeded — it is now in `actual`) AND evaluate the LIVE near-expiry
    ///   ROTATION branch (feature-delta D-OC-1/2/3, decision A1): when the held
    ///   cert's real `not_after` (read off `actual`, D4) is within
    ///   [`NEAR_EXPIRY_THRESHOLD_SECS`] (½ × `WORKLOAD_SVID_TTL`) of
    ///   `tick.now_unix`, emit a SINGLE rotate [`Action::IssueSvid`]
    ///   UNCONDITIONALLY — carrying the HELD `spiffe_id`, the running `node_id`,
    ///   and a `"rotate-svid"` correlation. Internal near-expiry reissue is a
    ///   reconciler ACTION, not a workflow (`.claude/rules/workflows.md`). The
    ///   `"rotate-svid"` correlation DISTINGUISHES it from the restart-recovery /
    ///   first-issue `"issue-svid"` path (the `running ∧ ¬held` branch).
    /// - **GC** — `IssueRetry` entries for allocations no longer Running are
    ///   dropped from `next_view` (mirror `ServiceMapHydrator`'s `retain`).
    ///
    /// The body holds no `.await`, reads wall-clock only via `tick.now_unix`,
    /// consults no RNG, and holds no CA / ObservationStore handle — it builds the
    /// `SpiffeId` purely and passes it in the action; CA I/O is the executor's
    /// (D3). dst-lint holds.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();
        let mut next_view = view.clone();

        // Per running alloc, in the ADR-0067 rev 5 D10 priority order:
        //   held              → no-op + gated near-expiry; clear retry
        //   ¬held ∧ ever_issued → restart recovery: IssueSvid IMMEDIATELY
        //                         (bypass the backoff gate); clear retry
        //   ¬held ∧ ¬ever_issued → first-issue / failing: IssueSvid backoff-gated;
        //                          record/keep the failure memory
        for (alloc_id, running) in &desired.desired {
            if let Some(held) = actual.actual.get(alloc_id) {
                // running ∧ held → the issue succeeded, so clear any recorded
                // retry memory (clear-on-success).
                next_view.retry.remove(alloc_id);

                // The LIVE near-expiry ROTATION branch (ADR-0067 D8;
                // feature-delta D-OC-1/2/3, decision A1). It reads the held
                // cert's REAL `not_after` (`HeldSvidFacts.not_after`, an OBSERVED
                // fact off `actual`, D4 — NOT a View field; there is no
                // `expires_at` anywhere) and compares it against
                // `tick.now_unix + NEAR_EXPIRY_THRESHOLD_SECS` (½ × TTL),
                // recomputed each tick (no persisted deadline). When near-expiry
                // it emits a SINGLE rotate `Action::IssueSvid` UNCONDITIONALLY —
                // internal SVID near-expiry reissue is a reconciler ACTION, not a
                // workflow (`.claude/rules/workflows.md`): a single internal
                // mint+swap with no external DNS-propagation wait. It reuses the
                // EXISTING `IssueSvid` variant, carries the HELD `spiffe_id` (off
                // `actual`, D4), the running `node_id`, and a `"rotate-svid"`
                // correlation that DISTINGUISHES it from the restart-recovery /
                // first-issue `"issue-svid"` path (`running ∧ ¬held`, below). The
                // executor does the single `Ca::issue_svid` mint+swap; in-flight
                // readers see either the old or the new leaf (D2).
                if near_expiry(held.not_after, tick.now_unix) {
                    let correlation =
                        identity_correlation(alloc_id, &held.spiffe_id, "rotate-svid");
                    actions.push(Action::IssueSvid {
                        alloc_id: alloc_id.clone(),
                        spiffe_id: held.spiffe_id.clone(),
                        node_id: running.node_id.clone(),
                        correlation,
                    });
                }
                continue;
            }

            // ¬held — derive the identity once; it drives both the recovery
            // membership test (against `actual.ever_issued`) and the action.
            let spiffe_id = SpiffeId::for_allocation(&running.workload_id, alloc_id);

            // ¬held ∧ ever_issued → RESTART RECOVERY (ADR-0067 rev 5 D10): a
            // prior successful mint whose hold was lost to a control-plane
            // restart, proven by the `issued_certificates` audit row's identity
            // being in `actual.ever_issued`. Re-issue IMMEDIATELY, BYPASSING the
            // backoff gate — and clear any stale retry entry, because a durable
            // success is proven (no failure is pending). This is the fix for the
            // `restart_after_successful_issue_before_clear` defect: the
            // `ever_issued` check is evaluated BEFORE the backoff gate, so a stale
            // retry entry (a record-on-emit artifact from the prior successful
            // issue) can no longer suppress recovery. It is the ordinary
            // first-issue branch running again because the holder was reset —
            // NOT the gated #40 rotation path.
            if actual.ever_issued.contains(&spiffe_id) {
                next_view.retry.remove(alloc_id);
                let correlation = identity_correlation(alloc_id, &spiffe_id, "issue-svid");
                actions.push(Action::IssueSvid {
                    alloc_id: alloc_id.clone(),
                    spiffe_id,
                    node_id: running.node_id.clone(),
                    correlation,
                });
                continue;
            }

            // ¬held ∧ ¬ever_issued — first-issue / genuinely-failing path. Backoff
            // gate: emit only when no prior failed attempt is recorded OR the
            // backoff window has elapsed. The DEADLINE is recomputed here from the
            // persisted inputs (`attempts` + `last_failure_seen_at`) + the live
            // `backoff_for_attempt` policy — never persisted (a `next_attempt_at`
            // field would be a persist-derived-state smell;
            // `.claude/rules/development.md` § "Persist inputs, not derived
            // state").
            if let Some(retry) = next_view.retry.get(alloc_id) {
                let deadline = retry.last_failure_seen_at + backoff_for_attempt(retry.attempts);
                if tick.now_unix < deadline {
                    // Inside the backoff window — suppress the re-issue this
                    // tick; the retry entry is preserved (NOT cleared, NOT
                    // bumped) so the deadline recomputes identically next tick.
                    continue;
                }
            }

            let correlation = identity_correlation(alloc_id, &spiffe_id, "issue-svid");
            actions.push(Action::IssueSvid {
                alloc_id: alloc_id.clone(),
                spiffe_id,
                node_id: running.node_id.clone(),
                correlation,
            });

            // Record the attempt: bump the retry memory so a re-issue that then
            // FAILS backs off (the alloc remains `¬held` next tick with a
            // recorded entry). Persist INPUTS — `attempts` (count) and
            // `last_failure_seen_at` (`tick.now_unix`, the observation
            // timestamp), never the deadline.
            let entry = next_view.retry.entry(alloc_id.clone()).or_default();
            entry.attempts = entry.attempts.saturating_add(1);
            entry.last_failure_seen_at = tick.now_unix;
        }

        // ¬running ∧ held → DropSvid.
        for (alloc_id, held) in &actual.actual {
            if desired.desired.contains_key(alloc_id) {
                continue;
            }
            let correlation = identity_correlation(alloc_id, &held.spiffe_id, "drop-svid");
            actions.push(Action::DropSvid { alloc_id: alloc_id.clone(), correlation });
        }

        // GC: drop retry memory for allocations no longer in the Running set
        // (mirror `ServiceMapHydrator`'s `retain`). The clear-on-success path
        // above already removed entries for now-held running allocs; this prunes
        // entries for allocs that have left the running set entirely.
        next_view.retry.retain(|alloc_id, _| desired.desired.contains_key(alloc_id));

        // The §18 self-re-enqueue gate treats an all-Noop vector as "converged
        // this tick"; emit a single Noop when nothing needed doing so the gate
        // reads the converged shape (mirrors `WorkflowLifecycle::reconcile`).
        if actions.is_empty() {
            actions.push(Action::Noop);
        }

        (actions, next_view)
    }
}

#[cfg(test)]
mod tests {
    use super::HeldSvidFacts;
    use crate::SpiffeId;
    use crate::wall_clock::UnixInstant;
    use std::time::Duration;

    /// `HeldSvidFacts` is a faithful two-field projection: constructing it from a
    /// `spiffe_id` + `not_after` exposes exactly those two values back through
    /// its public fields. This pins the projection shape the
    /// `IdentityMgr::held_snapshot` surface produces (01-03) and the
    /// `SvidLifecycle` reconciler `actual` consumes (01-04) — a regression that
    /// dropped or swapped a field is caught here.
    #[test]
    fn held_svid_facts_carries_the_identity_and_validity_end() {
        let spiffe = SpiffeId::new("spiffe://overdrive.local/job/payments/alloc/a1b2c3")
            .expect("valid workload SpiffeId");
        let not_after = UnixInstant::from_unix_duration(Duration::from_secs(1_700_003_600));

        let facts = HeldSvidFacts { spiffe_id: spiffe.clone(), not_after };

        assert_eq!(facts.spiffe_id, spiffe, "projection preserves the held identity");
        assert_eq!(facts.not_after, not_after, "projection preserves the validity-window end");
    }
}
