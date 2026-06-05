//! redb-backed implementation of [`ObservationStore`] for the single-node
//! `LocalObservationStore`.
//!
//! Per ADR-0012 (revised 2026-04-24), Phase 1 observation state lives in
//! a dedicated redb database co-resident with the intent store. The
//! previous wiring routed production observation through
//! `SimObservationStore` (an in-memory CR-SQLite-shaped fixture) — that
//! reversal traded persistence for purity and was reversed once the
//! objection surfaced: observation rows must survive a server restart,
//! and the "all production impls live under real adapters" rule in
//! ADR-0003 must not be perforated for convenience.
//!
//! # Durability shape
//!
//! Two redb tables:
//!
//! * `alloc_status` — keyed by canonical `AllocationId` bytes, value is
//!   the rkyv-archived `AllocStatusRow`. Overwrite semantics on the key
//!   (second write for same id replaces the first).
//! * `node_health` — keyed by canonical `NodeId` bytes, value is the
//!   rkyv-archived `NodeHealthRow`. Same overwrite semantics.
//!
//! Phase 1 has NO on-disk schema versioning for observation rows — the
//! format is the rkyv layout of the Rust types at build time. A Phase 2
//! migration (new row variants, field additions) ships its own
//! schema-migration reconciler; the Phase 1 file is considered
//! rebuild-on-upgrade until then.
//!
//! # Subscription shape
//!
//! Subscribers receive a `tokio::sync::broadcast` stream of every row
//! written to this peer AFTER the subscription opens — the future-only
//! contract from the `ObservationStore` trait. Subscribers that lag
//! past the broadcast capacity drop the lagged notifications silently
//! and continue delivering subsequent events; the stream does not
//! close, so a caller relying on end-of-stream as a catch-up trigger
//! will miss the lost events. Phase 2's Corrosion replacement
//! recovers via CR-SQLite gossip catch-up.
//!
//! # Malformed-row warn cadence
//!
//! Malformed observation rows produce a
//! `tracing::warn!(name = "observation.envelope.decode_failed", ...)`
//! event on every scan that encounters them — once per row per call
//! to `<table>_rows()`. A row that persists across many reconciler
//! ticks will emit many warn events. The intended remediation is to
//! rewrite the row through the typed write API (`apply_*_lww`),
//! which replaces the malformed bytes and silences subsequent warns
//! naturally. There is no in-memory dedup; the scan path is
//! stateless by design (the `ObservationStore` is gossiped and any
//! node may converge first — so a stateful "we already warned for
//! this row" cache on one peer would still re-warn on every other
//! peer that scans the row and would also re-warn this peer after
//! restart). Per ADR-0048 § 3, observation log-and-skip is the
//! correct degrade path; the warn cadence is the price.
//!
//! # LWW guard on `write`
//!
//! Per the `ObservationStore::write` trait contract codified in
//! `overdrive-core`, a write whose `updated_at` (alloc-status) or
//! `last_heartbeat` (node-health) does not dominate the existing row
//! at the same primary key MUST NOT mutate state and MUST NOT be
//! emitted on subscriptions. This implementation runs the comparison
//! INSIDE the redb `begin_write` transaction (no TOCTOU window) and
//! suppresses the post-commit broadcast on loss. Comparator:
//! [`overdrive_core::traits::observation_store::LogicalTimestamp::dominates`].
//! See `docs/feature/fix-observation-lww-merge/deliver/rca.md` for the
//! bug RCA and `docs/product/architecture/adr-0012-observation-store-server-impl.md`
//! for the third-revision rationale.

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::Stream;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::codec::{VersionedEnvelope, decode_envelope_bytes};
use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{AllocationId, ServiceId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeResultRowEnvelope};
use overdrive_core::traits::observation_store::{
    AllocStatusRow, AllocStatusRowEnvelope, NodeHealthRow, NodeHealthRowEnvelope, ObservationRow,
    ObservationStore, ObservationStoreError, ObservationSubscription, ReconcileConflictRow,
    ReconcileConflictRowEnvelope, ServiceBackendRow, ServiceBackendRowEnvelope,
    ServiceHydrationResultRow, ServiceHydrationResultRowEnvelope,
};
use redb::{Database, ReadableTable, Table, TableDefinition};
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

/// Holds the rkyv-archived bytes of every `AllocStatusRow`, keyed by
/// canonical `AllocationId` bytes.
const ALLOC_STATUS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_alloc_status");

/// Holds the rkyv-archived bytes of every `NodeHealthRow`, keyed by
/// canonical `NodeId` bytes.
const NODE_HEALTH_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_node_health");

/// Holds the rkyv-archived bytes of every `ServiceHydrationResultRow`,
/// keyed on the canonical 16-byte encoding of `(service_id,
/// fingerprint)` (each component little-endian u64). Phase 2.2 is
/// single-writer (the action shim) and additive-only per
/// `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 12.
const SERVICE_HYDRATION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_service_hydration_results");

