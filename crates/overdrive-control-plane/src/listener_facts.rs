//! In-memory listener-fact projection (ADR-0062) — the hydration-layer
//! efficiency fix that replaces the `ServiceMapHydrator`'s
//! O(S²)-per-tick cluster scan (`gather_service_listener_facts`) with
//! an O(1) keyed read off a maintained in-memory view.
//!
//! The [`ListenerFactStore`] is an **in-memory projection of the intent
//! SSOT** — it persists nothing. Boot re-derives the whole view from
//! the `IntentStore` + allocator memo via [`ListenerFactStore::rebuild_from_intent`],
//! which satisfies `.claude/rules/development.md` § "Persist inputs, not
//! derived state": the inputs (Service intents + allocator-issued VIPs)
//! are the SSOT, and the store is a recompute-on-boot cache of the
//! projection over them.
//!
//! # Shape
//!
//! Two `BTreeMap`s (NOT `HashMap` — `.claude/rules/development.md`
//! § "Ordered-collection choice"; both maps are iterated by the boot
//! rebuild and observed by DST invariants, so iteration order must be
//! seed-deterministic):
//!
//! * **primary** `ServiceId -> ListenerRow` — the read-path key. The
//!   `ServiceMapHydrator` resolves `service_id` from its target and
//!   keys `desired` by that id, so `ServiceId` is the natural read key.
//! * **secondary** `WorkloadId -> Vec<ServiceId>` — a cleanup index.
//!   It exists ONLY because the stop handler holds a [`WorkloadId`],
//!   not the per-listener [`ServiceId`]s; on workload removal the index
//!   tells [`ListenerFactStore::remove_workload`] exactly which primary
//!   entries to evict.
//!
//! Per listener the store derives `ServiceId::derive(&vip, port,
//! protocol, "service-map")` — the identical derivation
//! `hydrate_bridge_desired_listeners` and `gather_service_listener_facts`
//! use, so the read-path key matches the hydrator's projection. The
//! `protocol` axis (ADR-0040 companion revision 2026-06-03 / ADR-0052
//! § 1) splits two listeners on the same `(vip, port)` but different L4
//! protocol (the canonical CoreDNS `tcp/53` + `udp/53` case) into two
//! distinct primary entries instead of colliding.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use overdrive_core::aggregate::{Listener, WorkloadIntent};
use overdrive_core::id::{ServiceId, ServiceVip, WorkloadId};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::ListenerRow;
use overdrive_dataplane::allocators::PersistentServiceVipAllocator;
use overdrive_store_local::LocalIntentStore;

use crate::reconciler_runtime::ConvergenceError;

/// `purpose` namespacing token passed to [`ServiceId::derive`]. The
/// single canonical value the bridge / hydrator path uses; keeping it
/// as one constant guarantees the store's derived ids match the
/// hydrator's.
const SERVICE_MAP_PURPOSE: &str = "service-map";

/// In-memory projection of per-listener protocol facts, keyed for O(1)
/// read by the `ServiceMapHydrator`.
///
/// See the module docs for the rationale behind the two-map shape. The
/// store owns no I/O and persists nothing — it is rebuilt from the
/// intent SSOT at boot via [`Self::rebuild_from_intent`] and maintained
/// incrementally via [`Self::upsert`] / [`Self::remove_workload`] on
/// the submit / stop paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListenerFactStore {
    /// Read-path index: derived `ServiceId` → the listener's
    /// `(port, protocol, vip)` row. One entry per listener.
    primary: BTreeMap<ServiceId, ListenerRow>,
    /// Cleanup index: workload → the `ServiceId`s its listeners
    /// derived, in listener (declaration) order. Drives eviction in
    /// [`Self::remove_workload`].
    secondary: BTreeMap<WorkloadId, Vec<ServiceId>>,
}

impl ListenerFactStore {
    /// Construct an empty store. Equivalent to [`Default::default`];
    /// provided as a named constructor for call-site clarity.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or replace) the listener facts for `workload_id` at the
    /// allocator-issued `vip`.
    ///
    /// For EACH listener in `listeners` (declaration order): derives
    /// `ServiceId::derive(vip, listener.port, listener.protocol,
    /// "service-map")`, inserts a `ListenerRow { vip: Some(vip), port,
    /// protocol }` into the primary map, and appends the derived
    /// `ServiceId` to the workload's `Vec` in the secondary map. The
    /// proto axis (ADR-0040 companion / ADR-0052 § 1) keeps two
    /// same-`(vip, port)` listeners that differ only in protocol as two
    /// distinct primary entries.
    ///
    /// # Postconditions
    ///
    /// After return, `self.fact_for(&id)` returns `Some(row)` for every
    /// listener's derived `id`, and the workload's secondary `Vec` holds
    /// exactly those `ServiceId`s in listener order. An empty
    /// `listeners` slice records an empty secondary `Vec` for the
    /// workload and inserts no primary entries.
    pub fn upsert(&mut self, workload_id: WorkloadId, vip: &ServiceVip, listeners: &[Listener]) {
        let mut ids = Vec::with_capacity(listeners.len());
        for listener in listeners {
            let service_id =
                ServiceId::derive(vip, listener.port, listener.protocol, SERVICE_MAP_PURPOSE);
            // Defensive invariant guard (the original bug report's ask,
            // reframed post-widening). Two listeners IN THIS SAME upsert
            // call deriving the SAME ServiceId would silently overwrite
            // one another (last-writer-wins). Admission
            // (`ServiceV1::from_submit`) already rejects duplicate
            // `(port, protocol)` listeners, so a collision here is a
            // STRUCTURAL INVARIANT VIOLATION — a malformed listener set
            // that bypassed admission — not a legitimate runtime state.
            //
            // The check is scoped to THIS call's accumulated `ids`, NOT
            // `self.primary`: `upsert` is idempotent across calls (a
            // re-upsert of the same workload re-inserts the same
            // ServiceIds), so consulting `self.primary` would false-fire
            // on every legitimate re-upsert. Only a duplicate WITHIN one
            // `listeners` slice is the invariant violation.
            //
            // `debug_assert!` fails loud in tests / debug builds; the
            // `tracing::warn!` surfaces it in release without panicking
            // the control plane (last-writer-wins on the duplicate).
            if ids.contains(&service_id) {
                debug_assert!(
                    !ids.contains(&service_id),
                    "listener_facts: two listeners of workload {workload_id} derived the same \
                     ServiceId {service_id} within one upsert — admission must reject duplicate \
                     (port, proto) listeners (ServiceV1::from_submit); a collision here is a \
                     structural invariant violation"
                );
                tracing::warn!(
                    name: "listener_facts.service_id_collision",
                    workload_id = %workload_id,
                    service_id = %service_id,
                    port = listener.port.get(),
                    protocol = %listener.protocol,
                    "two listeners in one upsert derived the same ServiceId; admission should \
                     reject duplicate (port, proto) listeners — this overwrites the prior fact",
                );
            }
            self.primary.insert(
                service_id,
                ListenerRow { port: listener.port, protocol: listener.protocol, vip: Some(*vip) },
            );
            ids.push(service_id);
        }
        self.secondary.insert(workload_id, ids);
    }

