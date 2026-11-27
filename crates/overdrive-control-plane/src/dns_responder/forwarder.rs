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
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::traits::clock::Clock;

use super::owned_ranges::{OwnedAddressRanges, OwnedRangeKind};
use super::upstream::{
    DnsTransport, ExchangeError, NoUsableUpstream, UpstreamConfigError, UpstreamExchange,
    UpstreamResolvers, UpstreamSet,
};
use super::wire::{ReplyFacts, blocked_reply, inspect_reply, question_name, servfail_reply};

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
        let upstreams = match self.upstreams().await {
            Ok(upstreams) => upstreams,
            Err(cause) => return Self::failed(request, cause),
        };
        match self.first_accepted_reply(&upstreams, request, deadline).await {
            Ok(reply) => self.deliver(reply, request),
            Err(cause) => Self::failed(request, cause),
        }
    }

    /// The upstreams to try, or why there are none.
    async fn upstreams(&self) -> Result<UpstreamSet, ForwardFailure> {
        let candidates = match self.resolvers.candidates().await {
            Ok(candidates) => candidates,
            Err(UpstreamConfigError::Absent) => return Err(ForwardFailure::ConfigAbsent),
            Err(UpstreamConfigError::Unreadable { source }) => {
                return Err(ForwardFailure::ConfigUnreadable { kind: source.kind() });
            }
        };
        if candidates.is_empty() {
            return Err(ForwardFailure::NoNameserverListed);
        }
        UpstreamSet::select(candidates, self.gateway)
            .map_err(|NoUsableUpstream| ForwardFailure::NoUsableUpstream)
    }

    /// Tries each upstream in order while time remains; the first reply whose
    /// addresses could be inspected wins, whatever its rcode.
    async fn first_accepted_reply(
        &self,
        upstreams: &UpstreamSet,
        request: &ForwardRequest<'_>,
        deadline: Instant,
    ) -> Result<AcceptedReply, ForwardFailure> {
        let mut last = None;
        for server in &upstreams.servers {
            let remaining = deadline.saturating_duration_since(self.clock.now());
            if remaining.is_zero() {
                return Err(ForwardFailure::Attempts { last, budget_exhausted: true });
            }
            let timeout = UPSTREAM_ATTEMPT_TIMEOUT.min(remaining);
            match self.attempt(*server, request, timeout).await {
                Ok(reply) => return Ok(reply),
                Err(failure) => last = Some(failure),
            }
        }
        Err(ForwardFailure::Attempts { last, budget_exhausted: false })
    }

    /// One exchange with one upstream, its reply projected for inspection.
    async fn attempt(
        &self,
        server: IpAddr,
        request: &ForwardRequest<'_>,
        timeout: Duration,
    ) -> Result<AcceptedReply, AttemptFailure> {
        let bytes = self
            .exchange
            .exchange(server, request.transport, request.wire, timeout)
            .await
            .map_err(AttemptFailure::from)?;
        let facts = inspect_reply(&bytes).map_err(|_| AttemptFailure::UndecodableReply)?;
        Ok(AcceptedReply { server, bytes, facts })
    }

    /// The owned-range guard: a reply with an owned address in its answer
    /// section is emptied whole and marked Blocked; any other reply is relayed
    /// byte for byte.
    fn deliver(&self, reply: AcceptedReply, request: &ForwardRequest<'_>) -> ForwardOutcome {
        let owned = reply
            .facts
            .answer_addresses
            .iter()
            .find_map(|address| self.owned.owning_range(*address).map(|range| (*address, range)));
        let Some((address, range)) = owned else {
            return ForwardOutcome::Relayed(reply.bytes);
        };
        tracing::debug!(
            name: "dns.forward.owned_range_blocked",
            qname = %question_name(request.wire),
            address = %address,
            range = range.as_str(),
            upstream = %reply.server,
        );
        ForwardOutcome::OwnedRangeBlocked {
            reply: blocked_reply(&reply.bytes, request.wire),
            address,
            range,
        }
    }

    fn failed(request: &ForwardRequest<'_>, cause: ForwardFailure) -> ForwardOutcome {
        ForwardOutcome::Failed { reply: servfail_reply(request.wire), cause }
    }
}

/// A reply an upstream gave and whose addresses could be inspected.
struct AcceptedReply {
    server: IpAddr,
    bytes: Vec<u8>,
    facts: ReplyFacts,
}

impl From<ExchangeError> for AttemptFailure {
    fn from(error: ExchangeError) -> Self {
        match error {
            ExchangeError::Timeout => Self::Timeout,
            ExchangeError::Unreachable { source } => Self::Unreachable { kind: source.kind() },
            ExchangeError::Io { source } => Self::Io { kind: source.kind() },
            ExchangeError::ReplyTooLarge => Self::ReplyTooLarge,
        }
    }
}

impl ForwardFailure {
    /// The canonical label; `budget_exhausted` selects the `Attempts` label.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigAbsent => "config_absent",
            Self::ConfigUnreadable { .. } => "config_unreadable",
            Self::NoNameserverListed => "no_nameserver_listed",
            Self::NoUsableUpstream => "no_usable_upstream",
            Self::Attempts { budget_exhausted: true, .. } => "budget_exhausted",
            Self::Attempts { budget_exhausted: false, .. } => "all_upstreams_failed",
        }
    }
}

impl AttemptFailure {
    /// The canonical label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unreachable { .. } => "unreachable",
            Self::Io { .. } => "io",
            Self::ReplyTooLarge => "reply_too_large",
            Self::UndecodableReply => "undecodable_reply",
        }
    }
}