/// Holds the rkyv-archived bytes of every `ServiceBackendRow`, keyed
/// on the canonical 8-byte LE encoding of `ServiceId`. One row per
/// service — the full current backend set. GH #160.
const SERVICE_BACKENDS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_service_backends");

/// Holds the rkyv-archived bytes of every `ReconcileConflictRow`,
/// keyed on the canonical 15-byte encoding of `(service_id, vip, port,
/// proto)` — the conflicting map slot (Fix C,
/// `fix-mixed-backend-dispatch-spin`). New table; additive-only,
/// never alters existing tables.
const RECONCILE_CONFLICT_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_reconcile_conflict");

/// Holds the rkyv-archived bytes of every `ProbeResultRow`, keyed on
/// the canonical composite encoding of `(alloc_id, probe_idx)` per
/// ADR-0054 §5. Key layout: `alloc_id_bytes || 0x00 || probe_idx LE
/// u32` — the NUL separator guarantees unambiguous prefix-scan
/// boundaries (no `AllocationId` byte sequence may contain NUL since
/// it is parsed from non-empty `&str`). LWW resolution on
/// `last_observed_at_unix_ms`.
const PROBE_RESULTS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_probe_results");

/// Holds the rkyv-archived bytes of every `IssuedCertificateRow`
/// (`issued_certificates` audit row, ADR-0063 D6), keyed on the issued
/// certificate's serial bytes. The audit surface is **append-only** —
/// one row per distinct issued serial, never overwritten (serials are
/// CSPRNG-drawn, so a collision is the issuance bug, not an LWW case).
/// The row is OBSERVATION (the record of what was issued), persisted in
/// the production observation store alongside `alloc_status` /
/// `node_health`, and routed through the `ObservationStore` trait as a
/// first-class [`ObservationRow::IssuedCertificate`] variant — exactly
/// like its `alloc_status` / `node_health` siblings. New table;
/// additive-only, never alters existing tables.
const ISSUED_CERTIFICATES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("observation_issued_certificates");

/// Encode the composite `(alloc_id, probe_idx)` key as a byte
/// sequence. Sort order on the underlying redb `BTree` mirrors the
/// lexicographic-on-bytes order of this layout — for a given
/// `alloc_id`, probe indices sort ascending.
fn encode_probe_result_key(alloc_id: &AllocationId, probe_idx: ProbeIdx) -> Vec<u8> {
    let alloc_bytes = alloc_id.as_str().as_bytes();
    let idx_bytes = probe_idx.get().to_le_bytes();
    let mut out = Vec::with_capacity(alloc_bytes.len() + 1 + idx_bytes.len());
    out.extend_from_slice(alloc_bytes);
    out.push(0x00);
    out.extend_from_slice(&idx_bytes);
    out
}

/// Encode the prefix `(alloc_id, *)` for range scans — the
/// `alloc_id` bytes + NUL separator (first segment of
/// [`encode_probe_result_key`]).
fn encode_probe_result_prefix(alloc_id: &AllocationId) -> Vec<u8> {
    let alloc_bytes = alloc_id.as_str().as_bytes();
    let mut out = Vec::with_capacity(alloc_bytes.len() + 1);
    out.extend_from_slice(alloc_bytes);
    out.push(0x00);
    out
}

/// Encode the composite `(service_id, fingerprint)` key as 16 bytes
/// (`service_id` LE u64 || `fingerprint` LE u64). Sort order on the
/// `BTreeMap`-backed redb tree mirrors the lexicographic-on-bytes
/// order of this layout — predictable, deterministic, no JSON.
const fn encode_service_hydration_key(
    service_id: ServiceId,
    fingerprint: BackendSetFingerprint,
) -> [u8; 16] {
    let sid = service_id.get().to_le_bytes();
    let fp = fingerprint.to_le_bytes();
    [
        sid[0], sid[1], sid[2], sid[3], sid[4], sid[5], sid[6], sid[7], fp[0], fp[1], fp[2], fp[3],
        fp[4], fp[5], fp[6], fp[7],
    ]
}

/// Encode the prefix of `(service_id, *)` for range scans — first 8
/// bytes of [`encode_service_hydration_key`].
const fn encode_service_hydration_prefix(service_id: ServiceId) -> [u8; 8] {
    service_id.get().to_le_bytes()
}

/// Encode the `ServiceId` key as 8 LE bytes for the
/// `SERVICE_BACKENDS_TABLE`. Single-keyed (no composite). GH #160.
const fn encode_service_backends_key(service_id: ServiceId) -> [u8; 8] {
    service_id.get().to_le_bytes()
}

