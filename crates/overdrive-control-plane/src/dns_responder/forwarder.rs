//! `forwarder` — the per-query upstream exchange, failover, owned-range guard
//! and in-flight permits (GPD-1, GPD-4, GPD-8; ADR-0356, ADR-0359, ADR-0362).
//!
//! DISTILL scaffold: shapes are the pinned contract; bodies are `todo!`.

#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::too_many_arguments,
    clippy::unnecessary_wraps,
    clippy::doc_markdown,
    dead_code,
    reason = "DISTILL scaffold: bodies are todo until DELIVER (ADR-0356, ADR-0362)"
)]

use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::traits::clock::Clock;

use super::owned_ranges::{OwnedAddressRanges, OwnedRangeKind};
use super::upstream::{DnsTransport, UpstreamExchange, UpstreamResolvers, UpstreamSet};
use super::wire::inspect_reply;

/// One upstream attempt budget.
pub const UPSTREAM_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(1500);
/// The whole forward budget.
pub const FORWARD_BUDGET: Duration = Duration::from_secs(4);
/// Descriptors one in-flight forward costs.
pub const FDS_PER_PERMIT: u64 = 2;

/// Pure: the in-flight permit count for a soft descriptor limit and a reserve.
#[must_use]
pub fn forward_permits(nofile_soft: u64, reserve: u64) -> usize {
    let budget = nofile_soft.saturating_sub(reserve) / FDS_PER_PERMIT;
    usize::try_from(budget).unwrap_or(usize::MAX)
}

/// The forwarder: one verbatim exchange per query.
pub struct Forwarder {
    resolvers: Arc<dyn UpstreamResolvers>,
    exchange: Arc<dyn UpstreamExchange>,
    clock: Arc<dyn Clock>,
    owned: OwnedAddressRanges,
    gateway: Ipv4Addr,
    permits: usize,
}

/// RAII in-flight permit. Not `Clone`. Released on drop.
pub struct ForwardPermit {
    _private: (),
}

/// One query to forward.
pub struct ForwardRequest<'a> {
    /// The guest's query bytes, sent unchanged.
    pub wire: &'a [u8],
    /// The transport the guest used.
    pub transport: DnsTransport,
}

/// What one forward produced.
#[derive(Debug)]
pub enum ForwardOutcome {
    /// The upstream's reply, byte for byte.
    Relayed(Vec<u8>),
    /// The reply carried an owned address; `reply` is the blocked reply.
    OwnedRangeBlocked {
        /// The blocked reply bytes.
        reply: Vec<u8>,
        /// The first owned address found.
        address: Ipv4Addr,
        /// The owning range.
        range: OwnedRangeKind,
    },
    /// No upstream answered; `reply` is a SERVFAIL.
    Failed {
        /// The SERVFAIL reply bytes.
        reply: Vec<u8>,
        /// Why.
        cause: ForwardFailure,
    },
}

/// Why a forward failed.
#[derive(Debug)]
pub enum ForwardFailure {
    /// The resolver file does not exist.
    ConfigAbsent,
    /// The resolver file could not be read.
    ConfigUnreadable {
        /// The OS error kind.
        kind: io::ErrorKind,
    },
    /// The resolver file lists no nameserver.
    NoNameserverListed,
    /// Every candidate was the gateway.
    NoUsableUpstream,
    /// At least one attempt was made or the deadline passed.
    Attempts {
        /// The last attempt's failure.
        last: Option<AttemptFailure>,
        /// The deadline, not the end of the list, ended the loop.
        budget_exhausted: bool,
    },
}

/// Why one attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptFailure {
    /// No reply in time.
    Timeout,
    /// Refused or ICMP error.
    Unreachable {
        /// The OS error kind.
        kind: io::ErrorKind,
    },
    /// Other socket failure.
    Io {
        /// The OS error kind.
        kind: io::ErrorKind,
    },
    /// UDP reply filled the receive buffer.
    ReplyTooLarge,
    /// The reply could not be decoded.
    UndecodableReply,
}

impl Forwarder {
    /// Build a forwarder with `permits` in-flight permits.
    #[must_use]
    pub fn new(
        resolvers: Arc<dyn UpstreamResolvers>,
        exchange: Arc<dyn UpstreamExchange>,
        clock: Arc<dyn Clock>,
        owned: OwnedAddressRanges,
        gateway: Ipv4Addr,
        permits: usize,
    ) -> Self {
        Self { resolvers, exchange, clock, owned, gateway, permits }
    }

    /// Takes one permit without waiting; `None` when all are held.
    #[must_use]
    pub fn try_acquire(&self) -> Option<ForwardPermit> {
        // SCAFFOLD: always grants a permit, so the permit bound, the shed and the
        // shed-episode warn are absent until DELIVER lands them (ADR-0362).
        let _ = self.permits;
        Some(ForwardPermit { _private: () })
    }

    /// Runs one forward under a permit the caller already holds.
    pub async fn forward(
        &self,
        request: &ForwardRequest<'_>,
        permit: &ForwardPermit,
    ) -> ForwardOutcome {
        let _ = permit;
        let deadline = self.clock.now() + FORWARD_BUDGET;
        let Ok(candidates) = self.resolvers.candidates().await else { Self::failure_scaffold() };
        let Ok(upstreams) = UpstreamSet::select(candidates, self.gateway) else {
            Self::failure_scaffold()
        };
        // Only the first upstream is tried until failover lands.
        let Some(server) = upstreams.servers.into_iter().next() else { Self::failure_scaffold() };
        let remaining = deadline.saturating_duration_since(self.clock.now());
        let timeout = UPSTREAM_ATTEMPT_TIMEOUT.min(remaining);
        let Ok(reply) =
            self.exchange.exchange(server, request.transport, request.wire, timeout).await
        else {
            Self::failure_scaffold()
        };
        let Ok(facts) = inspect_reply(&reply) else { Self::failure_scaffold() };
        if facts.answer_addresses.iter().any(|a| self.owned.owning_range(*a).is_some()) {
            Self::failure_scaffold()
        }
        ForwardOutcome::Relayed(reply)
    }

    /// Failover, SERVFAIL replies and the owned-range guard land in later steps.
    fn failure_scaffold() -> ! {
        todo!("forwarder: exchange the query verbatim (failure and guard paths are not yet built)")
    }
}
