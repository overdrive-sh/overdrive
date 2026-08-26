//! [`ListenerFacts`] — the per-`ServiceId` listener-fact read port (ADR-0086 D5).
//!
//! One of the four narrow driven read-ports the reconciler hydration boundary
//! reads (ADR-0086 D5). The `ServiceMapHydrator`'s `hydrate_desired` sources the
//! per-listener `(port, protocol)` fact through this port — an O(1) keyed read
//! per `service_backends` row rather than an O(S²) per-tick cluster scan
//! (ADR-0062 § Decision (3)). The production impl is `ListenerFactStore`
//! (control-plane, boot-rebuilt + edge-maintained); the DST impl is
//! `SimListenerFacts` (`overdrive-sim`, step 02-05), which makes this hydration
//! surface injectable for the first time (ADR-0086 D8).
//!
//! Per `.claude/rules/development.md` § "Port-trait dependencies" the reconciler
//! reads through `&dyn ListenerFacts` on the [`HydrationContext`], never a
//! concrete `AppState` field. Per § "Trait definitions specify behavior, not
//! just signature" the method rustdoc below is the SSOT the sim adapter and the
//! DST equivalence test enforce against every impl.
//!
//! [`HydrationContext`]: crate::reconcilers::HydrationContext

use async_trait::async_trait;

use crate::id::ServiceId;
use crate::traits::observation_store::ListenerRow;

/// The per-`ServiceId` listener-fact read port (ADR-0086 D5).
///
/// A **driven, read-only** projection: the reconciler calls out to fetch the
/// listener fact for a service; there is no write method (the read/write split
/// of Principle 12 holds by construction). Async because the production impl
/// (`ListenerFactStore`) is held behind a `tokio::sync::Mutex` on `AppState`,
/// so the read crosses an `.await`.
#[async_trait]
pub trait ListenerFacts: Send + Sync {
    /// The boot-rebuilt + edge-maintained listener fact for `service_id`, or
    /// `None` when no fact is held for it.
    ///
    /// # Preconditions
    /// None. Any [`ServiceId`] is a valid query — a service that was never
    /// projected into the store, or whose fact was removed, reads as `None`.
    ///
    /// # Postconditions
    /// On `Some`, returns an **owned clone** of the [`ListenerRow`] currently
    /// held for `service_id` (the `(vip, port, protocol)` listener projection).
    /// The store is unchanged — this is a pure read, mutates nothing. The
    /// caller holds no lock after the call returns (the impl clones the small
    /// value and drops any interior guard before returning).
    ///
    /// # Edge cases
    /// `None` is **explicit absence** — never an error, never a
    /// default-bearing fact. The caller (the `ServiceMapHydrator`) MUST treat
    /// `None` as "skip this service": it may NOT synthesise a listener fact and
    /// in particular may NEVER default the protocol to `Proto::Tcp` (ADR-0060
    /// C3). A service co-locating `tcp/53` + `udp/53` is represented by its own
    /// derived `ServiceId` per listener, so a per-`ServiceId` read is
    /// unambiguous.
    ///
    /// # Observable invariants
    /// This is a **point read**: it carries NO ordering guarantee across
    /// distinct keys, and two reads of the same `service_id` with no intervening
    /// store mutation return equal values. `fact_for(service_id)` reflects
    /// exactly the fact the store currently holds for that key: `Some(row)`
    /// after the fact is projected, `None` after it is removed. It never
    /// mutates the store.
    async fn fact_for(&self, service_id: ServiceId) -> Option<ListenerRow>;
}
