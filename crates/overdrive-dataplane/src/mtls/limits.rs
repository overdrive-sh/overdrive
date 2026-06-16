//! F4/F7 resource-limit ENFORCEMENT for the transparent-mTLS proxy (ADR-0069,
//! GH #26; step 04-01).
//!
//! The [`MtlsLimits`] STRUCT and its F7 `Default` are the FROZEN contract
//! (`overdrive-core::traits::mtls_enforcement`); this module is the host-adapter
//! ENFORCEMENT of those bounds — the pure trip predicates (extracted so the
//! `<` vs `<=` boundary at each CONCRETE threshold is unit- and mutation-testable)
//! plus the per-allocation in-flight ledger that gates `enforce` fail-closed at the
//! `max_inflight_per_alloc` ceiling.
//!
//! The four trip predicates each pin one CONCRETE F7 threshold (criteria 3/4):
//! - [`prearm_exceeds`] — the pre-arm buffer cap (256 KiB): trips at `held > max`
//!   (the 256 KiB+1 byte), NOT at `held == max`. → `BufferLimitExceeded`.
//! - [`inflight_at_ceiling`] — the per-alloc in-flight ceiling (128): the 129th
//!   CONCURRENT pre-arm is refused (the claim that would make `current == max` is
//!   the LAST admitted; the next, which would reach `max + 1`, trips). →
//!   `InFlightLimitExceeded`.
//! - [`handshake_expired`] — the handshake-and-arm deadline (5 s): trips at
//!   `elapsed >= deadline`. → `HandshakeTimeout`.
//! - [`stall_elapsed`] — the F6 pump-stall deadline (30 s): trips at
//!   `no_progress_for >= deadline` (consumed by [`super::supervision`]). →
//!   `PumpLiveness::Stalled`.
//!
//! Every trip is FAIL-CLOSED: the buffer is dropped, the leg reset, the intercept
//! refused — never queue-unbounded, never degrade to cleartext.

use std::time::Duration;

use overdrive_core::AllocationId;
use parking_lot::Mutex;

/// `true` iff the pre-arm plaintext captured so far EXCEEDS `max_prearm_bytes` —
/// i.e. `held > max` (the 256 KiB+1 byte trips; exactly 256 KiB does NOT). The
/// `BufferLimitExceeded` DoS guard (F4): a workload streaming more than this before
/// kTLS arms has its buffer dropped and its leg reset, no cleartext egresses.
///
/// The `>` (NOT `>=`) is load-bearing and mutation-pinned: a connection that wrote
/// EXACTLY `max_prearm_bytes` is within budget; only the byte PAST the cap trips.
#[must_use]
pub(super) const fn prearm_exceeds(held_len: usize, max_prearm_bytes: usize) -> bool {
    held_len > max_prearm_bytes
}

/// `true` iff a NEW pre-arm intercept must be refused because the per-allocation
/// in-flight ceiling is already saturated — i.e. `current >= max` (admitting one
/// more would exceed `max_inflight_per_alloc`). With `max == 128`, the 129th
/// concurrent pre-arm (the one that finds `current == 128`) is refused
/// (`InFlightLimitExceeded`); the 128th (finding `current == 127`) is admitted.
///
/// The `>=` (NOT `>`) is load-bearing and mutation-pinned: the ceiling is the count
/// of concurrently-admitted pre-arms, so refusal fires the instant a new claim would
/// make the count exceed `max`.
#[must_use]
pub(super) const fn inflight_at_ceiling(current: u32, max_inflight_per_alloc: u32) -> bool {
    current >= max_inflight_per_alloc
}

/// `true` iff the handshake-and-arm has run for at least `handshake_deadline` —
/// i.e. `elapsed >= deadline`. The `HandshakeTimeout` trip (F4): a stalled/silent
/// peer that has not completed the handshake within the deadline is refused
/// fail-closed so it cannot pin agent resources.
///
/// The `>=` is load-bearing and mutation-pinned: reaching the deadline trips.
#[must_use]
pub(super) const fn handshake_expired(elapsed: Duration, handshake_deadline: Duration) -> bool {
    elapsed.as_nanos() >= handshake_deadline.as_nanos()
}

/// `true` iff a pump's bytes-moved progress metric has not advanced for at least
/// `pump_stall_deadline` — i.e. `no_progress_for_nanos >= deadline_nanos`. The F6
/// stall trip (consumed by [`super::supervision::derive_liveness`]): a pump pending
/// a record whose progress has frozen this long is `Stalled`.
///
/// The `>=` is load-bearing and mutation-pinned: reaching the deadline trips.
#[must_use]
pub(super) const fn stall_elapsed(no_progress_for_nanos: u64, deadline_nanos: u64) -> bool {
    no_progress_for_nanos >= deadline_nanos
}

