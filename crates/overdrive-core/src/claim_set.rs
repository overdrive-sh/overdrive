//! `ClaimSet<K>` — an atomic claim primitive that makes the
//! check-and-act (TOCTOU) split *unrepresentable*.
//!
//! A raw `Arc<Mutex<BTreeSet<K>>>` used as a "is this slot already taken?"
//! gate invites a time-of-check-to-time-of-use bug: callers reach for
//! `contains()` to check and a *separate* `insert()` to act, and two racers
//! both observe "absent" between the two calls. The same hole opens when a
//! caller discards `insert`'s `bool` return — the return value *is* the
//! check-at-the-moment-of-mutation, and dropping it re-opens the gap a
//! prior `contains()` would have appeared to close.
//!
//! This type exposes exactly one mutating operation — [`ClaimSet::try_claim`]
//! — whose return *is* the outcome of the atomic check-and-claim: `Some`
//! guard iff the key was newly claimed, `None` iff a claim is already live.
//! There is no `contains`, no bare `insert`, no `remove`: the racy surface
//! does not exist, so the bug cannot be written here. Release is by RAII —
//! the returned [`ClaimGuard`]'s `Drop` removes the key unconditionally, on
//! normal scope exit *and* on unwind.
//!
//! See `.claude/rules/development.md` § "Check-and-act must be atomic (no
//! TOCTOU)" for the discipline this primitive embodies, and the precedent it
//! retires (`WorkflowEngine::start`'s discarded-`insert` concurrent-start
//! hole, commit `6b9bafde`). Peer primitive: [`crate::race_once_cell`].

use std::collections::BTreeSet;
use std::sync::Arc;

use parking_lot::Mutex;

/// A set of currently-held claims keyed by `K`, exposing only an atomic
/// claim-and-release surface.
///
/// `BTreeSet` (not `HashSet`) for deterministic [`snapshot`](Self::snapshot)
/// iteration, per `.claude/rules/development.md` § "Ordered-collection
/// choice" — observers (e.g. a reconciler deriving `has_live_task`) walk the
/// snapshot, so its order must be stable across seeds. `parking_lot::Mutex`
/// (not `tokio::sync::Mutex`) because the only critical sections are a point
/// `insert` / `remove` / `clone` that never cross an `.await`, and
/// [`ClaimGuard`]'s `Drop` is sync.
///
/// ```
/// use overdrive_core::claim_set::ClaimSet;
///
/// let claims: ClaimSet<u32> = ClaimSet::new();
///
/// let guard = claims.try_claim(7).expect("7 is unheld, so the claim wins");
/// assert!(claims.try_claim(7).is_none(), "a second claim of a held key loses");
///
/// drop(guard); // releasing is RAII — no explicit `remove` surface exists
/// assert!(claims.try_claim(7).is_some(), "a released key is claimable again");
/// ```
pub struct ClaimSet<K: Ord + Clone> {
    held: Arc<Mutex<BTreeSet<K>>>,
}

impl<K: Ord + Clone> ClaimSet<K> {
    /// Construct an empty claim set.
    #[must_use]
    pub fn new() -> Self {
        Self { held: Arc::new(Mutex::new(BTreeSet::new())) }
    }

    /// Atomically claim `key`.
    ///
    /// # Returns
    ///
    /// - `Some(guard)` iff `key` was **not** already held — the claim is now
    ///   live and is released when `guard` is dropped.
    /// - `None` iff a claim for `key` is **already** held — the caller did
    ///   not win the claim and must treat its operation as a no-op.
    ///
    /// # Atomicity
    ///
    /// The membership check and the claim are a **single** locked operation
    /// (`BTreeSet::insert` returns `false` iff the key was already present).
    /// There is no window between a check and the act for a second caller to
    /// slip through — the TOCTOU hole a `contains()`-then-`insert()` pair
    /// would re-open does not exist on this type.
    ///
    /// The returned guard is `#[must_use]`: dropping it immediately releases
    /// the claim, so a discarded guard is almost always a bug (the claim is
    /// gone the instant the statement ends).
    #[must_use = "the claim is released as soon as the returned ClaimGuard is dropped; \
                  hold the guard for the lifetime of the claimed work"]
    pub fn try_claim(&self, key: K) -> Option<ClaimGuard<K>> {
        if self.held.lock().insert(key.clone()) {
            Some(ClaimGuard { held: Arc::clone(&self.held), key })
        } else {
            None
        }
    }

