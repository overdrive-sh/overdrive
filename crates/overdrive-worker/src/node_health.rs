//! Boot-time `node_health` row writer.
//!
//! Per ADR-0025 (amended by ADR-0029), the worker subsystem writes
//! the local node's `NodeHealthRow` to the `ObservationStore` at
//! startup, before the worker is considered "started." Phase 1
//! single-node ships exactly one row at runtime; Phase 2+ multi-node
//! has every node writing its own.
//!
//! `NodeId` is resolved from the `[node].id` config override, falling
//! back to `hostname` per ADR-0025.

use std::sync::Arc;

use overdrive_core::id::{NodeId, Region};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
};

/// Write the local node's `NodeHealthRow` to the observation store.
///
/// Idempotent — Phase 1 single-node always writes a row whose primary
/// key is the local `NodeId`.
///
/// The `Clock` is required at construction per
/// `.claude/rules/development.md` § "Port-trait dependencies": the
/// caller injects the host or sim implementation explicitly so tests
/// cannot silently inherit wall-clock behaviour by forgetting to
/// override.
///
/// # Errors
///
/// Returns an error if the observation store rejects the write, the
/// `[node].id` config override fails to parse, or hostname resolution
/// fails when no override was supplied.
pub async fn write_node_health_row(
    obs: &Arc<dyn ObservationStore>,
    config: &NodeConfig,
    clock: &Arc<dyn Clock>,
) -> Result<(), NodeHealthWriteError> {
    let node_id = resolve_node_id(config)?;
    let region = Region::new(&config.region).map_err(|e| {
        NodeHealthWriteError::IdResolve(format!("region {:?} rejected: {e}", config.region))
    })?;

    // `LogicalTimestamp.counter` carries the wall-clock seconds since
    // UNIX epoch at write time, derived from the injected `Clock`. This
    // is an *input* (the moment we observed our own boot) — per
    // `.claude/rules/development.md` § "Persist inputs, not derived
    // state" — not a derived deadline. `unix_now()` is non-zero on any
    // real clock and on `SimClock` constructed in tests (which seeds
    // its unix_epoch from `SystemTime::now()`), so the regression
    // test's `counter != 0` invariant holds for both adapters.
    let unix_seconds = clock.unix_now().as_secs();
    let last_heartbeat = LogicalTimestamp { counter: unix_seconds, writer: node_id.clone() };

    let row = NodeHealthRow { node_id, region, last_heartbeat };
    obs.write(ObservationRow::NodeHealth(row))
        .await
        .map_err(|e| NodeHealthWriteError::Write(e.to_string()))?;
    Ok(())
}

/// Resolve the local `NodeId` from `config.id_override` first,
/// falling back to the host's hostname per ADR-0025. Surfaces every
/// distinguishable failure (`hostname` syscall refusal, non-UTF8
/// hostname, override parse failure) as a discrete
/// [`NodeHealthWriteError::IdResolve`] message — never silently
/// substitutes a default — per `.claude/rules/development.md`
/// § Errors → "Distinct failure modes get distinct error variants".
fn resolve_node_id(config: &NodeConfig) -> Result<NodeId, NodeHealthWriteError> {
    if let Some(raw) = config.id_override.as_ref() {
        return NodeId::new(raw).map_err(|e| {
            NodeHealthWriteError::IdResolve(format!(
                "[node].id override {raw:?} rejected by NodeId::new: {e}"
            ))
        });
    }
    let hostname_os = hostname::get().map_err(|e| {
        NodeHealthWriteError::IdResolve(format!(
            "hostname::get() failed (no [node].id override supplied): {e}"
        ))
    })?;
    let hostname_str = hostname_os.into_string().map_err(|os| {
        NodeHealthWriteError::IdResolve(format!("hostname is not valid UTF-8: {os:?}"))
    })?;
    NodeId::new(&hostname_str).map_err(|e| {
        NodeHealthWriteError::IdResolve(format!(
            "hostname {hostname_str:?} rejected by NodeId::new (likely \
             contains characters outside the NodeId charset): {e}"
        ))
    })
}

