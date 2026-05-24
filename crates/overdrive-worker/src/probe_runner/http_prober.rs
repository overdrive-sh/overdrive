//! `HyperHttpProber` — production binding of `HttpProber` over
//! `hyper-util::client::legacy::Client` + `tokio::time::timeout`.
//!
//! Per ADR-0054 §4 + DDD-20: `hyper-util` 1.x already in workspace
//! graph as transitive; promoted to direct ref. GET only per
//! US-02; no redirect-follow (3xx → Fail) per US-02 AC + research
//! § 6.1 Pitfall 5.
//!
//! RED scaffold — `probe()` body lands in slice 02.
// SCAFFOLD: true

#![allow(dead_code)]
#![expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice-02")]
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
use overdrive_core::traits::prober::{HttpProber, ProbeFailure, ProbeOutcome};

/// Production `HttpProber` over `hyper-util`.
pub struct HyperHttpProber;

impl HyperHttpProber {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for HyperHttpProber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpProber for HyperHttpProber {
    async fn probe(&self, url: &str, timeout: Duration) -> Result<ProbeOutcome, ProbeFailure> {
        let _ = (url, timeout);
        todo!("RED scaffold: HyperHttpProber::probe — hyper-util GET + status classify in slice-02")
    }
}