/// Encode the composite `(service_id, vip, port, proto)` key as 15
/// bytes (`service_id` LE u64 || `vip` 4 octets BE || `port` LE u16 ||
/// `proto` IANA byte) for the `RECONCILE_CONFLICT_TABLE`. The leading
/// 8 `service_id` bytes are the prefix-scan boundary
/// ([`encode_reconcile_conflict_prefix`]); within a service the slot
/// triple disambiguates rows. Fix C.
const fn encode_reconcile_conflict_key(
    service_id: ServiceId,
    vip: std::net::Ipv4Addr,
    port: u16,
    proto: overdrive_core::dataplane::backend_key::Proto,
) -> [u8; 15] {
    let sid = service_id.get().to_le_bytes();
    let vip_oct = vip.octets();
    let port_b = port.to_le_bytes();
    [
        sid[0],
        sid[1],
        sid[2],
        sid[3],
        sid[4],
        sid[5],
        sid[6],
        sid[7],
        vip_oct[0],
        vip_oct[1],
        vip_oct[2],
        vip_oct[3],
        port_b[0],
        port_b[1],
        proto.as_u8(),
    ]
}

/// Encode the prefix `(service_id, *)` for range scans — the first 8
/// bytes of [`encode_reconcile_conflict_key`]. Fix C.
const fn encode_reconcile_conflict_prefix(service_id: ServiceId) -> [u8; 8] {
    service_id.get().to_le_bytes()
}

/// Capacity of the in-process broadcast channel used for
/// `subscribe_all`. Sized to absorb a short-lived reader stall on a
/// single-node workload without backing memory to the moon. Subscribers
/// that lag past this silently lose the dropped notifications and keep
/// receiving subsequent ones — the stream does not close on lag (see
/// module docs).
const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 1024;

/// Redb-backed `ObservationStore`. Cheap to clone via `Arc`; safe to
/// share across tasks and threads.
pub struct LocalObservationStore {
    inner: Arc<Inner>,
}

struct Inner {
    /// `redb::Database` handles its own internal locking.
    db: Database,
    /// Fan-out channel for `subscribe_all` subscribers. Every
    /// successful `write` emits the row on this channel after the redb
    /// commit succeeds — subscribers never observe a phantom row that
    /// failed to persist.
    subscription_tx: broadcast::Sender<ObservationRow>,
}

impl LocalObservationStore {
    /// Open (or create) a redb-backed `LocalObservationStore` at `path`.
    ///
    /// The parent directory is created if missing. Both observation
    /// tables are materialised on open so the first read does not need
    /// to take a write transaction to create them.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ObservationStoreError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(ObservationStoreError::Io)?;
        }
        let db = Database::create(path).map_err(map_to_io)?;

        // Materialize all observation tables up-front.
        {
            let write = db.begin_write().map_err(map_to_io)?;
            {
                let _ = write.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
                let _ = write.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
                // Service-hydration table — additive-only migration
                // per `docs/feature/phase-2-xdp-service-map/design/
                // architecture.md` § 12. New table; never alters
                // existing tables.
                let _ = write.open_table(SERVICE_HYDRATION_TABLE).map_err(map_to_io)?;
                // Service-backends table — GH #160.
                let _ = write.open_table(SERVICE_BACKENDS_TABLE).map_err(map_to_io)?;
                // Reconcile-conflict table — Fix C. New table;
                // additive-only.
                let _ = write.open_table(RECONCILE_CONFLICT_TABLE).map_err(map_to_io)?;
                // Probe-results table — ADR-0054 §5. New table;
                // never alters existing tables.
                let _ = write.open_table(PROBE_RESULTS_TABLE).map_err(map_to_io)?;
                // Issued-certificates audit table — ADR-0063 D6. New
                // table; additive-only, never alters existing tables.
                let _ = write.open_table(ISSUED_CERTIFICATES_TABLE).map_err(map_to_io)?;
            }
            write.commit().map_err(map_to_io)?;
        }

        let (subscription_tx, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);

        Ok(Self { inner: Arc::new(Inner { db, subscription_tx }) })
    }

    fn emit(&self, row: ObservationRow) {
        // `send` returns `Err` only when there are no active
        // subscribers — that's not a failure.
        let _ = self.inner.subscription_tx.send(row);
    }
}