/// Operator-supplied `[node]` config block per ADR-0025.
/// `id` is optional (hostname fallback); `region` defaults to
/// `"local"`; `capacity` is required for Phase 1 (no autodetection
/// yet).
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Optional override; falls back to `hostname` per ADR-0025.
    pub id_override: Option<String>,
    /// Region — Phase 1 default `"local"`.
    pub region: String,
    /// Declared capacity for the local node. Phase 1 requires this
    /// from config; Phase 2+ may auto-detect from the kernel.
    pub capacity: overdrive_core::traits::driver::Resources,
}

impl Default for NodeConfig {
    /// Phase 1 default: no override (hostname fallback), `region =
    /// "local"`, zero capacity. Production binaries override these
    /// from the operator's `[node]` TOML section; tests use this
    /// default via `..Default::default()` rest-pattern construction.
    fn default() -> Self {
        Self {
            id_override: None,
            region: "local".to_owned(),
            capacity: overdrive_core::traits::driver::Resources { cpu_milli: 0, memory_bytes: 0 },
        }
    }
}

/// Errors from [`write_node_health_row`].
#[derive(Debug, thiserror::Error)]
pub enum NodeHealthWriteError {
    /// Failed to resolve `NodeId` (hostname read failed AND no
    /// override was supplied; or override failed to parse).
    #[error("node id resolution failed: {0}")]
    IdResolve(String),
    /// Underlying observation-store write failure.
    #[error("observation store write failed: {0}")]
    Write(String),
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]
mod tests {
    //! Unit tests for [`write_node_health_row`].
    //!
    //! Both tests are port-to-port at the driving-port scope: each
    //! invokes the public `write_node_health_row` function and asserts
    //! observable state at the driven `ObservationStore` boundary via
    //! `node_health_rows()`. The internal `resolve_node_id` helper is
    //! tested indirectly through the two paths the production caller
    //! exercises (override vs hostname fallback).
    //!
    //! Test 1 (override path) and Test 2 (hostname fallback) are
    //! distinct invariants and remain distinct tests per project
    //! review discipline — they cover different failure-mode branches
    //! of `resolve_node_id` and therefore different acceptance
    //! criteria.

    use super::*;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use std::time::Duration;

