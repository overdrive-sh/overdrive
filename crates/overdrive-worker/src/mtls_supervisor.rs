//! `MtlsSupervisor` — the worker's F6 pump-supervision POLICY for the
//! transparent-mTLS proxy (ADR-0069 D-MTLS-10 / SD-4; GH #26; step 04-01).
//!
//! The dataplane adapter (`overdrive-dataplane::mtls`) owns the `Stalled`
//! DERIVATION (SD-2 — the adapter watches the pump's bytes-moved progress metric).
//! The WORKER owns the REACTION (D-MTLS-10): on its reconciler-tick cadence (SD-4)
//! it point-queries `MtlsEnforcement::liveness` for each established connection and,
//! on observing [`PumpLiveness::Stalled`], tears the connection down — teardown +
//! fail-closed reset (close the legs, stop the pumps, reclaim the kTLS state). It
//! does NOT reconnect-in-place (a foreign process cannot resume a kTLS record
//! sequence) and does NOT degrade to a userspace copy loop.
//!
//! This is the converge-on-boot / Bar-1 reconciler shape (`.claude/rules/
//! reconcilers.md`): a point-query each tick, idempotent teardown. The decision
//! reads ONLY the `PumpLiveness` variant the adapter returns — it computes no stall
//! itself, so it reads no clock for the decision; the injected [`Clock`] is held for
//! tick cadence / telemetry timestamping per the SD-4 dispatch.
//!
//! Telemetry (the two F6 events): `mtls.pump.stalled` is emitted per connection
//! observed `Stalled`; `mtls.pump.teardown_on_stall` is emitted per connection torn
//! down in reaction (per allocation).

use std::sync::Arc;

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_enforcement::{
    EnforcedConnection, EnforcedConnectionId, MtlsEnforcement, PumpLiveness,
};

/// The worker's F6 supervisor: point-queries `liveness` per tick and tears down on
/// `Stalled`.
///
/// Holds the `MtlsEnforcement` port (the connections it supervises) and the injected
/// `Clock` (SD-4 cadence; the decision itself is clock-free — it reacts to the
/// adapter-derived `PumpLiveness` variant, never `Instant::now()`).
pub struct MtlsSupervisor {
    enforcement: Arc<dyn MtlsEnforcement>,
    #[allow(
        dead_code,
        reason = "SD-4: injected for tick cadence / telemetry timestamping; the \
        stall DECISION reads only the adapter-derived PumpLiveness variant, never the clock"
    )]
    clock: Arc<dyn Clock>,
}

impl MtlsSupervisor {
    /// Construct the supervisor from its REQUIRED dependencies — the enforcement
    /// port whose connections it supervises and the injected clock (SD-4). Both
    /// mandatory: no builder, no defaulting (`.claude/rules/development.md`
    /// § "Port-trait dependencies").
    #[must_use]
    pub fn new(enforcement: Arc<dyn MtlsEnforcement>, clock: Arc<dyn Clock>) -> Self {
        Self { enforcement, clock }
    }

    /// One supervision tick over `handles` (D-MTLS-10 / SD-4): point-query
    /// `liveness` for each; on `Stalled`, emit `mtls.pump.stalled`, `teardown`
    /// (fail-closed reset), emit `mtls.pump.teardown_on_stall`, and collect the id.
    /// Returns the ids of the connections torn down this tick.
    ///
    /// `Running` and `Gone` connections are left untouched — a purely-idle (Running)
    /// connection is NEVER torn down (no false positives), and a `Gone` connection is
    /// already reclaimed. Teardown is idempotent, so a connection that raced to `Gone`
    /// between the query and the teardown is a harmless no-op.
    pub async fn supervise_tick(
        &self,
        handles: &[EnforcedConnection],
    ) -> Vec<EnforcedConnectionId> {
        let mut torn_down = Vec::new();
        for handle in handles {
            if matches!(self.enforcement.liveness(handle), PumpLiveness::Stalled { .. }) {
                let alloc = handle.id().alloc();
                tracing::warn!(
                    name: "mtls.pump.stalled",
                    connection = %handle.id(),
                    alloc = %alloc,
                    "transparent-mTLS pump stalled (no progress past pump_stall_deadline with a \
                     record pending); tearing the connection down (F6)"
                );
                // Teardown + fail-closed reset. Idempotent — a racing Gone is Ok.
                let teardown = self.enforcement.teardown(handle.clone()).await;
                tracing::warn!(
                    name: "mtls.pump.teardown_on_stall",
                    connection = %handle.id(),
                    alloc = %alloc,
                    ok = teardown.is_ok(),
                    "transparent-mTLS connection torn down on stall (F6 supervision reaction)"
                );
                torn_down.push(handle.id().clone());
            }
        }
        torn_down
    }
}