    /// Evict every listener fact recorded for `workload_id`.
    ///
    /// Looks up the workload's `Vec<ServiceId>` in the secondary index,
    /// removes each from the primary map, and drops the secondary entry.
    /// Other workloads' facts are untouched.
    ///
    /// # Postconditions
    ///
    /// A missing `workload_id` is a no-op (idempotent). After return,
    /// `self.fact_for(&id)` returns `None` for every `id` that was
    /// derived from `workload_id`'s listeners, and the workload has no
    /// secondary entry.
    pub fn remove_workload(&mut self, workload_id: &WorkloadId) {
        if let Some(ids) = self.secondary.remove(workload_id) {
            for id in &ids {
                self.primary.remove(id);
            }
        }
    }

    /// O(1) keyed read of the listener fact for `service_id`.
    ///
    /// Returns a clone of the small `ListenerRow` value when present,
    /// `None` otherwise. This is the read path the `ServiceMapHydrator`
    /// uses in place of the O(S²) cluster scan.
    #[must_use]
    pub fn fact_for(&self, service_id: ServiceId) -> Option<ListenerRow> {
        // `ServiceId` is an 8-byte `Copy` newtype — taken by value per
        // clippy::trivially_copy_pass_by_ref; the hydrator passes its
        // `row.service_id` directly.
        self.primary.get(&service_id).copied()
    }

    /// Rebuild the whole store from the intent SSOT — the boot-time
    /// re-derivation path (the store persists nothing).
    ///
    /// Scans the `workloads/` intent prefix, decodes each
    /// `WorkloadIntent::Service(_)`, joins the allocator-issued VIP, and
    /// populates BOTH maps for every listener. Non-Service intents,
    /// Service intents whose VIP the allocator has not yet issued, and
    /// the `workloads/<id>/stop` + `workloads/<id>/kind` sub-keys
    /// contribute nothing.
    ///
    /// This relocates the projection body of
    /// `reconciler_runtime::gather_service_listener_facts` (the
    /// per-tick caller of which is removed in step 01-04) onto the
    /// maintained-view shape: same scan, same `(vip, port, protocol)`
    /// derivation, but populating the keyed store rather than returning
    /// a flat `Vec<ListenerRow>`.
    ///
    /// # Parameters
    ///
    /// Takes the three boot inputs directly — the `IntentStore`, the
    /// redb path (for decode-failure remediation messages), and the
    /// allocator — RATHER than a full `&AppState`. The boot wiring
    /// constructs the store BEFORE assembling `AppState` (so the
    /// rebuilt store can be threaded into the constructor as a mandatory
    /// field), which makes a `&AppState` parameter a construction cycle.
    /// All three inputs are available at the wiring site immediately
    /// after the allocator's `bulk_load`. ORDERING IS LOAD-BEARING: the
    /// allocator must already be bulk-loaded so its `get()` resolves the
    /// per-Service VIP memo — a Service whose VIP the allocator has not
    /// issued is skipped.
    ///
    /// # Errors
    ///
    /// [`ConvergenceError::IntentRead`] when the `IntentStore` scan
    /// fails. A per-record decode failure or a UTF-8-invalid key is
    /// skipped (not fatal) — it is not a listener fact.
    pub async fn rebuild_from_intent(
        store: &Arc<LocalIntentStore>,
        intent_redb_path: &Path,
        allocator: &Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    ) -> Result<Self, ConvergenceError> {
        let rows = store
            .scan_prefix(b"workloads/")
            .await
            .map_err(|e| ConvergenceError::IntentRead(e.to_string()))?;

        let mut facts = Self::new();
        for (key_bytes, value_bytes) in rows {
            // Only the canonical `workloads/<id>` records carry the
            // intent payload — skip the `workloads/<id>/stop` and
            // `workloads/<id>/kind` sub-keys.
            let Ok(key_str) = std::str::from_utf8(&key_bytes) else { continue };
            let suffix = &key_str["workloads/".len()..];
            // Equivalent mutant note (`||` → `&&`): with `&&` the guard
            // never fires (an empty suffix cannot contain '/'), so the
            // `workloads/<id>/stop` and `workloads/<id>/kind` sub-keys are
            // not fast-skipped here — but they then fail the
            // `WorkloadIntent::from_store_bytes` decode + `Service(_)` match
            // below (a stop sentinel / 1-byte kind discriminator does not
            // bytecheck as a Service envelope) and `continue` regardless.
            // The canonical `workloads/<id>` key (non-empty, no '/') is
            // never skipped under either operator. Facts are byte-identical;
            // no test can distinguish the variants. cargo-mutants only
            // honors the marker on the immediately-adjacent line, so the
            // bare token must sit directly above the `if`.
            // mutants: skip
            if suffix.is_empty() || suffix.contains('/') {
                continue;
            }

            // A non-intent payload under the prefix (or a decode
            // failure) is not a listener fact — skip it.
            let Ok(intent) = WorkloadIntent::from_store_bytes(
                value_bytes.as_ref(),
                intent_redb_path,
                Some(key_str),
            ) else {
                continue;
            };
            let WorkloadIntent::Service(service_v1) = &intent else { continue };

            let Ok(spec_digest) = intent.spec_digest() else { continue };
            let digest_bytes: [u8; 32] = *spec_digest.as_bytes();
            // Lock discipline (`.claude/rules/development.md`
            // § "Concurrency & async"): acquire the allocator guard,
            // read the synchronous `get()`, and DROP it before any
            // subsequent `.await`. Mirrors
            // `hydrate_bridge_desired_listeners`.
            let assigned_vip_opt = {
                let guard = allocator.lock().await;
                let vip = guard.get(&digest_bytes);
                drop(guard);
                vip
            };
            let Some(assigned_vip) = assigned_vip_opt else { continue };

            facts.upsert(service_v1.id.clone(), &assigned_vip, &service_v1.listeners);
        }
        Ok(facts)
    }
}

