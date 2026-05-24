//! `TokioTcpProber` — production binding of `TcpProber` over
//! `tokio::net::TcpStream` + `tokio::time::timeout`.
//!
//! Per ADR-0054 §4: real socket per attempt; immediate drop on
//! handshake success (no data sent or expected).
//!
//! RED scaffold — `probe()` body lands in slice 01.
// SCAFFOLD: true

#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice-01")]
#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::struct_field_names,
    reason = "DISTILL RED scaffold; per `.claude/rules/testing.md` § 'RED scaffolds' lints land when DELIVER replaces todo!() bodies + rewrites docs"
)]

use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome, TcpProber};

/// Production `TcpProber` over `tokio::net`.
pub struct TokioTcpProber;

impl TokioTcpProber {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for TokioTcpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TcpProber for TokioTcpProber {
    async fn probe(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (host, port, timeout);
        todo!("RED scaffold: TokioTcpProber::probe — real TcpStream::connect + timeout in slice-01")
    }
}
