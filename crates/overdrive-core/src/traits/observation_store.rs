//! [`ObservationStore`] — live eventually-consistent cluster map.
//!
//! Allocation status, service backends, node health, compiled policy
//! verdicts. Every node writes its own rows; every node reads locally.
//! Production uses Corrosion (cr-sqlite + SWIM/QUIC); simulation uses
//! `SimObservationStore` with an injectable gossip-delay and partition
//! matrix.
//!
//! See `docs/whitepaper.md` §4 (Intent / Observation split) and §17
//! (storage rationale).

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObservationStoreError {
    #[error("query rejected: {0}")]
    Query(String),
    #[error("gossip peer {peer} unreachable")]
    Unreachable { peer: String },
    #[error("observation store I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// A single SQL parameter value — covers the scalar types CR-SQLite
/// supports natively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Rows returned from [`ObservationStore::read`]. Column order matches the
/// query; callers interpret values positionally.
#[derive(Debug, Clone)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

#[async_trait]
pub trait ObservationStore: Send + Sync + 'static {
    async fn read(&self, sql: &str, params: &[Value]) -> Result<Rows, ObservationStoreError>;

    async fn write(&self, sql: &str, params: &[Value]) -> Result<(), ObservationStoreError>;

    /// Live subscription to rows matching `sql`. Every row-change event
    /// yields a full row (§4 guardrail: full rows over field diffs).
    async fn subscribe(
        &self,
        sql: &str,
    ) -> Result<Box<dyn Stream<Item = Rows> + Send + Unpin>, ObservationStoreError>;
}