    /// Snapshot the currently-held claim keys.
    ///
    /// A point-in-time clone for read-only observers (a reconciler's
    /// `hydrate_actual` deriving `has_live_task`, a status endpoint). The
    /// snapshot is decoupled from the live set — a claim taken or released
    /// after this call is not reflected. Iteration order is `Ord` on `K`,
    /// deterministic across processes and seeds.
    #[must_use]
    pub fn snapshot(&self) -> BTreeSet<K> {
        self.held.lock().clone()
    }
}

impl<K: Ord + Clone> Default for ClaimSet<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for a live claim taken from a [`ClaimSet`]. Its `Drop` removes
/// the claimed key UNCONDITIONALLY — on normal scope exit AND on an unwind
/// through the holding scope (e.g. a panic in the work the claim guards). A
/// double-release (the guard fires after the key was already removed) is a
/// harmless `BTreeSet::remove` no-op.
///
/// The guard owns an `Arc` clone of the set's interior, so it can be moved
/// into a spawned task and outlive the `&ClaimSet` borrow that produced it.
pub struct ClaimGuard<K: Ord> {
    held: Arc<Mutex<BTreeSet<K>>>,
    key: K,
}

impl<K: Ord> Drop for ClaimGuard<K> {
    fn drop(&mut self) {
        self.held.lock().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, reason = "panic-on-None is the intended test oracle")]

    use super::ClaimSet;

    /// A fresh key claims; a snapshot reflects it.
    #[test]
    fn first_claim_succeeds_and_is_visible_in_snapshot() {
        let set: ClaimSet<u32> = ClaimSet::new();
        let guard = set.try_claim(7);
        assert!(guard.is_some(), "claiming an unheld key must succeed");
        assert!(set.snapshot().contains(&7), "a live claim must appear in the snapshot");
    }

    /// A second claim of a still-held key fails — the atomic check-and-claim
    /// reports the key already present. This is the property the
    /// concurrent-start guard relies on; a `contains`-then-`insert` split
    /// would let both callers through.
    #[test]
    fn second_claim_while_held_returns_none() {
        let set: ClaimSet<u32> = ClaimSet::new();
        let _held = set.try_claim(7).expect("first claim succeeds");
        assert!(
            set.try_claim(7).is_none(),
            "a second claim of an already-held key must return None (no second winner)"
        );
    }

    /// Dropping the guard releases the claim; the key can be re-claimed and
    /// is gone from the snapshot. RAII release is the whole release surface —
    /// there is no explicit `remove`.
    #[test]
    fn dropping_the_guard_releases_the_claim() {
        let set: ClaimSet<u32> = ClaimSet::new();
        let guard = set.try_claim(7).expect("first claim succeeds");
        drop(guard);
        assert!(!set.snapshot().contains(&7), "dropping the guard must release the claim");
        assert!(set.try_claim(7).is_some(), "a released key must be re-claimable");
    }

    /// Distinct keys are independent — claiming one does not block another.
    #[test]
    fn distinct_keys_claim_independently() {
        let set: ClaimSet<u32> = ClaimSet::new();
        let _a = set.try_claim(1).expect("claim 1");
        let _b = set.try_claim(2).expect("claim 2 (distinct key) succeeds");
        let snapshot = set.snapshot();
        assert!(snapshot.contains(&1) && snapshot.contains(&2), "both distinct claims are live");
    }
}