#[async_trait]
impl ObservationStore for LocalObservationStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let row_for_commit = row.clone();

        // The LWW comparison runs INSIDE the `begin_write` transaction
        // — a TOCTOU between read and insert is impossible because
        // redb's serializable isolation already linearises writers.
        // The closure returns whether the write was accepted; the
        // post-await branch suppresses `self.emit` on LWW reject.
        // See `ObservationStore::write`'s trait docstring in
        // `overdrive-core` for the trait-level contract; see
        // `docs/feature/fix-observation-lww-merge/deliver/rca.md` for
        // the bug RCA that motivated this guard.
        let accepted: bool = tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_to_io)?;
            let accepted = match &row_for_commit {
                ObservationRow::AllocStatus(incoming) => {
                    let mut table = write.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
                    apply_alloc_status_lww(&mut table, incoming)?
                }
                ObservationRow::NodeHealth(incoming) => {
                    let mut table = write.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
                    apply_node_health_lww(&mut table, incoming)?
                }
                ObservationRow::ServiceHydration(incoming) => {
                    let mut table = write.open_table(SERVICE_HYDRATION_TABLE).map_err(map_to_io)?;
                    apply_service_hydration_lww(&mut table, incoming)?
                }
                ObservationRow::ServiceBackend(incoming) => {
                    let mut table = write.open_table(SERVICE_BACKENDS_TABLE).map_err(map_to_io)?;
                    apply_service_backends_lww(&mut table, incoming)?
                }
                ObservationRow::ReconcileConflict(incoming) => {
                    let mut table =
                        write.open_table(RECONCILE_CONFLICT_TABLE).map_err(map_to_io)?;
                    apply_reconcile_conflict_lww(&mut table, incoming)?
                }
                ObservationRow::IssuedCertificate(incoming) => {
                    let mut table =
                        write.open_table(ISSUED_CERTIFICATES_TABLE).map_err(map_to_io)?;
                    apply_issued_certificate(&mut table, incoming)?
                }
                // `WorkflowTerminal` (ADR-0064 §2) — accept and fan out to
                // subscribers; the durable terminal record for slice-01 is
                // the engine-side redb+CBOR journal (`JournalEntry::Terminal`,
                // K5), and the workflow-lifecycle reconciler reads this row
                // off the live observation stream to converge the instance.
                // No typed redb table is persisted here: a cold-boot
                // recovery of the obs terminal row would require a versioned
                // rkyv envelope per ADR-0048; the journal already provides
                // durable terminal recovery, so the obs row is the live
                // convergence signal only. Accepted unconditionally (no LWW
                // key collision is possible — the correlation key is unique
                // per instance terminal).
                ObservationRow::WorkflowTerminal { .. } => true,
            };
            // Commit unconditionally — a rejected write performed only
            // a read inside the transaction; redb handles the no-op
            // commit cleanly.
            write.commit().map_err(map_to_io)?;
            Ok::<_, ObservationStoreError>(accepted)
        })
        .await
        .map_err(map_to_io)??;

        // Suppress emit on LWW reject — subscribers must NEVER observe
        // a row the store will then refuse to return on read. Matches
        // `SimObservationStore::apply_alloc_status` /
        // `apply_node_health` semantics: the broadcast `send` happens
        // only inside the dominate branch.
        if accepted {
            self.emit(row);
        }
        Ok(())
    }

    async fn subscribe_all(&self) -> Result<ObservationSubscription, ObservationStoreError> {
        let rx = self.inner.subscription_tx.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(Result::ok);
        Ok(Box::new(SubscriptionStream { inner: Box::pin(stream) }))
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        // Emit decode-failure events on the calling async thread so
        // per-test `tracing::subscriber::set_default` guards (which
        // are thread-local) observe them. The blocking task collects
        // the failures and returns them alongside the surviving rows.
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
            let mut out: Vec<AllocStatusRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            let iter = table.iter().map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                match decode_envelope::<AllocStatusRowEnvelope>(v.value()) {
                    Ok(row) => out.push(row),
                    Err(err) => failures.push((k.value().to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((out, failures))
        })
        .await
        .map_err(map_to_io)??;

        // Per ADR-0048 § 3 (observation layer): log + skip rows
        // whose envelope decode failed. Convergence proceeds on
        // surviving rows.
        log_decode_failures(
            "observation_alloc_status",
            "skipping alloc-status row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }

    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let key: Vec<u8> = alloc_id.as_str().as_bytes().to_vec();
        let key_for_emit = key.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(ALLOC_STATUS_TABLE).map_err(map_to_io)?;
            let Some(value) = table.get(key.as_slice()).map_err(map_to_io)? else {
                return Ok::<_, ObservationStoreError>(Ok(None));
            };
            // Point lookup: per ADR-0048 § 3 observation policy is
            // log + skip, so a malformed row returns None (with a
            // structured warn event emitted on the calling thread).
            match decode_envelope::<AllocStatusRowEnvelope>(value.value()) {
                Ok(row) => Ok(Ok(Some(row))),
                Err(err) => Ok(Err(err)),
            }
        })
        .await
        .map_err(map_to_io)??;

        match outcome {
            Ok(opt) => Ok(opt),
            Err(err) => {
                log_decode_failures(
                    "observation_alloc_status",
                    "skipping alloc-status row that failed envelope decode",
                    vec![(key_for_emit, err)],
                );
                Ok(None)
            }
        }
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        // Per ADR-0048 § 3 (observation layer): log + skip rows whose
        // envelope decode failed. Mirror of the alloc_status_rows path
        // above — failures are collected inside the blocking task and
        // emitted on the calling async thread so per-test
        // `tracing::subscriber::set_default` guards (thread-local)
        // observe them.
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(NODE_HEALTH_TABLE).map_err(map_to_io)?;
            let mut out: Vec<NodeHealthRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            let iter = table.iter().map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                match decode_envelope::<NodeHealthRowEnvelope>(v.value()) {
                    Ok(row) => out.push(row),
                    Err(err) => failures.push((k.value().to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((out, failures))
        })
        .await
        .map_err(map_to_io)??;

        log_decode_failures(
            "observation_node_health",
            "skipping node-health row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }

    async fn issued_certificate_rows(
        &self,
    ) -> Result<Vec<IssuedCertificateRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let rows = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(ISSUED_CERTIFICATES_TABLE).map_err(map_to_io)?;
            let mut out: Vec<IssuedCertificateRow> = Vec::new();
            let iter = table.iter().map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                // Observation log-and-skip (ADR-0048 § 3): the row's own
                // `from_store_bytes` emits the `observation.row.skipped`
                // warn and returns `Err` on a decode failure; we drop the
                // offending row and keep the surviving ones. The key is
                // surfaced for operator diagnosis.
                let key = String::from_utf8_lossy(k.value());
                if let Ok(row) =
                    IssuedCertificateRow::from_store_bytes(v.value(), Some(key.as_ref()))
                {
                    out.push(row);
                }
            }
            Ok::<_, ObservationStoreError>(out)
        })
        .await
        .map_err(map_to_io)??;
        Ok(rows)
    }

    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let prefix = encode_service_hydration_prefix(*service_id);
        let mut range_start = [0u8; 16];
        let mut range_end = [0u8; 16];
        range_start[..8].copy_from_slice(&prefix);
        range_end[..8].copy_from_slice(&prefix);
        range_end[8..].fill(0xFF);
        // Per ADR-0048 § 3 (observation layer): log + skip rows whose
        // envelope decode failed. Mirror of the alloc_status_rows /
        // node_health_rows paths above — failures are collected inside
        // the blocking task and emitted on the calling async thread so
        // per-test `tracing::subscriber::set_default` guards
        // (thread-local) observe them.
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(SERVICE_HYDRATION_TABLE).map_err(map_to_io)?;
            let mut out: Vec<ServiceHydrationResultRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            let iter =
                table.range(range_start.as_slice()..=range_end.as_slice()).map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                match decode_envelope::<ServiceHydrationResultRowEnvelope>(v.value()) {
                    Ok(row) => out.push(row),
                    Err(err) => failures.push((k.value().to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((out, failures))
        })
        .await
        .map_err(map_to_io)??;

        log_decode_failures(
            "observation_service_hydration_results",
            "skipping service-hydration row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }

    async fn write_probe_result(&self, row: ProbeResultRow) -> Result<(), ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let row_for_commit = row.clone();
        tokio::task::spawn_blocking(move || {
            let write = inner.db.begin_write().map_err(map_to_io)?;
            {
                let mut table = write.open_table(PROBE_RESULTS_TABLE).map_err(map_to_io)?;
                apply_probe_result_lww(&mut table, &row_for_commit)?;
            }
            write.commit().map_err(map_to_io)?;
            Ok::<_, ObservationStoreError>(())
        })
        .await
        .map_err(map_to_io)??;
        Ok(())
    }

    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<ProbeResultRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let prefix = encode_probe_result_prefix(alloc_id);
        // Build a range [prefix, prefix-after) — prefix-after is the
        // prefix with its trailing NUL replaced by 0x01 to capture
        // every key whose first segment is exactly `alloc_id_bytes ||
        // 0x00`. Equivalent to a prefix scan.
        let mut range_end = prefix.clone();
        if let Some(last) = range_end.last_mut() {
            *last = 0x01;
        }
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(PROBE_RESULTS_TABLE).map_err(map_to_io)?;
            let mut out: Vec<ProbeResultRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            let iter = table.range(prefix.as_slice()..range_end.as_slice()).map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                match decode_envelope::<ProbeResultRowEnvelope>(v.value()) {
                    Ok(row) => out.push(row),
                    Err(err) => failures.push((k.value().to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((out, failures))
        })
        .await
        .map_err(map_to_io)??;

        // Per ADR-0048 § 3 (observation layer): log + skip rows whose
        // envelope decode failed. Convergence proceeds on surviving
        // rows.
        log_decode_failures(
            "observation_probe_results",
            "skipping probe-result row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }

    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let key = encode_service_backends_key(*service_id);
        // Per ADR-0048 § 3 (observation layer): log + skip rows whose
        // envelope decode failed. Mirror of the alloc_status_rows /
        // node_health_rows / service_hydration_results_rows paths above
        // — failures are collected inside the blocking task and
        // emitted on the calling async thread so per-test
        // `tracing::subscriber::set_default` guards (thread-local)
        // observe them.
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(SERVICE_BACKENDS_TABLE).map_err(map_to_io)?;
            let mut rows: Vec<ServiceBackendRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            if let Some(value) = table.get(key.as_slice()).map_err(map_to_io)? {
                match decode_envelope::<ServiceBackendRowEnvelope>(value.value()) {
                    Ok(row) => rows.push(row),
                    Err(err) => failures.push((key.to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((rows, failures))
        })
        .await
        .map_err(map_to_io)??;

        log_decode_failures(
            "observation_service_backends",
            "skipping service-backend row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }

    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
        let inner = Arc::clone(&self.inner);
        let prefix = encode_reconcile_conflict_prefix(*service_id);
        // Prefix-scan over `(service_id, *)`: the key layout is
        // `service_id LE u64 || vip || port || proto` (15 bytes); the
        // range `[prefix||0x00.., prefix||0xFF..]` covers every slot for
        // the service. Mirror of `service_hydration_results_rows`.
        let mut range_start = [0u8; 15];
        let mut range_end = [0xFFu8; 15];
        range_start[..8].copy_from_slice(&prefix);
        range_end[..8].copy_from_slice(&prefix);
        let (rows, decode_failures) = tokio::task::spawn_blocking(move || {
            let read = inner.db.begin_read().map_err(map_to_io)?;
            let table = read.open_table(RECONCILE_CONFLICT_TABLE).map_err(map_to_io)?;
            let mut out: Vec<ReconcileConflictRow> = Vec::new();
            let mut failures: Vec<(Vec<u8>, ObservationStoreError)> = Vec::new();
            let iter =
                table.range(range_start.as_slice()..=range_end.as_slice()).map_err(map_to_io)?;
            for item in iter {
                let (k, v) = item.map_err(map_to_io)?;
                match decode_envelope::<ReconcileConflictRowEnvelope>(v.value()) {
                    Ok(row) => out.push(row),
                    Err(err) => failures.push((k.value().to_vec(), err)),
                }
            }
            Ok::<_, ObservationStoreError>((out, failures))
        })
        .await
        .map_err(map_to_io)??;

        log_decode_failures(
            "observation_reconcile_conflict",
            "skipping reconcile-conflict row that failed envelope decode",
            decode_failures,
        );
        Ok(rows)
    }
}

// -----------------------------------------------------------------------------
// Envelope decode helpers — shared between the read path
// (`*_rows` methods on the impl block) and the LWW guards
// (`apply_*_lww` below). Both surfaces decode the same per-table
// envelope bytes; centralising the alignment + probe + rkyv + project
// pipeline through these helpers keeps the four per-table fns
// readable.
// -----------------------------------------------------------------------------

/// Decode a prior rkyv-archived envelope from redb-returned bytes and
/// project to the latest payload shape per ADR-0048.
///
/// Thin wrapper over [`overdrive_core::codec::decode_envelope_bytes`]
/// that re-surfaces the typed envelope error as
/// [`ObservationStoreError::Envelope`] so callers can branch on the
/// structured cause (malformed bytes vs unknown future variant). Per
/// ADR-0048 § 3 the observation-layer policy is to log + skip the row,
/// not refuse to start — see [`log_decode_failures`] for the
/// structured warn-event surface every `*_rows` method routes through.
fn decode_envelope<E>(bytes: &[u8]) -> Result<E::Latest, ObservationStoreError>
where
    E: VersionedEnvelope + rkyv::Archive,
    E::Archived: for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
        + rkyv::Deserialize<E, rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>>,
{
    decode_envelope_bytes::<E>(bytes).map_err(ObservationStoreError::from)
}

/// Emit a structured `observation.envelope.decode_failed` event for
/// each row that failed envelope decode during a scan. The warning
/// fires on the calling async thread so per-test
/// `tracing::subscriber::set_default` guards (which are thread-local)
/// observe it. Per ADR-0048 § 3 the observation-layer policy is
/// log + skip — this is the "log" half; the caller's containing
/// `*_rows` method has already collected the surviving rows.
fn log_decode_failures(
    table_name: &'static str,
    skipped_row_label: &'static str,
    failures: Vec<(Vec<u8>, ObservationStoreError)>,
) {
    for (key_bytes, err) in failures {
        tracing::warn!(
            name: "observation.envelope.decode_failed",
            table = table_name,
            key = ?key_bytes,
            source = ?err,
            "{skipped_row_label}",
        );
    }
}

// -----------------------------------------------------------------------------
// LWW-guarded inserts — read-then-conditional-insert inside the open
// `begin_write` transaction. The `ObservationStore::write` trait
// docstring in `overdrive-core` codifies the contract: an incoming row
// whose `updated_at` does not dominate the existing row at the same
// primary key MUST NOT mutate state.
//
// Returns `true` when the write was accepted (the row dominates a
// prior, or there is no prior); `false` when the write loses to an
// existing row. The caller (`LocalObservationStore::write`) gates the
// post-commit emit on the returned bool — losers must never be emitted
// on subscriptions.
// -----------------------------------------------------------------------------

/// LWW-guarded insert for `AllocStatusRow`. Reads the prior row at
/// `incoming.alloc_id` (if any), compares via
/// [`overdrive_core::traits::observation_store::LogicalTimestamp::dominates`],
/// and inserts only on dominate. Returns `true` if the row was inserted.
fn apply_alloc_status_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &AllocStatusRow,
) -> Result<bool, ObservationStoreError> {
    let key = incoming.alloc_id.as_str().as_bytes();
    // If the prior row's bytes don't decode (malformed / unknown
    // variant), treat the incoming write as dominating — the
    // operator's typed write is the self-healing path per
    // ADR-0048 § 3. is_none_or short-circuits on absent prior;
    // the inner branch returns true on either dominate OR decode
    // failure.
    let dominates = table.get(key).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<AllocStatusRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.updated_at.dominates(&prior_row.updated_at),
            Err(_) => true,
        }
    });
    if dominates {
        // Wrap into the versioned envelope at the write boundary per
        // ADR-0048 § 1 — the on-disk shape is the envelope, never the
        // bare payload.
        let envelope = AllocStatusRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key, bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `NodeHealthRow`. Mirrors
/// [`apply_alloc_status_lww`] — keyed by `incoming.node_id`, compares
/// `incoming.last_heartbeat` via `LogicalTimestamp::dominates`. On
/// envelope decode failure of the prior row, treats the incoming
/// write as dominating per ADR-0048 § 3 (the operator's typed write
/// is the self-healing path).
fn apply_node_health_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &NodeHealthRow,
) -> Result<bool, ObservationStoreError> {
    let key = incoming.node_id.as_str().as_bytes();
    let dominates = table.get(key).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<NodeHealthRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.last_heartbeat.dominates(&prior_row.last_heartbeat),
            Err(_) => true,
        }
    });
    if dominates {
        // Wrap into the versioned envelope at the write boundary per
        // ADR-0048 § 1 — the on-disk shape is the envelope, never the
        // bare payload.
        let envelope = NodeHealthRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key, bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `ServiceBackendRow`. Keyed on `ServiceId`
/// alone — one row per service. Mirrors [`apply_alloc_status_lww`].
/// On envelope decode failure of the prior row, treats the incoming
/// write as dominating per ADR-0048 § 3 (the operator's typed write
/// is the self-healing path). GH #160.
fn apply_service_backends_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &ServiceBackendRow,
) -> Result<bool, ObservationStoreError> {
    let key = encode_service_backends_key(incoming.service_id);
    let dominates = table.get(key.as_slice()).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<ServiceBackendRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.updated_at.dominates(&prior_row.updated_at),
            Err(_) => true,
        }
    });
    if dominates {
        // Wrap into the versioned envelope at the write boundary per
        // ADR-0048 § 1 — the on-disk shape is the envelope, never the
        // bare payload.
        let envelope = ServiceBackendRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `ProbeResultRow`. Keyed on the composite
/// `(alloc_id, probe_idx)` per ADR-0054 §5. Strictly-dominate on
/// `last_observed_at_unix_ms` — equal timestamps are no-ops
/// (idempotent re-write). On envelope decode failure of the prior
/// row, treats the incoming write as dominating per ADR-0048 § 3.
fn apply_probe_result_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &ProbeResultRow,
) -> Result<bool, ObservationStoreError> {
    let key = encode_probe_result_key(&incoming.alloc_id, incoming.probe_idx);
    let dominates = table.get(key.as_slice()).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<ProbeResultRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.last_observed_at_unix_ms > prior_row.last_observed_at_unix_ms,
            Err(_) => true,
        }
    });
    if dominates {
        let envelope = ProbeResultRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `ServiceHydrationResultRow`. Keyed on
/// `(service_id, fingerprint)` per architecture.md § 12. Mirrors the
/// [`apply_alloc_status_lww`] / [`apply_node_health_lww`] shape. On
/// envelope decode failure of the prior row, treats the incoming
/// write as dominating per ADR-0048 § 3 (the operator's typed write
/// is the self-healing path).
fn apply_service_hydration_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &ServiceHydrationResultRow,
) -> Result<bool, ObservationStoreError> {
    let key = encode_service_hydration_key(incoming.service_id, incoming.fingerprint);
    let dominates = table.get(key.as_slice()).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<ServiceHydrationResultRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.updated_at.dominates(&prior_row.updated_at),
            Err(_) => true,
        }
    });
    if dominates {
        // Wrap into the versioned envelope at the write boundary per
        // ADR-0048 § 1 — the on-disk shape is the envelope, never the
        // bare payload.
        let envelope = ServiceHydrationResultRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// LWW-guarded insert for `ReconcileConflictRow`. Keyed on the
/// composite `(service_id, vip, port, proto)` slot per Fix C. Mirrors
/// [`apply_service_hydration_lww`]. On envelope decode failure of the
/// prior row, treats the incoming write as dominating per ADR-0048 § 3.
fn apply_reconcile_conflict_lww(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &ReconcileConflictRow,
) -> Result<bool, ObservationStoreError> {
    let key = encode_reconcile_conflict_key(
        incoming.service_id,
        incoming.vip,
        incoming.port,
        incoming.proto,
    );
    let dominates = table.get(key.as_slice()).map_err(map_to_io)?.is_none_or(|prior| {
        match decode_envelope::<ReconcileConflictRowEnvelope>(prior.value()) {
            Ok(prior_row) => incoming.updated_at.dominates(&prior_row.updated_at),
            Err(_) => true,
        }
    });
    if dominates {
        // Wrap into the versioned envelope at the write boundary per
        // ADR-0048 § 1 — the on-disk shape is the envelope, never the
        // bare payload. Goes through `::latest(...)` (NOT `::V1(...)`)
        // per the dst_lint variant-construction clause.
        let envelope = ReconcileConflictRowEnvelope::latest(incoming.clone());
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope).map_err(map_to_io)?;
        table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
    }
    Ok(dominates)
}