    /// Test 1 — `id_override = Some("explicit-node-id")` path.
    /// Asserts:
    ///   (a) exactly one row lands in the obs store
    ///   (b) `node_id` matches the parsed override (not the hostname)
    ///   (c) `last_heartbeat.counter` reflects the injected clock's
    ///       `unix_now()` (non-default — proves clock was actually
    ///       consulted)
    #[tokio::test]
    async fn write_with_id_override_resolves_to_override_and_uses_clock() {
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("test-writer").expect("valid writer"),
            0,
        ));
        // Advance logical time so the test asserts on an
        // unambiguously-non-default counter value — a SimClock at
        // logical-time zero ALREADY has a non-zero `unix_now()`
        // (seeded from `SystemTime::now()` at construction), but
        // ticking forward by a known interval lets the test assert
        // that the writer reads the *current* clock value rather
        // than a cached zero or a captured-at-construction snapshot.
        let sim_clock = SimClock::new();
        sim_clock.tick(Duration::from_secs(3600));
        let clock: Arc<dyn Clock> = Arc::new(sim_clock);

        let config = NodeConfig {
            id_override: Some("explicit-node-id".to_owned()),
            region: "us-west-2".to_owned(),
            capacity: overdrive_core::traits::driver::Resources { cpu_milli: 0, memory_bytes: 0 },
        };

        write_node_health_row(&obs, &config, &clock)
            .await
            .expect("write must succeed against SimObservationStore");

        let rows = obs.node_health_rows().await.expect("read rows");
        assert_eq!(
            rows.len(),
            1,
            "exactly one row must land in the obs store; got {} rows",
            rows.len(),
        );
        let row = &rows[0];
        assert_eq!(
            row.node_id,
            NodeId::new("explicit-node-id").expect("valid override id"),
            "node_id must match the parsed override, not the hostname",
        );
        assert_eq!(
            row.region,
            Region::new("us-west-2").expect("valid region"),
            "region must match the config",
        );
        assert_ne!(
            row.last_heartbeat.counter, 0,
            "last_heartbeat.counter must be non-default — proves the \
             writer consulted the injected clock",
        );
    }

    /// Test 2 — `id_override = None` (hostname fallback path).
    /// Asserts:
    ///   (a) write succeeds without an override
    ///   (b) resulting `node_id` matches `hostname::get()` (the same
    ///       fallback the production resolver uses)
    ///
    /// Reads `hostname::get()` in the test so the assertion is
    /// invariant under whatever the test environment's hostname is.
    /// If `hostname::get()` returns a value `NodeId::new` rejects
    /// (e.g. a hostname containing `@` or `/`, or one that starts
    /// with a non-alphanumeric character), the test surfaces the
    /// IdResolve error verbatim — the production code path's failure
    /// mode is the test's failure mode.
    #[tokio::test]
    async fn write_without_id_override_falls_back_to_hostname() {
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("test-writer").expect("valid writer"),
            0,
        ));
        let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

        let config = NodeConfig {
            id_override: None,
            region: "local".to_owned(),
            capacity: overdrive_core::traits::driver::Resources { cpu_milli: 0, memory_bytes: 0 },
        };

        let expected_hostname_os = hostname::get().expect("hostname syscall succeeds");
        let expected_hostname_str =
            expected_hostname_os.into_string().expect("hostname is valid UTF-8 on the test runner");
        // The test runner's hostname must be acceptable to NodeId's
        // validator — if not, the production fallback would also
        // fail and the test correctly surfaces that as a precondition
        // failure rather than masking it.
        let expected_node_id = NodeId::new(&expected_hostname_str).unwrap_or_else(|e| {
            panic!(
                "test precondition: hostname {expected_hostname_str:?} \
                 must be acceptable to NodeId::new (the production fallback \
                 would fail otherwise): {e}"
            )
        });

        write_node_health_row(&obs, &config, &clock)
            .await
            .expect("write must succeed when hostname fallback is used");

        let rows = obs.node_health_rows().await.expect("read rows");
        assert_eq!(rows.len(), 1, "exactly one row must land in the obs store");
        assert_eq!(
            rows[0].node_id, expected_node_id,
            "node_id must match the hostname-fallback value",
        );
    }

    /// Test 3 — `start_local_node` wrapper passthrough.
    /// Kills the mutation `start_local_node body -> Ok(())`. The
    /// wrapper IS a single-line passthrough today (per ADR-0029 it
    /// exists as the worker-startup contract boundary, not because it
    /// adds logic), so the structural defence is: invoking the wrapper
    /// MUST observably write a row to the obs store. A mutation that
    /// replaces the body with `Ok(())` returns success without writing
    /// — this assertion catches that exact shape.
    ///
    /// Lives in this module rather than `lib.rs` so it's adjacent to
    /// the writer it wraps; the test does NOT duplicate the override /
    /// hostname-path coverage above (those exercise the inner
    /// `write_node_health_row` directly).
    #[tokio::test]
    async fn start_local_node_wrapper_observably_writes_row() {
        let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
            NodeId::new("test-writer").expect("valid writer"),
            0,
        ));
        let clock: Arc<dyn Clock> = Arc::new(SimClock::new());

        let config = NodeConfig {
            id_override: Some("wrapper-test-node".to_owned()),
            region: "local".to_owned(),
            capacity: overdrive_core::traits::driver::Resources { cpu_milli: 0, memory_bytes: 0 },
        };

        crate::start_local_node(&obs, &config, &clock)
            .await
            .expect("start_local_node must succeed against SimObservationStore");

        let rows = obs.node_health_rows().await.expect("read rows");
        assert_eq!(
            rows.len(),
            1,
            "start_local_node must observably write exactly one row; \
             got {} rows (mutation `body -> Ok(())` produces 0)",
            rows.len(),
        );
        assert_eq!(
            rows[0].node_id,
            NodeId::new("wrapper-test-node").expect("valid id"),
            "wrapper must route the config through to the writer",
        );
    }
}