/// Test-only structural snapshot of a [`ListenerFactStore`]'s two
/// internal maps, materialised as `Ord`-ordered `Vec`s.
///
/// Exists so in-crate unit tests assert on the store's observable
/// shape through ONE `pub(crate)` accessor rather than reaching into
/// the private `primary` / `secondary` fields directly. Routing the
/// asserts through [`ListenerFactStore::snapshot`] keeps the tests
/// coupled to the store's observable projection (the maps' key/value
/// contents in deterministic order), not to the field names — a rename
/// of either field stays GREEN, and the production surface is not
/// widened (`#[cfg(test)]` strips this from every non-test build).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListenerFactSnapshot {
    /// `primary` flattened to `(ServiceId, ListenerRow)` pairs in
    /// `BTreeMap` (`Ord`-on-`ServiceId`) iteration order.
    pub service_facts: Vec<(ServiceId, ListenerRow)>,
    /// `secondary` flattened to `(WorkloadId, Vec<ServiceId>)` pairs in
    /// `BTreeMap` (`Ord`-on-`WorkloadId`) iteration order.
    pub workload_index: Vec<(WorkloadId, Vec<ServiceId>)>,
}

#[cfg(test)]
impl ListenerFactStore {
    /// Materialise a [`ListenerFactSnapshot`] of both internal maps.
    ///
    /// Clones each map into an `Ord`-ordered `Vec` so a test can assert
    /// on the store's observable shape (and compare two stores for
    /// byte-equivalence) without reading the private fields.
    pub(crate) fn snapshot(&self) -> ListenerFactSnapshot {
        ListenerFactSnapshot {
            service_facts: self.primary.iter().map(|(id, row)| (*id, *row)).collect(),
            workload_index: self
                .secondary
                .iter()
                .map(|(wid, ids)| (wid.clone(), ids.clone()))
                .collect(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::num::NonZeroU16;
    use std::str::FromStr;
    use std::sync::Arc;

    use overdrive_core::aggregate::{
        DriverInput, ExecInput, IntentKey, Listener, ResourcesInput, WorkloadIntent, WorkloadKind,
    };
    use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
    use overdrive_core::dataplane::backend_key::Proto;
    use overdrive_core::id::{NodeId, ServiceId, ServiceVip, WorkloadId};
    use overdrive_core::traits::intent_store::IntentStore;
    use overdrive_core::traits::observation_store::{ListenerRow, ObservationStore};
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::dataplane::SimDataplane;
    use overdrive_sim::adapters::driver::SimDriver;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_store_local::LocalIntentStore;
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;
    use crate::AppState;
    use crate::reconciler_runtime::ReconcilerRuntime;
    use overdrive_core::traits::driver::DriverType;

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn vip(addr: &str) -> ServiceVip {
        ServiceVip::new(addr.parse().expect("valid ip")).expect("valid vip")
    }

    fn listener(port: u16, proto: Proto) -> Listener {
        Listener { port: NonZeroU16::new(port).expect("non-zero port"), protocol: proto }
    }

    fn workload(name: &str) -> WorkloadId {
        WorkloadId::new(name).expect("valid workload id")
    }

    fn node_id(name: &str) -> NodeId {
        NodeId::from_str(name).expect("valid NodeId")
    }

    fn proto_str(p: Proto) -> &'static str {
        match p {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }

    fn derived(v: &ServiceVip, port: u16, proto: Proto) -> (ServiceId, ListenerRow) {
        let nz = NonZeroU16::new(port).expect("non-zero port");
        (
            ServiceId::derive(v, nz, proto, super::SERVICE_MAP_PURPOSE),
            ListenerRow { port: nz, protocol: proto, vip: Some(*v) },
        )
    }

    // -----------------------------------------------------------------
    // AppState fixture (mirrors service_backends_hydrate_desired.rs)
    // -----------------------------------------------------------------

    fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
        let runtime =
            ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
        let store_path = tmp.path().join("intent.redb");
        let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
        let driver: Arc<dyn overdrive_core::traits::driver::Driver> =
            Arc::new(SimDriver::new(DriverType::Exec));
        let allocator = crate::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
        // The fixture seeds intent AFTER construction, so the
        // boot-rebuild at this point would be empty regardless — pass a
        // fresh store. Tests that exercise the boot-rebuild path call
        // `rebuild_from_intent` explicitly after seeding (mirroring the
        // production wiring's post-`bulk_load` rebuild).
        let listener_facts = Arc::new(tokio::sync::Mutex::new(ListenerFactStore::new()));
        AppState::new(
            store,
            store_path,
            obs,
            Arc::new(runtime),
            driver,
            Arc::new(SimClock::new()),
            Arc::new(SimDataplane::new()),
            node_id("writer-1"),
            allocator,
            listener_facts,
            std::net::Ipv4Addr::LOCALHOST,
        )
    }

    /// Persist a Service intent with the given listeners and allocate
    /// its VIP through the production allocator. Returns the
    /// allocator-issued VIP.
    async fn persist_service_and_allocate_vip(
        state: &AppState,
        id: &str,
        listeners: &[(u16, Proto)],
    ) -> ServiceVip {
        let listener_inputs: Vec<ListenerInput> = listeners
            .iter()
            .map(|(port, proto)| ListenerInput { port: *port, protocol: proto_str(*proto).into() })
            .collect();
        let svc = overdrive_core::aggregate::ServiceV1::from_submit(ServiceSpecInput {
            id: id.to_string(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/serve".to_string(),
                args: vec![],
            }),
            listeners: listener_inputs,
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        })
        .expect("valid service spec");
        persist_intent_and_kind(
            state,
            WorkloadIntent::Service(svc.clone()),
            &svc.id,
            WorkloadKind::Service,
        )
        .await;

        let digest = WorkloadIntent::Service(svc).spec_digest().expect("spec_digest");
        let bytes: [u8; 32] = *digest.as_bytes();
        let mut guard = state.allocator.lock().await;
        let vip = guard.allocate(bytes).await.expect("allocate vip");
        drop(guard);
        vip
    }

    /// Persist a Job intent (no VIP allocation) — a negative case for
    /// rebuild: Job intents contribute no listener facts.
    async fn persist_job(state: &AppState, id: &str) {
        let job = overdrive_core::aggregate::JobV1::from_submit(
            overdrive_core::aggregate::JobSpecInput {
                id: id.to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/run".to_string(),
                    args: vec![],
                }),
            },
        )
        .expect("valid job spec");
        let wid = job.id.clone();
        persist_intent_and_kind(state, WorkloadIntent::Job(job), &wid, WorkloadKind::Job).await;
    }

    async fn persist_intent_and_kind(
        state: &AppState,
        intent: WorkloadIntent,
        id: &WorkloadId,
        kind: WorkloadKind,
    ) {
        let key = IntentKey::for_workload(id);
        let archived = intent.archive_for_store().expect("rkyv archive");
        state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
        let kind_key = IntentKey::for_workload_kind(id);
        state.store.put(kind_key.as_bytes(), &[kind.discriminator_byte()]).await.expect("put kind");
    }

    // -----------------------------------------------------------------
    // U1 — upsert: one primary entry per listener, secondary Vec in order
    // -----------------------------------------------------------------

    #[test]
    fn upsert_multi_listener_creates_one_primary_entry_per_listener() {
        let v = vip("10.96.0.7");
        let listeners =
            vec![listener(80, Proto::Tcp), listener(443, Proto::Tcp), listener(53, Proto::Udp)];
        let wid = workload("web");

        let mut store = ListenerFactStore::new();
        store.upsert(wid.clone(), &v, &listeners);

        // One primary entry per listener, keyed by the derived ServiceId.
        for (port, proto) in [(80, Proto::Tcp), (443, Proto::Tcp), (53, Proto::Udp)] {
            let (sid, expected_row) = derived(&v, port, proto);
            assert_eq!(
                store.fact_for(sid),
                Some(expected_row),
                "primary entry for port {port} must carry the listener's row"
            );
        }

        // Secondary Vec holds the derived ids in listener order.
        let expected_ids: Vec<ServiceId> = listeners
            .iter()
            .map(|l| ServiceId::derive(&v, l.port, l.protocol, super::SERVICE_MAP_PURPOSE))
            .collect();
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.workload_index.iter().find(|(w, _)| w == &wid).map(|(_, ids)| ids),
            Some(&expected_ids),
            "secondary Vec must list every listener's ServiceId in declaration order"
        );
        assert_eq!(snapshot.service_facts.len(), 3, "exactly three primary entries");
    }

    // -----------------------------------------------------------------
    // REG — same (vip, port), different proto → two distinct facts
    //
    // The reported bug: two listeners on one VIP sharing a port but
    // differing in L4 protocol (the canonical CoreDNS tcp/53 + udp/53
    // case) must NOT collide in the primary map. Before the Model A
    // proto-widening of `ServiceId::derive` (ADR-0040 companion
    // revision 2026-06-03 / ADR-0052 § 1) both listeners derived the
    // SAME ServiceId and the udp `insert` silently overwrote the tcp
    // one (last-writer-wins) — `primary.len()` was 1, and `fact_for`
    // returned a single UDP row. Post-widening: two distinct entries,
    // each `fact_for` resolving its own proto.
    // -----------------------------------------------------------------

    #[test]
    fn upsert_same_port_distinct_proto_yields_two_distinct_facts() {
        let v = vip("10.96.0.53");
        // CoreDNS-shape: one VIP, port 53, both tcp and udp.
        let listeners = vec![listener(53, Proto::Tcp), listener(53, Proto::Udp)];
        let wid = workload("coredns");

        let mut store = ListenerFactStore::new();
        store.upsert(wid.clone(), &v, &listeners);

        // Two distinct primary entries — the proto axis splits the
        // otherwise-identical (vip, port) into two ServiceIds.
        let (sid_tcp, row_tcp) = derived(&v, 53, Proto::Tcp);
        let (sid_udp, row_udp) = derived(&v, 53, Proto::Udp);
        assert_ne!(sid_tcp, sid_udp, "tcp/53 and udp/53 must derive distinct ServiceIds");

        assert_eq!(
            store.fact_for(sid_tcp),
            Some(row_tcp),
            "fact_for(tcp/53) resolves the TCP listener's row"
        );
        assert_eq!(
            store.fact_for(sid_udp),
            Some(row_udp),
            "fact_for(udp/53) resolves the UDP listener's row"
        );

        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.service_facts.len(),
            2,
            "both listeners populate the primary map — no last-writer-wins collapse"
        );
        // The secondary index lists BOTH derived ids for the workload.
        assert_eq!(
            snapshot.workload_index.iter().find(|(w, _)| w == &wid).map(|(_, ids)| ids.len()),
            Some(2),
            "secondary Vec lists both ServiceIds for the workload"
        );
    }

    /// A re-upsert of the SAME workload (same listeners) is idempotent —
    /// the collision guard checks duplicates WITHIN one call, NOT across
    /// calls, so re-upserting must NOT fire it. (Guards against the
    /// regression where the guard consulted `self.primary` and
    /// false-fired on every legitimate re-upsert — the
    /// lock-discipline-under-contention acceptance test exercises this
    /// repeatedly.)
    #[test]
    fn re_upsert_same_workload_is_idempotent_and_does_not_trip_collision_guard() {
        let v = vip("10.96.0.7");
        let listeners = vec![listener(80, Proto::Tcp), listener(53, Proto::Udp)];
        let wid = workload("web");

        let mut store = ListenerFactStore::new();
        store.upsert(wid.clone(), &v, &listeners);
        let after_first = store.snapshot();
        // Re-upsert the SAME workload + listeners — must be a no-op
        // delta (idempotent) and must NOT panic via the guard.
        store.upsert(wid, &v, &listeners);
        let after_second = store.snapshot();

        assert_eq!(after_first, after_second, "re-upsert of same workload is idempotent");
        assert_eq!(after_second.service_facts.len(), 2, "still exactly two distinct facts");
    }

    /// Defensive-invariant guard: two listeners with the SAME
    /// `(port, protocol)` in ONE `upsert` call derive the same
    /// `ServiceId`. Admission (`ServiceV1::from_submit`) rejects such a
    /// duplicate, so reaching `upsert` with one is a STRUCTURAL
    /// INVARIANT VIOLATION — the guard's `debug_assert!` fires in debug
    /// / test builds. We construct the malformed `Listener` set directly
    /// (bypassing admission) to exercise the guard.
    #[test]
    #[should_panic(expected = "derived the same ServiceId")]
    fn duplicate_port_proto_listeners_in_one_upsert_trip_collision_guard() {
        let v = vip("10.96.0.7");
        // Two IDENTICAL (port, proto) listeners — admission would reject
        // this, but a malformed set that bypasses admission must trip the
        // guard rather than silently collapse to one fact.
        let listeners = vec![listener(53, Proto::Udp), listener(53, Proto::Udp)];

        let mut store = ListenerFactStore::new();
        store.upsert(workload("malformed"), &v, &listeners);
    }

    // -----------------------------------------------------------------
    // U2 — remove_workload evicts only the target's ServiceIds
    // -----------------------------------------------------------------

    #[test]
    fn remove_workload_evicts_only_target_service_ids() {
        let vip_web = vip("10.96.0.7");
        let vip_api = vip("10.96.0.8");
        let mut store = ListenerFactStore::new();
        store.upsert(workload("web"), &vip_web, &[listener(80, Proto::Tcp)]);
        store.upsert(workload("api"), &vip_api, &[listener(9000, Proto::Tcp)]);

        store.remove_workload(&workload("web"));

        // web's id is gone; api's survives.
        let (web_sid, _) = derived(&vip_web, 80, Proto::Tcp);
        let (api_sid, api_row) = derived(&vip_api, 9000, Proto::Tcp);
        assert_eq!(store.fact_for(web_sid), None, "web's fact must be evicted");
        assert_eq!(store.fact_for(api_sid), Some(api_row), "api's fact must be untouched");
        let snapshot = store.snapshot();
        assert!(
            !snapshot.workload_index.iter().any(|(w, _)| w == &workload("web")),
            "web's secondary entry dropped"
        );
        assert!(
            snapshot.workload_index.iter().any(|(w, _)| w == &workload("api")),
            "api's secondary entry retained"
        );

        // Removing an absent workload is a no-op.
        store.remove_workload(&workload("does-not-exist"));
        assert_eq!(store.fact_for(api_sid), Some(api_row), "no-op removal leaves api intact");
    }

    // -----------------------------------------------------------------
    // U3 — fact_for: Some for present, None for absent
    // -----------------------------------------------------------------

    #[test]
    fn fact_for_returns_row_for_present_and_none_for_absent() {
        let v = vip("10.96.0.7");
        let mut store = ListenerFactStore::new();
        store.upsert(workload("web"), &v, &[listener(80, Proto::Tcp)]);

        let (present, present_row) = derived(&v, 80, Proto::Tcp);
        let (absent, _) = derived(&v, 9999, Proto::Udp);
        assert_eq!(store.fact_for(present), Some(present_row), "present id returns Some");
        assert_eq!(store.fact_for(absent), None, "absent id returns None");
    }

    // -----------------------------------------------------------------
    // U4 — rebuild projects Service intents (with VIP) only
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn rebuild_from_intent_projects_service_intents_only() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        // (a) Service WITH a VIP → contributes its listeners.
        let vip_web =
            persist_service_and_allocate_vip(&state, "web", &[(80, Proto::Tcp), (53, Proto::Udp)])
                .await;
        // (b) Service WITHOUT a VIP allocation → contributes nothing.
        {
            let svc = overdrive_core::aggregate::ServiceV1::from_submit(ServiceSpecInput {
                id: "novip".to_string(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/serve".to_string(),
                    args: vec![],
                }),
                listeners: vec![ListenerInput { port: 7000, protocol: "tcp".into() }],
                startup_probes: vec![],
                readiness_probes: vec![],
                liveness_probes: vec![],
            })
            .expect("valid service");
            let wid = svc.id.clone();
            persist_intent_and_kind(
                &state,
                WorkloadIntent::Service(svc),
                &wid,
                WorkloadKind::Service,
            )
            .await;
        }
        // (c) Job intent → contributes nothing.
        persist_job(&state, "batch").await;

        let store = ListenerFactStore::rebuild_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.allocator,
        )
        .await
        .expect("rebuild");

        // Only web's two listeners are present.
        let (web_tcp, web_tcp_row) = derived(&vip_web, 80, Proto::Tcp);
        let (web_udp, web_udp_row) = derived(&vip_web, 53, Proto::Udp);
        assert_eq!(store.fact_for(web_tcp), Some(web_tcp_row));
        assert_eq!(store.fact_for(web_udp), Some(web_udp_row));
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.service_facts.len(),
            2,
            "only the VIP'd Service's listeners contribute"
        );
        assert_eq!(
            snapshot.workload_index.iter().map(|(w, _)| w.clone()).collect::<Vec<_>>(),
            vec![workload("web")],
            "only web has a secondary entry; novip + batch contribute nothing"
        );
    }

    // -----------------------------------------------------------------
    // U5 — iteration order is seed-deterministic (BTreeMap, no HashMap)
    // -----------------------------------------------------------------

    #[test]
    fn listener_fact_store_iteration_order_is_seed_deterministic() {
        let v = vip("10.96.0.7");
        let mut store = ListenerFactStore::new();
        store.upsert(
            workload("web"),
            &v,
            &[listener(80, Proto::Tcp), listener(443, Proto::Tcp), listener(53, Proto::Udp)],
        );

        // BTreeMap iterates in Ord(ServiceId) order — snapshot twice and
        // confirm identical, ascending key order.
        let order_a: Vec<ServiceId> =
            store.snapshot().service_facts.into_iter().map(|(id, _)| id).collect();
        let order_b: Vec<ServiceId> =
            store.snapshot().service_facts.into_iter().map(|(id, _)| id).collect();
        assert_eq!(order_a, order_b, "iteration order is stable across reads");
        let mut sorted = order_a.clone();
        sorted.sort_unstable();
        assert_eq!(order_a, sorted, "BTreeMap iterates in ascending Ord order");

        // The secondary index iterates deterministically too: keys in
        // ascending WorkloadId order across two reads.
        let mut store2 = store.clone();
        store2.upsert(workload("api"), &vip("10.96.0.8"), &[listener(9000, Proto::Tcp)]);
        let keys_a: Vec<WorkloadId> =
            store2.snapshot().workload_index.into_iter().map(|(w, _)| w).collect();
        let keys_b: Vec<WorkloadId> =
            store2.snapshot().workload_index.into_iter().map(|(w, _)| w).collect();
        assert_eq!(keys_a, keys_b, "secondary iteration order is stable");
        let mut keys_sorted = keys_a.clone();
        keys_sorted.sort();
        assert_eq!(keys_a, keys_sorted, "secondary iterates in ascending Ord order");

        // The map types themselves are the structural guarantee that
        // iteration order is seed-deterministic: both fields are
        // `BTreeMap` (not `HashMap`), enforced at compile time by the
        // struct definition. `BTreeMap::keys` yields ascending `Ord`
        // order — there is no `RandomState` nondeterminism to leak.
        // (A source-text grep for "HashMap" is the AST-shape
        // anti-pattern per nw-test-optimization §2; the type signature
        // is the real, refactor-safe invariant.)
    }

    // -----------------------------------------------------------------
    // U6 — a non-upsert flow leaves both maps unchanged
    // -----------------------------------------------------------------

    #[test]
    fn conflict_release_does_not_insert_facts() {
        // A construct-then-non-upsert flow (e.g. a VIP conflict that
        // releases without ever calling upsert) leaves the store empty.
        let mut store = ListenerFactStore::new();
        // A no-op removal of an absent workload is the kind of flow a
        // release path performs — it must not insert anything.
        store.remove_workload(&workload("conflicted"));
        let snapshot = store.snapshot();
        assert!(snapshot.service_facts.is_empty(), "primary stays empty without upsert");
        assert!(snapshot.workload_index.is_empty(), "secondary stays empty without upsert");
        assert_eq!(store, ListenerFactStore::new(), "store equals a fresh empty store");
    }

    // -----------------------------------------------------------------
    // 01-02 wiring — AppState carries a boot-rebuilt ListenerFactStore
    // -----------------------------------------------------------------

    /// Reassemble `AppState` (mirroring the production boot wiring's
    /// post-`bulk_load` rebuild) carrying a `listener_facts` field that
    /// is the boot-rebuilt projection over `state`'s OWN store +
    /// allocator. Reuses the same `store` / `allocator` Arcs so the
    /// allocator memo populated by `persist_service_and_allocate_vip`
    /// is visible to the rebuild — a fresh allocator would have an empty
    /// memo and skip every Service.
    async fn reassemble_with_boot_rebuild(state: &AppState) -> AppState {
        let listener_facts = Arc::new(tokio::sync::Mutex::new(
            ListenerFactStore::rebuild_from_intent(
                &state.store,
                &state.intent_redb_path,
                &state.allocator,
            )
            .await
            .expect("boot rebuild"),
        ));
        AppState::new(
            Arc::clone(&state.store),
            state.intent_redb_path.clone(),
            Arc::clone(&state.obs),
            Arc::clone(&state.runtime),
            Arc::clone(&state.driver),
            Arc::clone(&state.clock),
            Arc::clone(&state.dataplane),
            state.node_id.clone(),
            Arc::clone(&state.allocator),
            listener_facts,
            state.host_ipv4,
        )
    }

    /// Boot rebuild over a fixture intent set populates `AppState.
    /// listener_facts` with one primary entry per listener of each
    /// VIP-allocated Service. This is the wiring contract 01-02 adds:
    /// the store exists, is boot-rebuilt, and is held on `AppState`.
    #[tokio::test]
    async fn app_state_boot_rebuilds_listener_facts_from_intent() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 11));

        // Seed two VIP-allocated Services through the SAME state (its
        // allocator memo is what the rebuild joins against).
        let seed_state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);
        let vip_web = persist_service_and_allocate_vip(
            &seed_state,
            "web",
            &[(80, Proto::Tcp), (53, Proto::Udp)],
        )
        .await;
        let vip_api =
            persist_service_and_allocate_vip(&seed_state, "api", &[(9000, Proto::Tcp)]).await;

        // Reassemble AppState with the boot-rebuilt projection over the
        // SAME store + allocator (production wiring shape).
        let state = reassemble_with_boot_rebuild(&seed_state).await;

        let (web_tcp, web_tcp_row) = derived(&vip_web, 80, Proto::Tcp);
        let (web_udp, web_udp_row) = derived(&vip_web, 53, Proto::Udp);
        let (api_tcp, api_tcp_row) = derived(&vip_api, 9000, Proto::Tcp);
        // Read the observable values out while holding the lock, then
        // drop the guard before asserting (clippy::significant_drop_
        // tightening — don't hold the mutex across the assertion block).
        let (web_tcp_got, web_udp_got, api_tcp_got, primary_len) = {
            let facts = state.listener_facts.lock().await;
            (
                facts.fact_for(web_tcp),
                facts.fact_for(web_udp),
                facts.fact_for(api_tcp),
                facts.snapshot().service_facts.len(),
            )
        };
        assert_eq!(web_tcp_got, Some(web_tcp_row), "web tcp listener present");
        assert_eq!(web_udp_got, Some(web_udp_row), "web udp listener present");
        assert_eq!(api_tcp_got, Some(api_tcp_row), "api tcp listener present");
        assert_eq!(primary_len, 3, "one primary entry per listener of each VIP'd Service");
    }

    /// AppState construction with an EMPTY intent set yields an empty
    /// listener-fact store (both maps empty).
    #[tokio::test]
    async fn app_state_empty_intent_yields_empty_store() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 12));

        // No intent seeded — the boot rebuild runs over an empty store.
        let seed_state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);
        let state = reassemble_with_boot_rebuild(&seed_state).await;

        // Clone the projection out while holding the lock, then drop the
        // guard before asserting (clippy::significant_drop_tightening).
        let store = { state.listener_facts.lock().await.clone() };
        let snapshot = store.snapshot();
        assert!(snapshot.service_facts.is_empty(), "primary map empty over empty intent set");
        assert!(snapshot.workload_index.is_empty(), "secondary map empty over empty intent set");
        assert_eq!(store, ListenerFactStore::new(), "store equals a fresh empty store");
    }

    // -----------------------------------------------------------------
    // B (proptest) — incremental store == rebuild over the same intents
    // -----------------------------------------------------------------

    /// A generated intent set: a list of (workload-name, vip, listeners)
    /// triples with at least one service carrying ≥2 listeners.
    #[derive(Debug, Clone)]
    struct IntentSet {
        services: Vec<(String, ServiceVip, Vec<Listener>)>,
    }

    fn proto_strategy() -> impl Strategy<Value = Proto> {
        prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
    }

    fn listener_strategy() -> impl Strategy<Value = Listener> {
        (1u16..=65535, proto_strategy()).prop_map(|(port, proto)| Listener {
            port: NonZeroU16::new(port).unwrap(),
            protocol: proto,
        })
    }

    prop_compose! {
        fn intent_set_strategy()(
            // One listener-list per service; the service index supplies a
            // DISTINCT VIP last octet (10.96.0.{1+i}) so no two services
            // collide on a ServiceId (which would let one upsert's primary
            // entry overwrite another and make the equivalence vacuous).
            listener_lists in prop::collection::vec(
                prop::collection::vec(listener_strategy(), 1..4),
                1..5,
            )
        ) -> IntentSet {
            let services = listener_lists
                .into_iter()
                .enumerate()
                .map(|(i, listeners)| {
                    let name = format!("svc-{i}");
                    let octet = u8::try_from(1 + i).unwrap_or(1);
                    let v = ServiceVip::new(
                        std::net::Ipv4Addr::new(10, 96, 0, octet).into(),
                    )
                    .unwrap();
                    (name, v, listeners)
                })
                .collect();
            IntentSet { services }
        }
    }

    /// Build a store incrementally via `upsert` over the generated set.
    fn build_incremental(set: &IntentSet) -> ListenerFactStore {
        let mut store = ListenerFactStore::new();
        for (name, v, listeners) in &set.services {
            store.upsert(WorkloadId::new(name).unwrap(), v, listeners);
        }
        store
    }

    /// Build a store via the full intent path: persist each Service +
    /// allocate its VIP, then `rebuild_from_intent`. Forcing the
    /// allocator to issue the SAME VIPs the incremental path used is not
    /// possible (the allocator picks), so instead we assert the rebuild
    /// equals an incremental build over the ACTUAL allocator-issued VIPs.
    async fn build_via_rebuild_and_expected(
        set: &IntentSet,
    ) -> (ListenerFactStore, ListenerFactStore) {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 7));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        // Persist + allocate; record the (workload, issued-vip, listeners)
        // so the expected incremental store uses the SAME vips.
        let mut expected = ListenerFactStore::new();
        for (name, _gen_vip, listeners) in &set.services {
            let listener_inputs: Vec<ListenerInput> = listeners
                .iter()
                .map(|l| ListenerInput {
                    port: l.port.get(),
                    protocol: proto_str(l.protocol).into(),
                })
                .collect();
            let svc = overdrive_core::aggregate::ServiceV1::from_submit(ServiceSpecInput {
                id: name.clone(),
                replicas: 1,
                resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
                driver: DriverInput::Exec(ExecInput {
                    command: "/bin/serve".to_string(),
                    args: vec![],
                }),
                listeners: listener_inputs,
                startup_probes: vec![],
                readiness_probes: vec![],
                liveness_probes: vec![],
            })
            .expect("valid service");
            let wid = svc.id.clone();
            persist_intent_and_kind(
                &state,
                WorkloadIntent::Service(svc.clone()),
                &wid,
                WorkloadKind::Service,
            )
            .await;
            let digest = WorkloadIntent::Service(svc.clone()).spec_digest().expect("digest");
            let bytes: [u8; 32] = *digest.as_bytes();
            let mut guard = state.allocator.lock().await;
            let issued = guard.allocate(bytes).await.expect("allocate");
            drop(guard);
            expected.upsert(wid, &issued, &svc.listeners);
        }

        let rebuilt = ListenerFactStore::rebuild_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.allocator,
        )
        .await
        .expect("rebuild");
        (rebuilt, expected)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

        /// The store built incrementally via `upsert` is byte-equivalent
        /// (entry-for-entry on BOTH maps, incl. secondary inner-`Vec`
        /// order) to one built via `rebuild_from_intent` over the same
        /// intent set. Guards against the relocated projection drifting
        /// from `upsert`.
        #[test]
        fn edge_maintained_store_byte_equivalent_to_rebuild(set in intent_set_strategy()) {
            // At least one service with ≥2 listeners (per AC).
            prop_assume!(set.services.iter().any(|(_, _, l)| l.len() >= 2));

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let (rebuilt, expected) = rt.block_on(build_via_rebuild_and_expected(&set));

            // Incremental-vs-incremental sanity: build_incremental over the
            // GENERATED vips is structurally the same construction the
            // rebuild's `expected` uses over ISSUED vips — both go through
            // `upsert`, so the equivalence under test is rebuilt == expected.
            let _ = build_incremental(&set);
            prop_assert_eq!(rebuilt.snapshot(), expected.snapshot());
        }
    }

    // -----------------------------------------------------------------
    // B-ex (example) — three-listener web service via BOTH paths
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn edge_maintained_store_byte_equivalent_to_rebuild_three_listener_example() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 7));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        // "web" with (80,Tcp)(443,Tcp)(53,Udp).
        let issued = persist_service_and_allocate_vip(
            &state,
            "web",
            &[(80, Proto::Tcp), (443, Proto::Tcp), (53, Proto::Udp)],
        )
        .await;

        let rebuilt = ListenerFactStore::rebuild_from_intent(
            &state.store,
            &state.intent_redb_path,
            &state.allocator,
        )
        .await
        .expect("rebuild");

        let mut expected = ListenerFactStore::new();
        expected.upsert(
            workload("web"),
            &issued,
            &[listener(80, Proto::Tcp), listener(443, Proto::Tcp), listener(53, Proto::Udp)],
        );

        assert_eq!(
            rebuilt.snapshot(),
            expected.snapshot(),
            "rebuild and upsert produce identical stores"
        );
        // 3 primary entries + a 3-element secondary Vec via the rebuild.
        let snapshot = rebuilt.snapshot();
        assert_eq!(snapshot.service_facts.len(), 3, "three primary entries");
        assert_eq!(
            snapshot
                .workload_index
                .iter()
                .find(|(w, _)| w == &workload("web"))
                .map(|(_, ids)| ids.len()),
            Some(3),
            "secondary Vec has three elements"
        );
    }

    // -----------------------------------------------------------------
    // 01-03 handler-edge maintenance — submit upsert / conflict no-op /
    // stop remove, driven through the public submit_workload /
    // stop_workload driving ports (port-to-port; the tests would not
    // flip RED→GREEN if the handler short-circuited around the edge
    // mutations).
    // -----------------------------------------------------------------

    use axum::Json;
    use axum::extract::{Path as AxPath, State};
    use axum::http::HeaderMap;
    use axum::response::Response;
    use overdrive_core::aggregate::JobSpecInput;
    use overdrive_core::api::submit::{ServiceSpecInput as WireServiceSpec, SubmitSpecInput};

    fn wire_service(id: &str, listeners: &[(u16, Proto)]) -> SubmitSpecInput {
        SubmitSpecInput::Service(WireServiceSpec {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput {
                command: "/bin/serve".to_string(),
                args: vec![],
            }),
            listeners: listeners
                .iter()
                .map(|(port, proto)| ListenerInput {
                    port: *port,
                    protocol: proto_str(*proto).into(),
                })
                .collect(),
            startup_probes: vec![],
            readiness_probes: vec![],
            liveness_probes: vec![],
        })
    }

    fn wire_job(id: &str) -> SubmitSpecInput {
        SubmitSpecInput::Job(JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Exec(ExecInput { command: "/bin/run".to_string(), args: vec![] }),
        })
    }

    async fn submit_via_handler(
        state: &AppState,
        spec: SubmitSpecInput,
    ) -> Result<Response, crate::error::ControlPlaneError> {
        crate::handlers::submit_workload(
            State(state.clone()),
            HeaderMap::new(),
            Json(crate::api::SubmitWorkloadRequest { spec }),
        )
        .await
    }

    async fn stop_via_handler(state: &AppState, id: &str) {
        let _response = crate::handlers::stop_workload(State(state.clone()), AxPath(id.to_owned()))
            .await
            .expect("stop must succeed");
    }

    /// Submit a Service whose VIP is allocated on the Inserted edge →
    /// the handler upserts one primary entry per listener, keyed by the
    /// derived `ServiceId`, carrying the allocator-issued VIP.
    #[tokio::test]
    async fn submit_inserted_upserts_listener_facts() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 21));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        submit_via_handler(&state, wire_service("web", &[(80, Proto::Tcp), (53, Proto::Udp)]))
            .await
            .expect("Service submit must succeed");

        // The allocator issued the VIP under the same spec_digest the
        // handler used — recover it the same way the edge / rebuild does.
        let svc = WorkloadIntent::Service(
            overdrive_core::aggregate::ServiceV1::from_submit(
                match wire_service("web", &[(80, Proto::Tcp), (53, Proto::Udp)]) {
                    SubmitSpecInput::Service(s) => s,
                    _ => unreachable!(),
                },
            )
            .expect("valid service"),
        );
        let digest: [u8; 32] = *svc.spec_digest().expect("digest").as_bytes();
        let vip = {
            let guard = state.allocator.lock().await;
            let v = guard.get(&digest);
            drop(guard);
            v.expect("VIP issued on Inserted edge")
        };

        let (tcp_sid, tcp_row) = derived(&vip, 80, Proto::Tcp);
        let (udp_sid, udp_row) = derived(&vip, 53, Proto::Udp);
        let store = { state.listener_facts.lock().await.clone() };
        assert_eq!(store.fact_for(tcp_sid), Some(tcp_row), "tcp listener fact present");
        assert_eq!(store.fact_for(udp_sid), Some(udp_row), "udp listener fact present");
        let snapshot = store.snapshot();
        assert_eq!(snapshot.service_facts.len(), 2, "exactly two primary entries");
        assert_eq!(
            snapshot
                .workload_index
                .iter()
                .find(|(w, _)| w == &workload("web"))
                .map(|(_, ids)| ids.len()),
            Some(2),
            "secondary Vec has two ServiceIds in listener order",
        );
    }

    /// A Job submit allocates no VIP and contributes no listener facts —
    /// the edge upsert is Service-only.
    #[tokio::test]
    async fn submit_job_inserts_no_listener_facts() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 22));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        submit_via_handler(&state, wire_job("batch")).await.expect("Job submit must succeed");

        let snapshot = { state.listener_facts.lock().await.clone() };
        assert_eq!(snapshot, ListenerFactStore::new(), "Job submit leaves the store empty");
    }

    /// U6 end-to-end — a conflicting submit (KeyExists, non-identical:
    /// same workload id, different listener set) is rejected with a 409
    /// AND its VIP is released; the conflict-release branch is a store
    /// NO-OP, so BOTH maps remain exactly as the first submit left them.
    #[tokio::test]
    async fn conflict_release_leaves_store_unchanged() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 23));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        submit_via_handler(&state, wire_service("api", &[(9000, Proto::Tcp)]))
            .await
            .expect("first Service submit must succeed");
        let before = { state.listener_facts.lock().await.clone() };

        // Same workload id, different listeners → KeyExists non-identical
        // → HTTP 409 Conflict, VIP allocated-then-released.
        let conflict = submit_via_handler(&state, wire_service("api", &[(9001, Proto::Tcp)])).await;
        assert!(
            matches!(conflict, Err(crate::error::ControlPlaneError::Conflict { .. })),
            "non-identical resubmit at the same key must 409",
        );

        let after = { state.listener_facts.lock().await.clone() };
        assert_eq!(before, after, "conflict-release branch must be a store NO-OP");
    }

    /// Submit-then-stop evicts the workload's facts from BOTH the primary
    /// and secondary maps via the stop edge's `remove_workload`.
    #[tokio::test]
    async fn stop_removes_workload_facts() {
        let tmp = TempDir::new().expect("tmpdir");
        let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 24));
        let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

        submit_via_handler(&state, wire_service("web", &[(80, Proto::Tcp), (443, Proto::Tcp)]))
            .await
            .expect("Service submit must succeed");
        // Non-empty before stop.
        let before = { state.listener_facts.lock().await.clone() }.snapshot();
        assert_eq!(before.service_facts.len(), 2, "two primary entries before stop");
        assert!(
            before.workload_index.iter().any(|(w, _)| w == &workload("web")),
            "secondary entry before stop"
        );

        stop_via_handler(&state, "web").await;

        let after = { state.listener_facts.lock().await.clone() }.snapshot();
        assert!(after.service_facts.is_empty(), "primary evicted after stop");
        assert!(
            !after.workload_index.iter().any(|(w, _)| w == &workload("web")),
            "secondary entry dropped after stop"
        );
        assert_eq!(
            after,
            ListenerFactStore::new().snapshot(),
            "store empty after the only workload is stopped"
        );
    }
}