/// Append-only insert for `IssuedCertificateRow` (`issued_certificates`
/// audit table, ADR-0063 D6). Unlike the LWW siblings there is no
/// `updated_at` to compare — the audit surface is append-only, keyed on
/// the issued certificate's serial bytes: the **first** row written at a
/// given serial is immutable and is never overwritten.
///
/// A serial already present in the table is therefore a no-op. Serials
/// are CSPRNG-drawn, so a collision is not an expected LWW case — it is
/// an issuance replay, an issuance-path retry/bug, or (once
/// `issued_certificates` is gossiped, GH #36) the idempotent
/// re-delivery that every other observation row already tolerates. In
/// all cases the correct behaviour is identical: keep the original row.
///
/// Returns `true` only when a fresh serial is inserted; `false` on a
/// duplicate. The `false` return suppresses the caller's post-commit
/// emit (the `write` path above), mirroring the LWW-reject path — a
/// serial already broadcast is never re-broadcast.
///
/// The row's bytes go through the typed co-located codec
/// [`IssuedCertificateRow::archive_for_store`] (ADR-0048): wraps in the
/// latest envelope and rkyv-serialises to canonical bytes. The on-disk
/// shape is the envelope, never the bare payload.
fn apply_issued_certificate(
    table: &mut Table<'_, &[u8], &[u8]>,
    incoming: &IssuedCertificateRow,
) -> Result<bool, ObservationStoreError> {
    let key = incoming.serial.as_str().as_bytes().to_vec();
    // Enforce the append-only contract: a serial already in the audit
    // table is never overwritten. Read-before-write is collision-free
    // under redb's serializable isolation (the same TOCTOU-safety the
    // LWW siblings rely on). Return Ok(false) so the post-commit emit is
    // suppressed — mirrors the LWW-reject path.
    if table.get(key.as_slice()).map_err(map_to_io)?.is_some() {
        return Ok(false);
    }
    let bytes = incoming.archive_for_store().map_err(ObservationStoreError::from)?;
    table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
    Ok(true)
}

