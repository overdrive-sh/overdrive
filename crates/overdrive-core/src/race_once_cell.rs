//! `RaceOnceCell<T>` — a write-once cell that makes the lost-race outcome
//! *explicit* instead of silently discarded.
//!
//! A raw [`OnceLock`] invites the same check-and-act (TOCTOU) hole as a raw
//! claim set ([`crate::claim_set`]): `OnceLock::set` returns `Err(rejected)`
//! ONLY when a concurrent writer won the race, and that `Err` *is* the race
//! verdict. Discarding it — `let _ = lock.set(v)` — is correct only when any
//! winner is interchangeable; when the value you tried to install MUST be the
//! winner (or must match it), the discarded `Err` silently leaves the cell
//! holding the *other* writer's value, and every later read is wrong. That is
//! exactly the built-in-ca lost-race defect (`adopt_persisted_root` discarding
//! `OnceLock::set`'s `Err` and signing under an ephemeral anchor, commit
//! `ade22762`).
//!
//! This type splits the two intents into two named methods, each of which
//! *consumes* the race verdict rather than dropping it:
//!
//! - [`set_or_read_winner`](RaceOnceCell::set_or_read_winner) — install, or on
//!   a lost race read back the winner. Any winner is acceptable; every caller
//!   agrees on the single value that landed first. This is the one legitimate
//!   "discard the `set` result" case (the generator pattern), contained behind
//!   a method whose name and contract document *why* it is safe.
//! - [`set_or_verify`](RaceOnceCell::set_or_verify) — install, or on a lost
//!   race VERIFY the winner matches under a key projection. A divergent winner
//!   returns [`SetOutcome::Conflict`] so the caller fails loud instead of
//!   absorbing the wrong value.
//!
//! See `.claude/rules/development.md` § "Check-and-act must be atomic (no
//! TOCTOU)". Peer primitive: [`crate::claim_set`].

use std::sync::OnceLock;

/// The outcome of [`RaceOnceCell::set_or_verify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    /// The cell was empty; the offered value won and is now stored.
    Won,
    /// A racer won, but its value matches the offered one under the key
    /// projection — the install is an idempotent no-op.
    MatchedWinner,
    /// A racer won and its value DIVERGES under the key projection. The
    /// offered value was NOT stored; the caller must fail loud.
    Conflict,
}

/// A write-once cell whose lost-race outcome is surfaced, not discarded.
///
/// Wraps a [`OnceLock`]; the only way to install a value is through a method
/// that consumes the `set` verdict, so the "discard the `Err` and proceed"
/// hole cannot be written at a use site.
///
/// ```
/// use overdrive_core::race_once_cell::{RaceOnceCell, SetOutcome};
///
/// let cell: RaceOnceCell<u32> = RaceOnceCell::new();
///
/// // First install wins outright.
/// assert_eq!(cell.set_or_verify(7, |v| v), SetOutcome::Won);
/// // Re-installing the same value is an idempotent match.
/// assert_eq!(cell.set_or_verify(7, |v| v), SetOutcome::MatchedWinner);
/// // A divergent value is a conflict — the stored winner is unchanged.
/// assert_eq!(cell.set_or_verify(9, |v| v), SetOutcome::Conflict);
/// assert_eq!(cell.get(), Some(&7));
/// ```
pub struct RaceOnceCell<T> {
    inner: OnceLock<T>,
}

impl<T> RaceOnceCell<T> {
    /// Construct an empty cell.
    #[must_use]
    pub const fn new() -> Self {
        Self { inner: OnceLock::new() }
    }

    /// The stored value, or `None` if nothing has been installed yet.
    pub fn get(&self) -> Option<&T> {
        self.inner.get()
    }

    /// Install `value`, or — on a lost race — discard it and return the
    /// racer's winning value. Either way the single winning reference is
    /// returned.
    ///
    /// Use this when **any** winner is acceptable and every caller must agree
    /// on whichever value landed first — e.g. lazily generating material where
    /// concurrent generators produce *different* values but all readers must
    /// converge on one. This is the sole sanctioned "ignore the `set` result"
    /// path; the discard is safe precisely because the value is read back, so
    /// no caller proceeds on a stale assumption.
    pub fn set_or_read_winner(&self, value: T) -> &T {
        // Lost-race `Err` is intentionally dropped: the winner is read back
        // immediately below, so no caller acts on a value that did not land.
        let _ = self.inner.set(value);
        self.inner
            .get()
            .unwrap_or_else(|| unreachable!("OnceLock is populated immediately after set"))
    }