/// Per-allocation in-flight (pre-arm, not-yet-armed) connection ledger enforcing the
/// `max_inflight_per_alloc` ceiling (F4) atomically. One workload cannot exhaust the
/// agent by opening many stalled connections: a new claim that would exceed the
/// ceiling is REFUSED fail-closed (the claim returns `None`); admitted claims are an
/// RAII [`InFlightGuard`] that releases the slot on drop (when the pre-arm completes
/// — established or failed-closed).
///
/// Check-and-act is atomic (`.claude/rules/development.md` § "Check-and-act must be
/// atomic"): the count check and the increment happen under one lock; the guard's
/// Drop is the matching decrement. There is no separate `contains`-then-`insert`
/// TOCTOU window.
#[derive(Debug, Default)]
pub(super) struct InFlightLedger {
    // dst-lint: hashmap-ok adapter-host crate (not core-scanned); point-access only,
    // keyed by AllocationId, never iterated — per-alloc concurrent pre-arm counter.
    counts: Mutex<std::collections::HashMap<AllocationId, u32>>,
}

impl InFlightLedger {
    /// Construct an empty ledger.
    #[must_use]
    pub(super) fn new() -> Self {
        Self { counts: Mutex::new(std::collections::HashMap::new()) }
    }

    /// Attempt to claim one in-flight pre-arm slot for `alloc`, bounded by `max`.
    /// Returns `Some(guard)` if admitted (the slot is held until the guard drops),
    /// `None` if the per-alloc ceiling is already at `max` (the caller refuses the
    /// intercept fail-closed with `InFlightLimitExceeded`).
    ///
    /// Atomic: the ceiling check and the increment are one locked operation — the
    /// claim's own outcome IS the check at the moment of mutation (no stale
    /// pre-check).
    pub(super) fn try_claim(&self, alloc: &AllocationId, max: u32) -> Option<InFlightGuard<'_>> {
        let mut guard = self.counts.lock();
        let current = guard.get(alloc).copied().unwrap_or(0);
        if inflight_at_ceiling(current, max) {
            return None;
        }
        guard.insert(alloc.clone(), current + 1);
        // The check + increment above are one atomic critical section (no TOCTOU);
        // drop the lock now — the returned guard's release is a separate lock.
        drop(guard);
        Some(InFlightGuard { ledger: self, alloc: alloc.clone() })
    }
}

/// RAII slot held by [`InFlightLedger::try_claim`]. Releasing it (drop) decrements
/// the per-alloc count — so a slot is reclaimed whether the pre-arm established or
/// failed closed.
pub(super) struct InFlightGuard<'a> {
    ledger: &'a InFlightLedger,
    alloc: AllocationId,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        let mut guard = self.ledger.counts.lock();
        if let Some(count) = guard.get_mut(&self.alloc) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.remove(&self.alloc);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::items_after_statements)]
mod tests {
    //! Boundary unit tests for the F4/F7 trip predicates — each is its own driving
    //! port (a pure decision fn; calling it IS port-to-port testing, Mandate 2). The
    //! `<`/`<=`/`>`/`>=` boundary at each CONCRETE threshold is mutation-pinned here
    //! so a `> → ==` / `>= → >` / fn-body→`false` mutation dies.

    use super::*;

    fn alloc() -> AllocationId {
        AllocationId::new("alloc-limits-test").expect("valid alloc")
    }

    /// `prearm_exceeds` — the 256 KiB buffer cap. `held == max` is WITHIN budget;
    /// `held == max + 1` (the 256 KiB+1 byte) is the FIRST to trip. Pins `>` (not
    /// `>=`, not `==`): exactly-at-cap must NOT trip; one past must.
    #[test]
    fn prearm_exceeds_trips_only_past_the_cap() {
        const CAP: usize = 262_144; // 256 KiB
        assert!(!prearm_exceeds(0, CAP), "empty pre-arm is within budget");
        assert!(!prearm_exceeds(CAP - 1, CAP), "one below the cap is within budget");
        assert!(!prearm_exceeds(CAP, CAP), "EXACTLY at the cap is within budget (kills `> → >=`)");
        assert!(prearm_exceeds(CAP + 1, CAP), "the 256 KiB+1 byte trips (kills `> → ==`)");
        assert!(prearm_exceeds(CAP * 2, CAP), "far past the cap trips");
    }

