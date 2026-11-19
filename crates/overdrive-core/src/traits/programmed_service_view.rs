//! [`ProgrammedServiceView`] — the read-port over the programmed service map
//! (D-20, ADR-0324).
//!
//! `service-map-hydrator`'s `actual` is the resource it manages: the service
//! map the dataplane has programmed. The port returns the fingerprint of the
//! backend set the dataplane last applied for a frontend, not the backend set.
//! The `Dataplane` trait carries no read operation (ADR-0324 rejects adding
//! one), so this is a separate read-only port, reached only through
//! [`HydrationContext`](crate::reconcilers::HydrationContext).

use async_trait::async_trait;

use crate::dataplane::ServiceFrontend;
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::traits::dataplane::DataplaneError;

/// The read-port over the programmed service map (D-20, ADR-0324).
#[async_trait]
pub trait ProgrammedServiceView: Send + Sync {
    /// The fingerprint of the backend set the dataplane last applied for
    /// `frontend`, or `None` when no slot is programmed for it.
    ///
    /// # Preconditions
    ///
    /// `frontend` is V4 by construction (`ServiceFrontend::new`). The impl is
    /// bound to the same dataplane instance the action shim writes through
    /// `Dataplane::update_service`; a view over another instance reads a
    /// different resource. The composition root supplies both from one
    /// instance.
    ///
    /// # Postconditions
    ///
    /// On `Ok(Some(fp))`, `fp` is the fingerprint of the backend slice of the
    /// last successful non-empty `update_service` for the exact key
    /// `(frontend.vip, frontend.port, frontend.proto)`, as stored in the
    /// resource that is the service map's table for that key. The host impl
    /// reads it from the inner array the `SERVICE_MAP` outer slot currently
    /// points to: outer lookup (the inner map id), `BPF_MAP_GET_FD_BY_ID`,
    /// then the two element lookups at inner slots 16,381 and 16,382,
    /// recombined as `low | high << 32`. The sim impl reads the fingerprint its
    /// stored service carries. No in-process tracker, receipt or cache is the
    /// source. The call mutates nothing. The result is one state between whole
    /// `update_service` calls, never a mix of two: the host takes the
    /// apply-commit guard for read for the whole sequence.
    ///
    /// # Edge cases
    ///
    /// - `Ok(None)` means no slot is programmed for the key.
    ///   `update_service(frontend, [])` withdraws the slot, so "never
    ///   programmed", "withdrawn" and "lost with the maps" are
    ///   indistinguishable by design. A frontend never programmed is
    ///   `Ok(None)`, not an error.
    /// - Other ports and protocols on the same VIP are independent keys.
    /// - A failed `update_service` (including a set with an IPv6 backend,
    ///   which is refused before any write) leaves the prior fingerprint in
    ///   place.
    /// - A failed kernel read, including `ENOENT` on the inner map id,
    ///   returns `Err(DataplaneError::LoadFailed)` and is never reported as
    ///   `Ok(None)`.
    /// - The fingerprint covers every backend of the applied slice with its
    ///   allocation, address, weight and health, and the VIP. It does not
    ///   report whether `BACKEND_MAP` still holds each entry.
    ///
    /// # Observable invariants
    ///
    /// 1. After a successful `update_service(f, B)` with `B` non-empty and no
    ///    later write, the read equals `Some(fingerprint(&f.vip, &B))`.
    /// 2. After `update_service(f, [])` it is `None`.
    /// 3. A freshly constructed dataplane instance reads `None` for every
    ///    frontend (process restart, reboot).
    /// 4. Two reads with no write between are equal.
    /// 5. `SimDataplane` and `EbpfDataplane` return identical results for the
    ///    same call sequence.
    async fn programmed_fingerprint(
        &self,
        frontend: &ServiceFrontend,
    ) -> Result<Option<BackendSetFingerprint>, DataplaneError>;
}
