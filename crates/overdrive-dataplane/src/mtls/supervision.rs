//! F6 pump-liveness derivation for the transparent-mTLS proxy (ADR-0069, GH #26;
//! step 04-01; supervision shape (C)+(B) per ADR-0070 / D-MTLS-16).
//!
//! The [`PumpLiveness`] derivation is extracted here as a PURE function over the
//! pump's shared progress surface so the `Stalled` boundary (the 30 s
//! `pump_stall_deadline` × the record-pending gate) is unit- and mutation-testable
//! independent of the real splice/copy pump. The host adapter's
//! `MtlsEnforcement::liveness` reads the atomics off the `PumpState` and calls
//! [`derive_liveness`].
//!
//! **Who consumes the verdict (ADR-0070 / D-MTLS-16):** there is NO central worker
//! query in v1. Connection liveness is **(C)** kernel `TCP_USER_TIMEOUT`/keepalive on
//! the legs (the kernel reaps the transport-death class) **+ (B)** the per-connection
//! pump task self-tearing-down fail-closed on its own terminal exit (EOF / error /
//! `ETIMEDOUT`). The retired central `MtlsSupervisor` (04-01, shape (A)) is deleted.
//! `derive_liveness` + [`PumpLiveness::Stalled`] are RETAINED as (a) the SD-2 observe
//! surface the equivalence harness re-queries for the post-teardown `Gone` no-leak
//! assertion and (b) the RESERVED predicate for the deferred kernel-invisible
//! progress-stall watchdog ([#232], a per-connection watchdog — NOT a central loop).
//! They are NOT driven by a tick in v1.
//!
//! The two F6 telemetry events (`mtls.pump.stalled` / `mtls.pump.teardown_on_stall`)
//! re-homed from the retired worker supervisor into the per-connection (B)
//! self-teardown path in the host adapter ([`super::HostMtlsEnforcement`]); this
//! module owns only the pure DERIVATION (the dataplane adapter's concern, SD-2).
//!
//! [#232]: https://github.com/overdrive-sh/overdrive/issues/232

use std::time::Duration;

use overdrive_core::traits::mtls_enforcement::PumpLiveness;
use overdrive_core::wall_clock::UnixInstant;

use super::limits::stall_elapsed;

/// Derive a pump's [`PumpLiveness`] from its observable progress surface (F6).
///
/// - `running == false` ⇒ [`PumpLiveness::Gone`] — the pump thread has exited
///   (torn down or never enforced); the post-teardown observable, not an error.
/// - `record_pending == false` ⇒ [`PumpLiveness::Running`] — a purely-idle
///   connection (no record on the source leg) is Running, NEVER Stalled (no false
///   positives on quiescent long-lived connections).
/// - `record_pending == true` AND the bytes-moved metric has not advanced for at
///   least `pump_stall_deadline` (`now - last_progress >= deadline`) ⇒
///   [`PumpLiveness::Stalled`] `{ since: last_progress }` — a stranded/crashed pump.
/// - otherwise ⇒ [`PumpLiveness::Running`].
///
/// Pure over its inputs — the wall-clock `now_unix_nanos` is supplied by the caller
/// (the adapter reads it once per `liveness` call), so the derivation itself reads no
/// clock and is deterministic for the DST equivalence harness.
#[must_use]
pub(super) fn derive_liveness(
    running: bool,
    record_pending: bool,
    last_progress_unix_nanos: u64,
    now_unix_nanos: u64,
    pump_stall_deadline: Duration,
) -> PumpLiveness {
    if !running {
        return PumpLiveness::Gone;
    }
    if !record_pending {
        return PumpLiveness::Running;
    }
    let no_progress_for = now_unix_nanos.saturating_sub(last_progress_unix_nanos);
    let deadline_nanos = u64::try_from(pump_stall_deadline.as_nanos()).unwrap_or(u64::MAX);
    if stall_elapsed(no_progress_for, deadline_nanos) {
        return PumpLiveness::Stalled {
            since: UnixInstant::from_unix_duration(Duration::from_nanos(last_progress_unix_nanos)),
        };
    }
    PumpLiveness::Running
}

#[cfg(test)]
#[allow(clippy::items_after_statements)]
mod tests {
    //! Boundary unit tests for the F6 `derive_liveness` pure transition — its own
    //! driving port (Mandate 2). Pins each branch (Gone / Running-idle /
    //! Running-pending-but-not-stalled / Stalled) so the `!running` guard, the
    //! record-pending gate, and the 30 s × frozen-progress boundary are
    //! mutation-killed.

    use super::*;

    const DEADLINE: Duration = Duration::from_secs(30);
    const DEADLINE_NANOS: u64 = 30 * 1_000_000_000;

    /// `running == false` ⇒ Gone regardless of the other inputs. Pins the `!running`
    /// guard (a `delete !` mutation would return Running for a dead pump).
    #[test]
    fn not_running_is_gone() {
        // Even with a record pending and progress frozen far past the deadline, a
        // pump that has exited (running == false) is Gone — NOT Stalled, NOT Running.
        let now = DEADLINE_NANOS * 10;
        assert_eq!(
            derive_liveness(false, true, 0, now, DEADLINE),
            PumpLiveness::Gone,
            "a pump whose thread exited is Gone (kills `delete !` — which would say Running)"
        );
        assert_eq!(
            derive_liveness(false, false, now, now, DEADLINE),
            PumpLiveness::Gone,
            "running == false is Gone even with no record pending"
        );
    }

    /// `running == true`, no record pending ⇒ Running (idle-but-ready), NEVER Stalled
    /// — no false positive on a quiescent connection even when the progress metric is
    /// ancient.
    #[test]
    fn running_idle_is_running_never_stalled() {
        let now = DEADLINE_NANOS * 10;
        assert_eq!(
            derive_liveness(true, false, 0, now, DEADLINE),
            PumpLiveness::Running,
            "no record pending ⇒ Running, even if progress is ancient (no false positive)"
        );
    }

    /// `running == true`, record pending, progress NOT frozen past the deadline ⇒
    /// Running. Pins the `>=` boundary: exactly-under-deadline is still Running.
    #[test]
    fn running_pending_under_deadline_is_running() {
        let last = 1_000_000_000;
        // Elapsed is one nano UNDER the deadline ⇒ Running.
        let now = last + DEADLINE_NANOS - 1;
        assert_eq!(
            derive_liveness(true, true, last, now, DEADLINE),
            PumpLiveness::Running,
            "a record pending but progress within the deadline ⇒ Running"
        );
    }

    /// `running == true`, record pending, progress frozen AT/PAST the deadline ⇒
    /// Stalled { since: last_progress }. Pins the deadline boundary + the `since`.
    #[test]
    fn running_pending_at_or_past_deadline_is_stalled() {
        let last = 1_000_000_000;
        let since = UnixInstant::from_unix_duration(Duration::from_nanos(last));
        // EXACTLY at the deadline ⇒ Stalled (kills `>= → >` in stall_elapsed).
        let at = last + DEADLINE_NANOS;
        assert_eq!(
            derive_liveness(true, true, last, at, DEADLINE),
            PumpLiveness::Stalled { since },
            "progress frozen exactly the deadline long with a record pending ⇒ Stalled"
        );
        // Past the deadline ⇒ Stalled, carrying the last-progress instant as `since`.
        let past = last + DEADLINE_NANOS * 2;
        assert_eq!(
            derive_liveness(true, true, last, past, DEADLINE),
            PumpLiveness::Stalled { since },
            "progress frozen past the deadline ⇒ Stalled carrying the last-progress instant as since"
        );
    }
}