    /// `inflight_at_ceiling` — the per-alloc in-flight ceiling. `current == max`
    /// refuses the NEXT claim (the 129th finding count 128); `current == max - 1`
    /// admits it. Pins `>=` (not `>`): exactly-at-ceiling must refuse.
    #[test]
    fn inflight_at_ceiling_refuses_at_and_above_the_ceiling() {
        const MAX: u32 = 128;
        assert!(!inflight_at_ceiling(0, MAX), "no pre-arms held ⇒ admit");
        assert!(!inflight_at_ceiling(MAX - 1, MAX), "127 held ⇒ the 128th is admitted");
        assert!(inflight_at_ceiling(MAX, MAX), "128 held ⇒ the 129th is refused (kills `>= → >`)");
        assert!(inflight_at_ceiling(MAX + 1, MAX), "above the ceiling stays refused");
    }

    /// `handshake_expired` — the 5 s handshake deadline. `elapsed >= deadline` trips.
    /// Pins the predicate is not constant-`false` and the `>=` boundary.
    #[test]
    fn handshake_expired_trips_at_and_past_the_deadline() {
        let deadline = Duration::from_secs(5);
        assert!(!handshake_expired(Duration::ZERO, deadline), "no time elapsed ⇒ not expired");
        assert!(
            !handshake_expired(Duration::from_millis(4_999), deadline),
            "just under the deadline ⇒ not expired"
        );
        assert!(
            handshake_expired(deadline, deadline),
            "exactly at the deadline ⇒ expired (kills fn → false and `>= → >`)"
        );
        assert!(handshake_expired(Duration::from_secs(6), deadline), "past the deadline ⇒ expired");
    }

    /// `stall_elapsed` — the 30 s pump-stall deadline. `no_progress >= deadline`
    /// trips. Pins the predicate is not constant-`false` and the `>=` boundary.
    #[test]
    fn stall_elapsed_trips_at_and_past_the_deadline() {
        let deadline_nanos = Duration::from_secs(30).as_nanos() as u64;
        assert!(!stall_elapsed(0, deadline_nanos), "no stall yet ⇒ not elapsed");
        assert!(
            !stall_elapsed(deadline_nanos - 1, deadline_nanos),
            "one nano under the deadline ⇒ not elapsed"
        );
        assert!(
            stall_elapsed(deadline_nanos, deadline_nanos),
            "exactly at the deadline ⇒ elapsed (kills fn → false and `>= → >`)"
        );
        assert!(stall_elapsed(deadline_nanos * 2, deadline_nanos), "past the deadline ⇒ elapsed");
    }

    /// The `InFlightLedger` claim/release lifecycle: claims up to the ceiling are
    /// admitted, the over-ceiling claim is refused, and dropping a guard RELEASES the
    /// slot (so a subsequent claim is admitted again). Pins the `Drop` decrement (a
    /// `drop → ()` mutation leaves the slot held → the re-claim would be refused) and
    /// the `*count == 0` removal branch.
    #[test]
    fn ledger_claims_to_ceiling_refuses_over_and_releases_on_drop() {
        let ledger = InFlightLedger::new();
        let a = alloc();
        const MAX: u32 = 3;

        let g1 = ledger.try_claim(&a, MAX).expect("1st claim admitted");
        let g2 = ledger.try_claim(&a, MAX).expect("2nd claim admitted");
        let g3 = ledger.try_claim(&a, MAX).expect("3rd claim admitted (count now == MAX)");
        // The 4th claim (count == MAX) is refused.
        assert!(ledger.try_claim(&a, MAX).is_none(), "the over-ceiling claim is refused");

        // Releasing ONE guard frees exactly ONE slot — the next claim is admitted
        // (count 2 → 3) but a SECOND claim is refused (back at the ceiling). This pins
        // the decrement is EXACTLY one (a `drop → ()` no-op would refuse the re-claim;
        // a `count == 0` → `count != 0` mutation would REMOVE the key on this
        // non-zero release, dropping g1+g2's slots and wrongly admitting BOTH the
        // re-claim AND the second claim — so asserting the second is refused KILLS
        // the `== → !=` mutation).
        drop(g3);
        let g4 = ledger.try_claim(&a, MAX).expect("after ONE release, ONE fresh claim is admitted");
        assert!(
            ledger.try_claim(&a, MAX).is_none(),
            "g1 + g2 still hold their slots after g3's release — a second claim is refused (the \
             non-zero release must NOT erase the count)"
        );

        drop(g1);
        drop(g2);
        drop(g4);
        // All slots released ⇒ the alloc key is removed (count hit 0) and the full
        // ceiling is claimable again.
        let _g = ledger.try_claim(&a, MAX).expect("all released ⇒ claimable from zero again");
    }
}
