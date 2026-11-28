//! `dns_responder` — the in-agent dial-by-name DNS layer (ADR-0072, GH #243).
//!
//! The responder is the **third reader** of the `ObservationStore`
//! `service_backends` surface (outbound resolve · inbound install · name
//! answers, D-TME-11) — it answers `<workload>.svc.overdrive.local` queries with a
//! running-AND-healthy IPv4 backend addr, NODATA for AAAA-on-live, and
//! NXDOMAIN when no running-and-healthy backend exists.
//!
//! # Module map
//!
//! - `wire` — the `hickory-proto` DNS codec behind the anti-corruption
//!   boundary: decode and classify the inbound query; encode the A /
//!   NODATA-SOA / NXDOMAIN-SOA and SERVFAIL replies.
//! - `answer` — the pure `answer_for(name, qtype, &index) -> NameAnswer`.
//! - `name_index` — the List-then-Watch `NameIndex` mapping each resolvable
//!   `<workload>` to its stable frontend address.
//! - `frontend_addr_allocator` / `boot_rebuild` — the shared frontend
//!   allocator and its converge-on-boot rebuild from declared-Service intent.
//! - `responder` — the `DnsResponder` host adapter: UDP `:53` and TCP
//!   `gateway:53` serve loops behind the Earned-Trust probe gate.
//! - `forwarder` / `upstream` / `upstream_host` / `owned_ranges` — forwarding
//!   of public names: permits and the owned-range answer guard, the upstream
//!   ports, their host adapters, and the Overdrive-owned address ranges.

pub mod answer;
pub mod boot_rebuild;
pub mod forwarder;
pub mod frontend_addr_allocator;
pub mod name_index;
pub mod owned_ranges;
pub mod responder;
pub mod upstream;
pub mod upstream_host;
pub mod wire;

use std::net::Ipv4Addr;
use std::sync::Arc;

use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;

use self::forwarder::Forwarder;
use self::frontend_addr_allocator::FrontendAddrAllocator;
use self::responder::{DnsResponder, Result};
use self::upstream::{UpstreamExchange, UpstreamResolvers};
use self::upstream_host::{HostUpstreamExchange, RESOLV_CONF_PATH, ResolvConfUpstreams};

/// The shared-gateway DNS owner port (D-295-R16): one responder the serve
/// task owner probes, serves, audits, and stops.
///
/// Production binds it to [`DnsResponder`] through [`HostGuestDnsFactory`];
/// the DST double is `overdrive_sim::adapters::SimGuestDns`.
#[doc(hidden)]
#[async_trait::async_trait]
pub trait GuestDns: Send + Sync {
    /// Earned-Trust gate: bind the responder's sockets and seed its index.
    /// `Err` refuses boot or fails a replacement.
    async fn probe(&self) -> Result<()>;
    /// Run the serve loop until [`stop`](Self::stop) or a terminal failure.
    async fn serve(self: Arc<Self>);
    /// Non-mutating read-back of the socket identities `probe` recorded.
    async fn audit(&self) -> Result<()>;
    /// Signal the serve loop to stop. Idempotent.
    fn stop(&self);
}

/// The construction dependencies of one [`GuestDns`] responder.
#[doc(hidden)]
pub struct GuestDnsDeps {
    /// The observation store the responder's name index reads.
    pub store: Arc<dyn ObservationStore>,
    /// The injected clock (SOA serial source).
    pub clock: Arc<dyn Clock>,
    /// The shared bridge gateway used for the fallback bind.
    pub gateway: Ipv4Addr,
    /// The one shared frontend-address allocator.
    pub frontend: FrontendAddrAllocator,
    /// The Overdrive-owned address ranges the forwarded-answer guard checks.
    pub owned_ranges: owned_ranges::OwnedAddressRanges,
    /// The in-flight forward permit count (derived from the descriptor limit).
    pub forward_permits: usize,
}

/// Builds the [`GuestDns`] responders the serve task owner runs: one at boot,
/// and a fresh one for every recovery replacement.
#[doc(hidden)]
pub trait GuestDnsFactory: Send + Sync {
    /// Build one fresh responder over `deps`.
    fn responder(&self, deps: GuestDnsDeps) -> Arc<dyn GuestDns>;
}

/// The production [`GuestDnsFactory`]: builds a [`DnsResponder`] that forwards
/// public names through the host's resolver file and sockets.
#[doc(hidden)]
pub struct HostGuestDnsFactory {
    resolvers: Arc<dyn UpstreamResolvers>,
    exchange: Arc<dyn UpstreamExchange>,
}

impl HostGuestDnsFactory {
    /// The production factory: upstreams come from `/etc/resolv.conf` and are
    /// queried over real UDP and TCP sockets.
    #[must_use]
    pub fn host() -> Self {
        Self {
            resolvers: Arc::new(ResolvConfUpstreams::from_path(RESOLV_CONF_PATH)),
            exchange: Arc::new(HostUpstreamExchange),
        }
    }
}

impl GuestDnsFactory for HostGuestDnsFactory {
    fn responder(&self, deps: GuestDnsDeps) -> Arc<dyn GuestDns> {
        let forwarder = Forwarder::new(
            Arc::clone(&self.resolvers),
            Arc::clone(&self.exchange),
            Arc::clone(&deps.clock),
            deps.owned_ranges,
            deps.gateway,
            deps.forward_permits,
        );
        Arc::new(DnsResponder::new(
            deps.store,
            deps.clock,
            deps.gateway,
            deps.frontend,
            Arc::new(NoQueryObserver),
            Arc::new(forwarder),
        ))
    }
}

#[async_trait::async_trait]
impl GuestDns for DnsResponder {
    async fn probe(&self) -> Result<()> {
        Self::probe(self).await
    }

    async fn serve(self: Arc<Self>) {
        Self::serve(self).await;
    }

    async fn audit(&self) -> Result<()> {
        Self::audit(self).await
    }

    fn stop(&self) {
        Self::stop(self);
    }
}

/// Receives the service time of every datagram the responder handles.
pub trait QueryObserver: Send + Sync {
    /// Called once per datagram the responder handles, on the serving thread.
    /// `service_time` runs from the instant the receive returned to the instant
    /// the reply send (or the drop decision) completed, on the injected `Clock`.
    /// Must not block, panic or allocate unboundedly.
    fn observe(&self, outcome: QueryOutcome, service_time: std::time::Duration);
}

/// What the responder did with one datagram.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryOutcome {
    /// A resolvable name was answered with an address.
    Answered,
    /// The name resolves but has no record of the queried type.
    NoData,
    /// The name is unknown.
    NameError,
    /// The datagram was not answered.
    Dropped,
    /// A public name was forwarded and the upstream's reply relayed.
    Forwarded,
    /// A public name was forwarded and no upstream answered; the guest got a
    /// SERVFAIL.
    ForwardFailed,
    /// The upstream's reply carried an Overdrive-owned address; the guest got
    /// the blocked reply.
    OwnedRangeBlocked,
    /// No forward permit was free; the guest got a SERVFAIL at once. Answered,
    /// unlike [`Self::Dropped`].
    Shed,
}

/// The production observer: an empty body. Behaviour with it is byte-identical
/// to the responder before the observer existed.
pub struct NoQueryObserver;

impl QueryObserver for NoQueryObserver {
    fn observe(&self, _outcome: QueryOutcome, _service_time: std::time::Duration) {}
}
