//! The host backend of the `GatewayRelay` driven port.
//!
//! `HostGatewayRelay` (G1, ADR-0244/0246) does kernel-TLS arming from a rustls
//! `KernelConnection`, control-record dispatch, the peer-KeyUpdate answer, the
//! private relay pipe, `splice` and the Earned-Trust probe.
//!
//! SCAFFOLD: true. Only the shapes the DESIGN pins are present
//! (`HostGatewayRelay::new(clock)` and the `GatewayRelay` port methods);
//! every body fails with SCAFFOLD text until DELIVER. The one `unsafe` module
//! is [`sys`]; this module and its callers stay safe code.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    clippy::needless_pass_by_ref_mut,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

mod sys;

use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayRelay, HeadTerminator, KtlsReadHalf, KtlsWriteHalf, LegReadError,
    LegWriteError, PublicHandshakeError, ReadDeadlines, RelayDeadlines, RelayError, RelayPipeError,
    RelayProbeError, ThroughBytes,
};

/// The relay pipe's capacity floor (ADR-0246): the only runtime outcomes of the
/// pipe typestate are a failed creation or a capacity that reads back below it.
pub(crate) const PIPE_FLOOR: usize = 65_536;

/// A capacity read back from `F_GETPIPE_SZ`; it cannot be below [`PIPE_FLOOR`].
pub(crate) struct PipeCapacity(usize);

impl PipeCapacity {
    /// `Err(CapacityBelowFloor { got })` below the floor.
    pub(crate) fn new(read_back: usize) -> Result<Self, RelayPipeError> {
        todo!("SCAFFOLD: PipeCapacity::new")
    }

    pub(crate) fn get(&self) -> usize {
        todo!("SCAFFOLD: PipeCapacity::get")
    }
}

/// `min(octets owed, capacity)`: the length of the next splice into an empty
/// pipe (never above either).
pub(crate) fn splice_in_len(owed: NonZeroU64, capacity: PipeCapacity) -> NonZeroU64 {
    todo!("SCAFFOLD: splice_in_len")
}

/// Host kernel-TLS relay. One instance serves both gateway legs: the public
/// legs it accepts and the authenticated upstream legs the gateway-client mTLS
/// adapter arms through it.
pub struct HostGatewayRelay {
    /// The injected clock all `ReadDeadlines` / `RelayDeadlines` are evaluated on.
    clock: Arc<dyn Clock>,
}

impl HostGatewayRelay {
    /// Infallible and kernel-silent: it opens no socket and touches no ULP, so
    /// construction claims no capability. Every kernel fact is proven by
    /// [`GatewayRelay::probe`] (Earned Trust), never assumed at construction.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

#[async_trait]
impl GatewayRelay for HostGatewayRelay {
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError> {
        todo!("SCAFFOLD: HostGatewayRelay::probe")
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        todo!("SCAFFOLD: HostGatewayRelay::accept_public")
    }

    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        todo!("SCAFFOLD: HostGatewayRelay::read_through")
    }

    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        todo!("SCAFFOLD: HostGatewayRelay::write_all")
    }

    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        todo!("SCAFFOLD: HostGatewayRelay::relay_exact")
    }

    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        todo!("SCAFFOLD: HostGatewayRelay::relay_to_end")
    }

    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        todo!("SCAFFOLD: HostGatewayRelay::close_write")
    }

    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), LegReadError> {
        todo!("SCAFFOLD: HostGatewayRelay::discard_remaining")
    }
}