    /// Install `value`, or — on a lost race — verify the winner matches under
    /// the `key_of` projection.
    ///
    /// # Returns
    ///
    /// - [`SetOutcome::Won`] iff the cell was empty and `value` is now stored.
    /// - [`SetOutcome::MatchedWinner`] iff a racer won but `key_of(winner) ==
    ///   key_of(&value)` — an idempotent no-op.
    /// - [`SetOutcome::Conflict`] iff a racer won and `key_of` diverges — the
    ///   offered `value` is dropped and the caller must fail loud.
    ///
    /// The install and the verify are a **single** atomic step (`OnceLock::set`
    /// returns `Err` iff a writer already won); there is no separate
    /// `get()`-then-`set()` window. Use this when `value` MUST be the winner or
    /// must be byte-identical to it — the adopt-a-persisted-anchor pattern.
    pub fn set_or_verify<K, F>(&self, value: T, key_of: F) -> SetOutcome
    where
        K: PartialEq + ?Sized,
        F: Fn(&T) -> &K,
    {
        match self.inner.set(value) {
            Ok(()) => SetOutcome::Won,
            Err(rejected) => {
                let winner = self
                    .inner
                    .get()
                    .unwrap_or_else(|| unreachable!("OnceLock is populated immediately after set"));
                if key_of(winner) == key_of(&rejected) {
                    SetOutcome::MatchedWinner
                } else {
                    SetOutcome::Conflict
                }
            }
        }
    }
}

impl<T> Default for RaceOnceCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{RaceOnceCell, SetOutcome};

    /// A fixture with an identity field (`id`) and an unrelated `payload`, so
    /// `set_or_verify` keyed on `id` matches when ids agree even if payloads
    /// differ, and conflicts when ids diverge — mirroring "compare the cert
    /// DER, not the whole struct".
    #[derive(Clone)]
    struct Mat {
        id: u8,
        payload: u8,
    }

    /// An empty cell reads `None`; `set_or_read_winner` installs and returns it.
    #[test]
    fn empty_cell_reads_none_then_installs() {
        let cell: RaceOnceCell<u32> = RaceOnceCell::new();
        assert_eq!(cell.get(), None, "a fresh cell holds nothing");
        assert_eq!(*cell.set_or_read_winner(5), 5, "the first writer's value is returned");
        assert_eq!(cell.get(), Some(&5));
    }

    /// `set_or_read_winner` returns the FIRST winner on a later call; the
    /// second value is discarded (any-winner-acceptable contract).
    #[test]
    fn set_or_read_winner_returns_the_first_winner() {
        let cell: RaceOnceCell<u32> = RaceOnceCell::new();
        let _ = cell.set_or_read_winner(1);
        assert_eq!(*cell.set_or_read_winner(2), 1, "the first value wins; the second is read back");
        assert_eq!(cell.get(), Some(&1));
    }

    /// `set_or_verify` into an empty cell is `Won` and stores the value.
    #[test]
    fn set_or_verify_into_empty_is_won() {
        let cell: RaceOnceCell<u32> = RaceOnceCell::new();
        assert_eq!(cell.set_or_verify(7, |v| v), SetOutcome::Won);
        assert_eq!(cell.get(), Some(&7));
    }

    /// A lost race whose winner matches under the key projection is
    /// `MatchedWinner` — idempotent — even when an unrelated field differs.
    #[test]
    fn set_or_verify_matching_key_is_matched_winner() {
        let cell: RaceOnceCell<Mat> = RaceOnceCell::new();
        let _ = cell.set_or_read_winner(Mat { id: 1, payload: 10 });
        assert_eq!(
            cell.set_or_verify(Mat { id: 1, payload: 99 }, |m| &m.id),
            SetOutcome::MatchedWinner,
            "same id is an idempotent match regardless of the unrelated payload"
        );
        assert_eq!(cell.get().map(|m| m.payload), Some(10), "the winner is unchanged");
    }

    /// A lost race whose winner DIVERGES under the key projection is
    /// `Conflict`; the offered value is dropped and the winner is untouched.
    /// Several distinct id pairs exercise the `==`/`!=` comparison so a mutant
    /// flipping it is killed.
    #[test]
    fn set_or_verify_divergent_key_is_conflict() {
        for (winner_id, offered_id) in [(1u8, 2u8), (2, 1), (0, 255), (255, 0)] {
            let cell: RaceOnceCell<Mat> = RaceOnceCell::new();
            let _ = cell.set_or_read_winner(Mat { id: winner_id, payload: 0 });
            assert_eq!(
                cell.set_or_verify(Mat { id: offered_id, payload: 0 }, |m| &m.id),
                SetOutcome::Conflict,
                "winner id {winner_id} vs offered id {offered_id} must conflict"
            );
            assert_eq!(
                cell.get().map(|m| m.id),
                Some(winner_id),
                "the conflict is surfaced, not absorbed — the winner stays"
            );
        }
    }
}
