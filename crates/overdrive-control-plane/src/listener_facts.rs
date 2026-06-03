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
//! "service-map")` — the identical derivation
//! `hydrate_bridge_desired_listeners` and `gather_service_listener_facts`
//! use, so the read-path key matches the hydrator's projection.

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
    /// `ServiceId::derive(vip, listener.port, "service-map")`, inserts
    /// a `ListenerRow { vip: Some(vip), port, protocol }` into the
    /// primary map, and appends the derived `ServiceId` to the
    /// workload's `Vec` in the secondary map.
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
            let service_id = ServiceId::derive(vip, listener.port, SERVICE_MAP_PURPOSE);
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
            ServiceId::derive(v, nz, super::SERVICE_MAP_PURPOSE),
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
            .map(|l| ServiceId::derive(&v, l.port, super::SERVICE_MAP_PURPOSE))
            .collect();
        assert_eq!(
            store.secondary.get(&wid),
            Some(&expected_ids),
            "secondary Vec must list every listener's ServiceId in declaration order"
        );
        assert_eq!(store.primary.len(), 3, "exactly three primary entries");
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
        assert_eq!(store.secondary.get(&workload("web")), None, "web's secondary entry dropped");
        assert!(store.secondary.contains_key(&workload("api")), "api's secondary entry retained");

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
        assert_eq!(store.primary.len(), 2, "only the VIP'd Service's listeners contribute");
        assert_eq!(
            store.secondary.keys().collect::<Vec<_>>(),
            vec![&workload("web")],
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

        // BTreeMap iterates in Ord(ServiceId) order — collect twice and
        // confirm identical, ascending key order.
        let order_a: Vec<ServiceId> = store.primary.keys().copied().collect();
        let order_b: Vec<ServiceId> = store.primary.keys().copied().collect();
        assert_eq!(order_a, order_b, "iteration order is stable across reads");
        let mut sorted = order_a.clone();
        sorted.sort_unstable();
        assert_eq!(order_a, sorted, "BTreeMap iterates in ascending Ord order");

        // The secondary index iterates deterministically too: keys in
        // ascending WorkloadId order across two reads.
        let mut store2 = store.clone();
        store2.upsert(workload("api"), &vip("10.96.0.8"), &[listener(9000, Proto::Tcp)]);
        let keys_a: Vec<WorkloadId> = store2.secondary.keys().cloned().collect();
        let keys_b: Vec<WorkloadId> = store2.secondary.keys().cloned().collect();
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
        assert!(store.primary.is_empty(), "primary stays empty without upsert");
        assert!(store.secondary.is_empty(), "secondary stays empty without upsert");
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
                facts.primary.len(),
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
        let snapshot = { state.listener_facts.lock().await.clone() };
        assert!(snapshot.primary.is_empty(), "primary map empty over empty intent set");
        assert!(snapshot.secondary.is_empty(), "secondary map empty over empty intent set");
        assert_eq!(snapshot, ListenerFactStore::new(), "store equals a fresh empty store");
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
            prop_assert_eq!(rebuilt, expected);
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

        assert_eq!(rebuilt, expected, "rebuild and upsert produce identical stores");
        // 3 primary entries + a 3-element secondary Vec via the rebuild.
        assert_eq!(rebuilt.primary.len(), 3, "three primary entries");
        assert_eq!(
            rebuilt.secondary.get(&workload("web")).map(Vec::len),
            Some(3),
            "secondary Vec has three elements"
        );
    }
}
