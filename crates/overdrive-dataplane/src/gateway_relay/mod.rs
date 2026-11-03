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

#[cfg(test)]
mod pipe_floor_laws {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        reason = "CONTRACT_SHAPE is required metadata; expect names the precondition of a fixture"
    )]

    use proptest::prelude::*;

    use super::*;

    /// S-PIG-86 (the constant): the pipe floor is 65 536 octets, the capacity
    /// the relay's record-boundary and backpressure rules are sized for
    /// (ADR-0246). The DESIGN pins the value, so this row is expected-GREEN on
    /// the scaffold.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_pipe_floor_is_sixty_five_thousand_five_hundred_and_thirty_six_octets() {
        assert_eq!(PIPE_FLOOR, 65_536);
    }

    /// S-PIG-86 (the read-back): a capacity read back at the floor or above is a
    /// `PipeCapacity` holding exactly that value; one octet below the floor, and
    /// every smaller value, is `CapacityBelowFloor` carrying what was read.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER kernel-relay-host-adapter"]
    fn a_read_back_below_the_floor_is_refused_with_what_was_read() {
        for at_or_above in [PIPE_FLOOR, PIPE_FLOOR + 1, 2 * PIPE_FLOOR, 1 << 20] {
            let capacity =
                PipeCapacity::new(at_or_above).expect("a capacity at or above the floor");
            assert_eq!(capacity.get(), at_or_above);
        }
        for below in [0, 1, 4096, PIPE_FLOOR - 1] {
            assert_eq!(
                PipeCapacity::new(below).err(),
                Some(RelayPipeError::CapacityBelowFloor { got: below }),
                "{below} is below the floor",
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-86 (the splice-in length): for every octet count owed and every
        /// capacity at or above the floor, the next splice into an empty pipe is
        /// exactly `min(owed, capacity)`, never above either and never zero.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER kernel-relay-host-adapter"]
        fn the_splice_in_length_is_the_smaller_of_what_is_owed_and_the_capacity(
            owed in 1u64..=u64::MAX,
            capacity in PIPE_FLOOR..=(64usize << 20),
        ) {
            let owed = NonZeroU64::new(owed).expect("owed is nonzero");
            let capacity = PipeCapacity::new(capacity).expect("a capacity at or above the floor");
            let held = capacity.get() as u64;
            let length = splice_in_len(owed, capacity).get();
            prop_assert_eq!(length, owed.get().min(held));
            prop_assert!(length <= owed.get());
            prop_assert!(length <= held);
        }

        /// S-PIG-86 (the boundary around the floor): a body one octet below, at
        /// and one octet above the pipe capacity splices in whole, whole, and
        /// capacity octets respectively (the remainder is the next splice).
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER kernel-relay-host-adapter"]
        fn bodies_around_the_capacity_splice_in_whole_or_capacity_octets(
            capacity in PIPE_FLOOR..=(1usize << 20),
        ) {
            let held = capacity as u64;
            for (owed, expected) in [(held - 1, held - 1), (held, held), (held + 1, held)] {
                let owed = NonZeroU64::new(owed).expect("owed is nonzero");
                let pipe = PipeCapacity::new(capacity).expect("a capacity at or above the floor");
                prop_assert_eq!(splice_in_len(owed, pipe).get(), expected);
            }
        }
    }
}