/// Thin `Unpin` wrapper so we can return a `Box<dyn Stream + Unpin>`.
struct SubscriptionStream {
    inner: Pin<Box<dyn Stream<Item = ObservationRow> + Send>>,
}

impl Stream for SubscriptionStream {
    type Item = ObservationRow;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

// -----------------------------------------------------------------------------
// Error mapping helper — collapses every redb / rkyv / tokio error class
// onto `ObservationStoreError::Io`. The outer trait only distinguishes
// `Unreachable` (gossip) from `Io` — Phase 2's Corrosion impl will grow
// the error surface as needed; Phase 1 folds low-level failures into
// `Io`.
//
// Generic over any `std::error::Error + Send + Sync + 'static` source
// so the eight distinct concrete error types (`redb::DatabaseError`,
// `redb::TransactionError`, `redb::TableError`, `redb::StorageError`,
// `redb::CommitError`, `tokio::task::JoinError`, and the two
// `rkyv::rancor::Error` lanes) route through one definition instead of
// eight type-specialised stubs. The function-pointer coercion
// (`map_err(map_to_io)`) requires a concrete fn type at each call
// site; turbofishing the generic parameter pins it.
// -----------------------------------------------------------------------------

fn map_to_io<E>(err: E) -> ObservationStoreError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ObservationStoreError::Io(std::io::Error::other(err))
}
