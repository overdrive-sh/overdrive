//! The per-allocation transparent-mTLS **install** driven port (GH #250).
//!
//! Declares [`MtlsIntercept`] — the substitutable boundary over the three
//! privileged, un-ownable primitives
//! [`MtlsInterceptWorker::start_alloc`](crate::mtls_intercept_worker::MtlsInterceptWorker::start_alloc)
//! performs (the `IP_TRANSPARENT` socket setup and the two `nft`/`ip`
//! shell-outs) — plus [`InterceptGuard`], the marker trait its RAII install
//! handles satisfy, and [`HostMtlsIntercept`], the production binding.
//!
//! Production wires [`HostMtlsIntercept`], whose allocation methods delegate to
//! the existing `crate::mtls_intercept` free functions and whose shared-owner
//! methods drive the normalized nft adapter; tests wire
//! `overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept`.

#![allow(
    clippy::result_large_err,
    reason = "GH #295 exact InterceptError retains complete rollback identity"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::mtls_intercept::{
    InterceptError, InterceptLeg, InterceptPostcondition, InterceptSharedRollbackOperation,
    NetlinkError, Result, TproxyInterceptGuard, install_inbound_tproxy, make_transparent_listener,
};

/// Module-private effect seam for the shared-program observe/atomic-replace
/// algorithm. The host algorithm remains above this boundary.
#[allow(dead_code, reason = "GH #295 exact accepted scaffold; DELIVER wires the host algorithm")]
trait SharedInterceptProgramIo: Send + Sync {
    fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError>;

    fn replace_atomically(
        &self,
        expected_current: Option<&InterceptPostcondition>,
        desired: Option<&InterceptPostcondition>,
    ) -> std::result::Result<(), NetlinkError>;
}

/// RAII handle for ONE completed intercept install.
///
/// Marker-only by design: the guard's ENTIRE contract is its `Drop`, so the
/// trait exposes no callable method. The worker holds guards for the alloc
/// lifetime and drops them on `stop_alloc`; it never calls anything on them.
///
/// # Observable invariants
/// Dropping a guard releases EXACTLY what its originating
/// [`MtlsIntercept::install_outbound`] / [`MtlsIntercept::install_inbound`]
/// call acquired — no more, no less — and releases nothing another guard owns.
/// Dropping never panics and never errors, including for a guard whose
/// underlying state was already released out-of-band.
///
/// WHAT is acquired and released is adapter-specific and is NOT part of this
/// contract: [`HostMtlsIntercept`] acquires one `nft` rule and its `Drop`
/// removes that rule by handle; a simulation adapter acquires nothing and its
/// `Drop` is a no-op. Both honour the invariant above.
pub trait InterceptGuard: Send + Sync {}

impl InterceptGuard for TproxyInterceptGuard {}

/// Node-owned guard for the shared constant IP program.
///
/// The shared program's lifetime is deliberately controlled by the worker's
/// boot/shutdown owner.  A failed unpublished startup drops this marker, while
/// a published owner uses its sealed relinquish path so the constant rules and
/// empty sets remain available for the next boot's identity check.
struct SharedInterceptGuard {
    io: Arc<dyn SharedInterceptProgramIo>,
    requested: InterceptPostcondition,
    shared_targets: Arc<Mutex<Option<(u16, u16)>>>,
    shared_program: Arc<Mutex<Option<overdrive_netlink::nft::SharedIpInterceptIdentity>>>,
}

impl InterceptGuard for SharedInterceptGuard {}

impl Drop for SharedInterceptGuard {
    fn drop(&mut self) {
        // An unpublished guard may clean only the exact semantic identity it
        // armed.  A changed identity (including a successor's replacement)
        // is rejected by the conditional adapter operation and is therefore
        // never deleted by this stale guard.
        let _ = self.io.replace_atomically(Some(&self.requested), None);
        // The node guard is the only owner of the target-port mode.  The
        // sealed shutdown path forgets this guard, intentionally retaining
        // the constant empty program for next-boot identity recovery.
        self.shared_targets.lock().take();
        self.shared_program.lock().take();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SharedElementKey {
    Address { set: SharedElementSet, address: Ipv4Addr },
    Destination(SocketAddrV4),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SharedElementSet {
    ManagedGuestIps,
    OutboundSources,
}

#[derive(Default)]
struct SharedElementState {
    counts: Mutex<BTreeMap<SharedElementKey, usize>>,
    pending_sources: Mutex<BTreeSet<Ipv4Addr>>,
    pending_destinations: Mutex<BTreeMap<Ipv4Addr, BTreeSet<SocketAddrV4>>>,
}

struct SharedElementGuard {
    state: Arc<SharedElementState>,
    program: Arc<Mutex<Option<overdrive_netlink::nft::SharedIpInterceptIdentity>>>,
    keys: Vec<SharedElementKey>,
}

impl InterceptGuard for SharedElementGuard {}

#[allow(clippy::option_if_let_else, clippy::too_many_lines)]
impl Drop for SharedElementGuard {
    fn drop(&mut self) {
        let mut counts = self.state.counts.lock();
        let mut removals = Vec::new();
        for key in &self.keys {
            let Some(count) = counts.get_mut(key) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(key);
                removals.push(key.clone());
            }
        }
        if removals.is_empty() {
            drop(counts);
            return;
        }
        let expected = self.program.lock().clone();
        let Some(expected) = expected else {
            for key in removals {
                *counts.entry(key).or_insert(0) += 1;
            }
            drop(counts);
            return;
        };
        let source_addr = self.keys.iter().find_map(|key| match key {
            SharedElementKey::Address { set: SharedElementSet::OutboundSources, address } => {
                Some(*address)
            }
            SharedElementKey::Address { .. } | SharedElementKey::Destination(_) => None,
        });
        let inbound = removals
            .iter()
            .filter_map(|key| match key {
                SharedElementKey::Destination(destination) => Some(*destination),
                SharedElementKey::Address { .. } => None,
            })
            .collect::<Vec<_>>();
        let source_removed = source_addr.is_some()
            && removals.iter().any(|key| {
                matches!(
                    key,
                    SharedElementKey::Address { set: SharedElementSet::OutboundSources, .. }
                )
            });
        let mut pending_sources = self.state.pending_sources.lock();
        let mut pending_destinations = self.state.pending_destinations.lock();
        let mut candidates = BTreeSet::new();
        if let Some(source_addr) = source_addr.filter(|_| source_removed) {
            pending_sources.insert(source_addr);
            candidates.insert(source_addr);
        }
        for destination in &inbound {
            if pending_sources.contains(destination.ip()) {
                pending_destinations.entry(*destination.ip()).or_default().insert(*destination);
                candidates.insert(*destination.ip());
            }
        }
        let source_to_delete = candidates.into_iter().find(|source| {
            !counts.keys().any(|key| {
                matches!(key, SharedElementKey::Destination(destination)
                        if destination.ip() == source)
            })
        });
        let destination_only = inbound
            .iter()
            .copied()
            .filter(|destination| !pending_sources.contains(destination.ip()))
            .collect::<Vec<_>>();
        let grouped_destinations = source_to_delete.map(|source| {
            pending_destinations.remove(&source).unwrap_or_default().into_iter().collect::<Vec<_>>()
        });
        let result = if let Some(source_addr) = source_to_delete {
            pending_sources.remove(&source_addr);
            overdrive_netlink::nft::delete_shared_ip_intercept_elements_atomically(
                &expected,
                Some(source_addr),
                grouped_destinations.as_deref().unwrap_or_default(),
            )
            .map(|_| ())
        } else if !destination_only.is_empty() {
            overdrive_netlink::nft::delete_shared_ip_intercept_elements_atomically(
                &expected,
                None,
                &destination_only,
            )
            .map(|_| ())
        } else {
            Ok(())
        };
        if let Err(source) = result {
            for key in removals {
                *counts.entry(key).or_insert(0) += 1;
            }
            if let Some(source_addr) = source_to_delete {
                pending_sources.insert(source_addr);
                pending_destinations
                    .entry(source_addr)
                    .or_default()
                    .extend(grouped_destinations.unwrap_or_default());
            }
            tracing::error!(
                name: "health.mtls.shared_element_cleanup_failed",
                error = %source,
                "shared mTLS element cleanup failed; retaining process-local ownership"
            );
        }
        drop(pending_destinations);
        drop(pending_sources);
        drop(counts);
    }
}

/// The per-allocation transparent-mTLS **install** driven port.
///
/// Wraps the three privileged, un-ownable primitives
/// [`MtlsInterceptWorker::start_alloc`](crate::mtls_intercept_worker::MtlsInterceptWorker::start_alloc)
/// performs — the `IP_TRANSPARENT` socket setup (`libc::socket` +
/// `setsockopt`) and the two `nft`/`ip` shell-outs — so the install surface is
/// substitutable at the composition root. Production wires
/// [`HostMtlsIntercept`]; tests wire `overdrive_sim::adapters::mtls_intercept::SimMtlsIntercept`.
///
/// SYNC, not `#[async_trait]`: every underlying primitive is a blocking
/// syscall or a blocking `std::process::Command`, `start_alloc` is itself
/// sync, and the contract awaits no store I/O — the repo criterion recorded on
/// [`MtlsResolve`](overdrive_core::traits::mtls_resolve::MtlsResolve). Sync
/// also keeps the trait dyn-compatible with no `Pin<Box<dyn Future>>`
/// allocation per install.
///
/// `Send + Sync + 'static` to be held as `Arc<dyn MtlsIntercept>` and shared
/// across the worker's per-alloc calls.
///
/// # NO `probe()` — deliberate, and recorded
///
/// This port carries no Earned-Trust `probe()` and the composition root
/// installs no boot gate for it, unlike its sibling
/// [`MtlsResolve`](overdrive_core::traits::mtls_resolve::MtlsResolve) and
/// `MtlsEnforcement`. This is a deliberate scope decision recorded in
/// ADR-0076 § Decision 4, not an oversight: the capability this port depends
/// on (`CAP_NET_ADMIN`) is already proven per-deploy at the netns-provision
/// seam that runs strictly UPSTREAM of every call to this port, so a boot
/// probe would re-prove at boot what the deploy path proves anyway — buying a
/// better diagnosis, not a new safety property, at the cost of a production
/// behaviour change out of GH #250's scope. Do NOT add one back without
/// superseding that decision.
pub trait MtlsIntercept: Send + Sync + 'static {
    /// Bind ONE TCP listener at `addr`, suitable for accepting the intercept
    /// leg the caller is standing up.
    ///
    /// This is the primitive both intercept legs are built from: leg-F
    /// (outbound, workload-facing) and leg-C (inbound, peer-facing).
    ///
    /// # Preconditions
    /// - `addr` is an IPv4 address the caller intends to accept on;
    ///   `127.0.0.1:0` (agent-chosen ephemeral loopback) is the production
    ///   shape for both legs.
    ///
    /// # Postconditions on `Ok(listener)`
    /// The returned listener is bound and listening at `addr`.
    /// `listener.local_addr()` reports the concrete bound address; when `addr`
    /// carried port 0 the reported port is the kernel-assigned ephemeral port
    /// and is NON-ZERO. Ownership transfers to the caller — dropping it closes
    /// the socket.
    ///
    /// # Edge cases
    /// Every failure — a refused socket option, `EADDRINUSE`, fd exhaustion —
    /// surfaces as
    /// [`InterceptError::TransparentListener`](crate::mtls_intercept::InterceptError::TransparentListener)
    /// carrying a
    /// cause-distinct `io::Error` source. No fd leaks on any error path: a
    /// partially-created socket is closed before returning.
    ///
    /// # Observable invariants
    /// Each call returns a DISTINCT listener; two calls with port 0 bind two
    /// distinct ephemeral ports. The call installs no `nft` rule and mutates no
    /// routing state.
    ///
    /// # Substrate note (NOT part of this contract)
    /// The PRODUCTION leg semantics require the socket to carry
    /// `IP_TRANSPARENT` + `IP_FREEBIND` — both legs are TPROXY-divert targets,
    /// and a plain socket cannot receive a non-rewriting `tproxy` divert's
    /// orig-dst-addressed SYN. That is [`HostMtlsIntercept`]'s obligation and
    /// is documented on it, NOT here: a simulation adapter holds no
    /// `CAP_NET_ADMIN`, receives no diverted traffic, and honours the
    /// contract above with a plain listener. Stating the setopts as a TRAIT
    /// postcondition would make the contract unimplementable by half its
    /// sanctioned implementors.
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener>;

    /// Converge the one node-scoped shared rule/set program.
    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> Result<Box<dyn InterceptGuard>>;

    /// Observe the complete normalized shared rule/set identity without repair.
    fn observe_shared(&self) -> Result<Option<InterceptPostcondition>>;

    /// Install the per-allocation OUTBOUND source admission in the shared
    /// intercept program. The node-owned constant rule diverts admitted TCP
    /// to the shared leg-F listener without rewriting the destination.
    ///
    /// # Preconditions
    /// - `source_addr` is the canonical guest source IPv4 address.
    /// - `agent_leg_f_port` is the NON-ZERO bound port of a live leg-F
    ///   listener obtained from [`bind_transparent`](Self::bind_transparent).
    ///
    /// # Postconditions on `Ok(guard)`
    /// The outbound capture for `source_addr` is in effect **against this
    /// adapter's OWN substrate**, and every prerequisite it depends on has
    /// been converged idempotently. The returned guard OWNS exactly what this
    /// call acquired: dropping it releases that and nothing else (see
    /// [`InterceptGuard`]). What "in effect" MEANS is adapter-specific and is
    /// not observable through this trait — see the substrate note below.
    ///
    /// # Edge cases
    /// - Any install failure surfaces as one of the decomposed nft/ip install
    ///   errors —
    ///   [`InterceptError::NftRuleInstallFailed`](crate::mtls_intercept::InterceptError::NftRuleInstallFailed)
    ///   (op-keyed, errno-carrying),
    ///   [`InterceptError::NftHandleRecoveryFailed`](crate::mtls_intercept::InterceptError::NftHandleRecoveryFailed),
    ///   or the shared-routing-infra
    ///   [`InterceptError::IpRuleAddFailed`](crate::mtls_intercept::InterceptError::IpRuleAddFailed)
    ///   /
    ///   [`InterceptError::IpRouteLocalAddFailed`](crate::mtls_intercept::InterceptError::IpRouteLocalAddFailed).
    /// - A re-install for an already-owned source adopts the process-local
    ///   element token; it does not create a duplicate set element.
    ///
    /// # Observable invariants
    /// One call acquires at most ONE capture. On `Err` NOTHING acquired by
    /// this call outlives it — every partially-applied step is reverted or was
    /// never applied, so a failed install leaks nothing.
    ///
    /// # Substrate note (NOT part of this contract)
    /// [`HostMtlsIntercept`] realises the capture as one managed-guest element
    /// plus one outbound-source element in the node-shared sets. Its guard
    /// removes only those elements after exact read-back. A simulation adapter
    /// realises it as nothing at all. Both honour the contract above.
    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>>;

    /// Install ONE per-Service-port INBOUND intercept: capture connections
    /// destined for `virt` (the canonical per-workload address paired with one
    /// DECLARED Service listener port) and divert them to the agent's leg-C
    /// listener at `agent_leg_c_port` on loopback (D-A1, GH #241).
    ///
    /// # Preconditions
    /// - `virt` pairs the canonical per-workload address with a DECLARED
    ///   Service listener port — never the ephemeral leg-C port (D-BLOCKER1 /
    ///   D-TME-10 one-source/two-readers).
    /// - `agent_leg_c_port` is the NON-ZERO bound port of a live leg-C
    ///   listener obtained from [`bind_transparent`](Self::bind_transparent).
    ///
    /// # Postconditions on `Ok(guard)`
    /// The inbound capture for `virt` is in effect **against this adapter's
    /// OWN substrate**, and every prerequisite it depends on has been
    /// converged idempotently. The returned guard OWNS exactly what this call
    /// acquired: dropping it releases that and nothing else (see
    /// [`InterceptGuard`]). What "in effect" MEANS is adapter-specific and is
    /// not observable through this trait — see the substrate note below.
    ///
    /// # Edge cases
    /// Identical failure surface to
    /// [`install_outbound`](Self::install_outbound) — the decomposed nft/ip
    /// install errors
    /// ([`InterceptError::NftRuleInstallFailed`](crate::mtls_intercept::InterceptError::NftRuleInstallFailed)
    /// and siblings), each naming the failing operation.
    /// The caller installs N captures for N declared ports and ZERO for a
    /// Job-kind / host-netns workload; that N-vs-0 decision is the CALLER's,
    /// not this method's.
    ///
    /// # Observable invariants
    /// One call acquires at most ONE capture; on `Err`, nothing acquired by
    /// this call outlives it.
    ///
    /// # Substrate note (NOT part of this contract)
    /// [`HostMtlsIntercept`] realises the capture as one
    /// `ipv4_addr . inet_service` element keyed by `virt`, removed by guard
    /// `Drop` after exact read-back. A simulation adapter realises it as
    /// nothing at all.
    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>>;
}

/// Production [`MtlsIntercept`] binding.
///
/// Listener binding and node-program operations delegate to the existing
/// adapter code; allocation operations own the node-shared set-element
/// tokens. No per-allocation nft rule is created by the shared-owner path.
///
/// # Substrate obligations (BEYOND the [`MtlsIntercept`] contract)
///
/// These are this adapter's obligations, deliberately NOT stated on the trait
/// (a trait postcondition no sanctioned implementor can honour is a broken
/// contract, per `.claude/rules/development.md` § "Trait definitions specify
/// behavior, not just signature"). They are what the Tier-3 suite asserts:
///
/// - [`bind_transparent`](MtlsIntercept::bind_transparent) returns a socket
///   carrying BOTH `IP_TRANSPARENT` (so a non-rewriting `tproxy` divert's
///   orig-dst-addressed SYN is accepted, and `getsockname` recovers the
///   orig-dst) and `IP_FREEBIND` (so leg-C can bind a non-local address on the
///   OUTPUT path).
/// - [`install_outbound`](MtlsIntercept::install_outbound) adds exactly one
///   managed-guest element and one outbound-source element.
/// - [`install_inbound`](MtlsIntercept::install_inbound) adds exactly one
///   destination/port element.
#[allow(clippy::struct_field_names)]
pub struct HostMtlsIntercept {
    shared_program_io: Arc<dyn SharedInterceptProgramIo>,
    targets: Arc<Mutex<Option<(u16, u16)>>>,
    elements: Arc<SharedElementState>,
    program: Arc<Mutex<Option<overdrive_netlink::nft::SharedIpInterceptIdentity>>>,
}

impl Clone for HostMtlsIntercept {
    fn clone(&self) -> Self {
        Self {
            shared_program_io: Arc::clone(&self.shared_program_io),
            targets: Arc::clone(&self.targets),
            elements: Arc::clone(&self.elements),
            program: Arc::clone(&self.program),
        }
    }
}

impl std::fmt::Debug for HostMtlsIntercept {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HostMtlsIntercept").finish_non_exhaustive()
    }
}

impl HostMtlsIntercept {
    /// Construct the production binding with its private real nft I/O adapter.
    #[must_use]
    #[allow(
        clippy::missing_const_for_fn,
        reason = "the accepted host binding owns an Arc to its private real shared-program I/O adapter"
    )]
    pub fn new() -> Self {
        Self {
            shared_program_io: Arc::new(RealSharedInterceptProgramIo),
            targets: Arc::new(Mutex::new(None)),
            elements: Arc::new(SharedElementState::default()),
            program: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    fn with_shared_program_io(io: Arc<dyn SharedInterceptProgramIo>) -> Self {
        Self {
            shared_program_io: io,
            targets: Arc::new(Mutex::new(None)),
            elements: Arc::new(SharedElementState::default()),
            program: Arc::new(Mutex::new(None)),
        }
    }

    fn shared_guard(&self, requested: InterceptPostcondition) -> Box<dyn InterceptGuard> {
        Box::new(SharedInterceptGuard {
            io: Arc::clone(&self.shared_program_io),
            requested,
            shared_targets: Arc::clone(&self.targets),
            shared_program: Arc::clone(&self.program),
        })
    }

    fn shared_port(&self, leg: InterceptLeg, actual: u16) -> Result<u16> {
        let targets = *self.targets.lock();
        let Some((leg_f, leg_c)) = targets else {
            return Err(InterceptError::NftRuleInstallFailed {
                op: "shared-element-owner",
                source: NetlinkError::nft(
                    "shared-element-owner",
                    std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        "shared constant program is not published",
                    ),
                ),
            });
        };
        let expected = match leg {
            InterceptLeg::F => leg_f,
            InterceptLeg::C => leg_c,
        };
        if actual != expected {
            return Err(InterceptError::SharedListenerPortMismatch { leg, expected, actual });
        }
        Ok(expected)
    }

    #[allow(clippy::too_many_lines, clippy::option_if_let_else)]
    fn acquire_elements(&self, keys: Vec<SharedElementKey>) -> Result<Box<dyn InterceptGuard>> {
        let mut counts = self.elements.counts.lock();
        let expected =
            self.program.lock().clone().ok_or_else(|| InterceptError::NftRuleInstallFailed {
                op: "shared-element-owner",
                source: NetlinkError::nft(
                    "shared-element-owner",
                    std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        "shared constant program is not published",
                    ),
                ),
            })?;
        let source_addr = keys.iter().find_map(|key| match key {
            SharedElementKey::Address { set: SharedElementSet::OutboundSources, address } => {
                Some(*address)
            }
            _ => None,
        });
        let inbound = keys
            .iter()
            .filter_map(|key| match key {
                SharedElementKey::Destination(destination) => Some(*destination),
                SharedElementKey::Address { .. } => None,
            })
            .collect::<Vec<_>>();
        let present = keys.iter().filter(|key| counts.contains_key(*key)).count();
        let expected_group_size = if source_addr.is_some() { 2 } else { 1 };
        if present != 0 && present != expected_group_size {
            let error = InterceptError::NftElementUpdateFailed {
                set: crate::mtls_intercept::InterceptSet::ManagedGuestIps,
                operation: crate::mtls_intercept::InterceptElementOperation::ReadBack,
                key: crate::mtls_intercept::InterceptElementKey::Address(
                    source_addr.unwrap_or(Ipv4Addr::UNSPECIFIED),
                ),
                source: NetlinkError::nft(
                    "shared-element-readback",
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "shared allocation element group is partially owned",
                    ),
                ),
            };
            drop(counts);
            return Err(error);
        }
        if present == 0 {
            let result = if let Some(source_addr) = source_addr {
                overdrive_netlink::nft::insert_shared_ip_intercept_outbound_elements_atomically(
                    &expected,
                    source_addr,
                )
            } else if let Some(destination) = inbound.first().copied() {
                overdrive_netlink::nft::insert_shared_ip_intercept_inbound_element_atomically(
                    &expected,
                    destination,
                )
            } else {
                Err(NetlinkError::nft(
                    "shared-element-owner",
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty element group"),
                ))
            };
            if let Err(source) = result {
                let error = InterceptError::NftElementUpdateFailed {
                    set: match keys.first() {
                        Some(SharedElementKey::Address {
                            set: SharedElementSet::ManagedGuestIps,
                            ..
                        })
                        | None => crate::mtls_intercept::InterceptSet::ManagedGuestIps,
                        Some(SharedElementKey::Address {
                            set: SharedElementSet::OutboundSources,
                            ..
                        }) => crate::mtls_intercept::InterceptSet::OutboundSources,
                        Some(SharedElementKey::Destination(_)) => {
                            crate::mtls_intercept::InterceptSet::InboundDestinations
                        }
                    },
                    operation: crate::mtls_intercept::InterceptElementOperation::Insert,
                    key: match keys.first() {
                        Some(SharedElementKey::Address { address, .. }) => {
                            crate::mtls_intercept::InterceptElementKey::Address(*address)
                        }
                        Some(SharedElementKey::Destination(destination)) => {
                            crate::mtls_intercept::InterceptElementKey::Destination(*destination)
                        }
                        None => crate::mtls_intercept::InterceptElementKey::Address(
                            Ipv4Addr::UNSPECIFIED,
                        ),
                    },
                    source,
                };
                drop(counts);
                return Err(error);
            }
        }
        for key in &keys {
            *counts.entry(key.clone()).or_insert(0) += 1;
        }
        drop(counts);
        Ok(Box::new(SharedElementGuard {
            state: Arc::clone(&self.elements),
            program: Arc::clone(&self.program),
            keys,
        }))
    }

    fn shared_outbound(
        &self,
        source: Ipv4Addr,
        leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        self.shared_port(InterceptLeg::F, leg_f_port)?;
        self.acquire_elements(vec![
            SharedElementKey::Address { set: SharedElementSet::ManagedGuestIps, address: source },
            SharedElementKey::Address { set: SharedElementSet::OutboundSources, address: source },
        ])
    }

    fn shared_inbound(
        &self,
        destination: SocketAddrV4,
        leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        self.shared_port(InterceptLeg::C, leg_c_port)?;
        self.acquire_elements(vec![SharedElementKey::Destination(destination)])
    }

    #[allow(dead_code, reason = "private algorithm seam is driven by in-module acceptance bodies")]
    fn replace_shared_program_for_boot(
        &self,
        requested: InterceptPostcondition,
    ) -> Result<Box<dyn InterceptGuard>> {
        let prior = self.shared_program_io.observe().map_err(|source| {
            InterceptError::NftSharedReplaceFailed {
                prior: None,
                requested: requested.clone(),
                source,
            }
        })?;
        self.replace_observed_shared_program(prior, requested)
    }

    fn replace_observed_shared_program(
        &self,
        prior: Option<InterceptPostcondition>,
        requested: InterceptPostcondition,
    ) -> Result<Box<dyn InterceptGuard>> {
        if prior.as_ref() == Some(&requested) {
            return Ok(self.shared_guard(requested));
        }

        self.shared_program_io.replace_atomically(prior.as_ref(), Some(&requested)).map_err(
            |source| InterceptError::NftSharedReplaceFailed {
                prior: prior.clone(),
                requested: requested.clone(),
                source,
            },
        )?;

        let replacement_observed = match self.shared_program_io.observe() {
            Ok(observed) => observed,
            Err(replacement_read_source) => {
                if let Err(source) =
                    self.shared_program_io.replace_atomically(Some(&requested), prior.as_ref())
                {
                    return Err(InterceptError::NftSharedRollbackFailed {
                        operation: InterceptSharedRollbackOperation::RestorePrior,
                        prior,
                        requested,
                        replacement_read_source: Some(replacement_read_source),
                        replacement_observed: None,
                        source,
                    });
                }
                let rollback_observed = match self.shared_program_io.observe() {
                    Ok(observed) => observed,
                    Err(source) => {
                        return Err(InterceptError::NftSharedRollbackFailed {
                            operation: InterceptSharedRollbackOperation::ReadBackPrior,
                            prior,
                            requested,
                            replacement_read_source: Some(replacement_read_source),
                            replacement_observed: None,
                            source,
                        });
                    }
                };
                if rollback_observed == prior {
                    return Err(InterceptError::NftSharedReplacementReadFailedRolledBack {
                        prior,
                        requested,
                        replacement_read_source,
                    });
                }
                return Err(InterceptError::NftSharedRollbackPostconditionMismatch {
                    prior,
                    requested,
                    replacement_read_source: Some(replacement_read_source),
                    replacement_observed: None,
                    rollback_observed,
                });
            }
        };
        if replacement_observed.as_ref() == Some(&requested) {
            return Ok(self.shared_guard(requested));
        }

        self.shared_program_io
            .replace_atomically(replacement_observed.as_ref(), prior.as_ref())
            .map_err(|source| InterceptError::NftSharedRollbackFailed {
                operation: InterceptSharedRollbackOperation::RestorePrior,
                prior: prior.clone(),
                requested: requested.clone(),
                replacement_read_source: None,
                replacement_observed: replacement_observed.clone(),
                source,
            })?;

        let rollback_observed = self.shared_program_io.observe().map_err(|source| {
            InterceptError::NftSharedRollbackFailed {
                operation: InterceptSharedRollbackOperation::ReadBackPrior,
                prior: prior.clone(),
                requested: requested.clone(),
                replacement_read_source: None,
                replacement_observed: replacement_observed.clone(),
                source,
            }
        })?;
        if rollback_observed == prior {
            return Err(InterceptError::NftSharedReplacementMismatchRolledBack {
                prior,
                requested,
                replacement_observed,
            });
        }
        Err(InterceptError::NftSharedRollbackPostconditionMismatch {
            prior,
            requested,
            replacement_read_source: None,
            replacement_observed,
            rollback_observed,
        })
    }

    #[allow(dead_code, reason = "private runtime audit seam is driven by worker recovery wiring")]
    fn require_shared_program_at_runtime(&self, expected: InterceptPostcondition) -> Result<()> {
        let observed = self.shared_program_io.observe().map_err(|source| {
            InterceptError::NftRuleInstallFailed { op: "observe-shared-runtime", source }
        })?;
        if observed == Some(expected.clone()) {
            Ok(())
        } else {
            Err(InterceptError::PostconditionMismatch { expected, observed })
        }
    }
}

fn shared_program_for_targets(
    leg_f: SocketAddrV4,
    leg_c: SocketAddrV4,
) -> Result<InterceptPostcondition> {
    let identity = overdrive_netlink::nft::SharedIpInterceptIdentity::for_listener_ports(
        leg_f.port(),
        leg_c.port(),
    )
    .map_err(|source| InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", source })?;
    Ok(postcondition_from_shared_identity(&identity))
}

fn observe_shared_ip_program() -> std::result::Result<Option<InterceptPostcondition>, NetlinkError>
{
    overdrive_netlink::nft::observe_shared_ip_intercept()
        .map(|identity| identity.map(|identity| postcondition_from_shared_identity(&identity)))
}

fn replace_shared_ip_program(
    expected_current: Option<&InterceptPostcondition>,
    desired: Option<&InterceptPostcondition>,
) -> std::result::Result<(), NetlinkError> {
    let expected = expected_current.map(shared_identity_from_postcondition).transpose()?;
    let desired = desired.map(shared_identity_from_postcondition).transpose()?;
    overdrive_netlink::nft::replace_shared_ip_intercept_atomically(
        expected.as_ref(),
        desired.as_ref(),
    )
}

fn postcondition_from_shared_identity(
    identity: &overdrive_netlink::nft::SharedIpInterceptIdentity,
) -> InterceptPostcondition {
    let (table_and_chains, sets, prerouting, output) = identity.normalized_parts();
    InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output }
}

fn shared_identity_from_postcondition(
    postcondition: &InterceptPostcondition,
) -> std::result::Result<overdrive_netlink::nft::SharedIpInterceptIdentity, NetlinkError> {
    let InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output } =
        postcondition
    else {
        return Err(NetlinkError::nft(
            "shared-ip-program",
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "shared IP postcondition is not a constant-rule identity",
            ),
        ));
    };
    overdrive_netlink::nft::SharedIpInterceptIdentity::from_normalized_parts(
        table_and_chains.clone(),
        sets.clone(),
        prerouting.clone(),
        output.clone(),
    )
}

#[derive(Debug)]
struct RealSharedInterceptProgramIo;

impl SharedInterceptProgramIo for RealSharedInterceptProgramIo {
    fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError> {
        crate::mtls_intercept_port::observe_shared_ip_program()
    }

    fn replace_atomically(
        &self,
        expected_current: Option<&InterceptPostcondition>,
        desired: Option<&InterceptPostcondition>,
    ) -> std::result::Result<(), NetlinkError> {
        crate::mtls_intercept_port::replace_shared_ip_program(expected_current, desired)
    }
}

impl Default for HostMtlsIntercept {
    fn default() -> Self {
        Self::new()
    }
}

impl MtlsIntercept for HostMtlsIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener> {
        make_transparent_listener(addr)
    }

    fn converge_shared(
        &self,
        prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> Result<Box<dyn InterceptGuard>> {
        let requested = shared_program_for_targets(leg_f, leg_c)?;
        let requested_identity =
            shared_identity_from_postcondition(&requested).map_err(|source| {
                InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", source }
            })?;
        let observed = self.shared_program_io.observe().map_err(|source| {
            InterceptError::NftRuleInstallFailed { op: "observe-shared", source }
        })?;
        if observed.as_ref() != prior {
            return Err(InterceptError::PostconditionMismatch {
                expected: prior.cloned().unwrap_or_else(|| requested.clone()),
                observed,
            });
        }
        let guard = self.replace_observed_shared_program(observed, requested)?;
        *self.targets.lock() = Some((leg_f.port(), leg_c.port()));
        *self.program.lock() = Some(requested_identity);
        Ok(guard)
    }

    fn observe_shared(&self) -> Result<Option<InterceptPostcondition>> {
        self.shared_program_io
            .observe()
            .map_err(|source| InterceptError::NftRuleInstallFailed { op: "observe-shared", source })
    }

    fn install_outbound(
        &self,
        source_addr: Ipv4Addr,
        agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if self.targets.lock().is_some() {
            return self.shared_outbound(source_addr, agent_leg_f_port);
        }
        // The old no-network fixture lane never reaches this branch. The
        // accepted post-cut port has no textual/per-interface fallback.
        Err(InterceptError::NftRuleInstallFailed {
            op: "shared-owner-required",
            source: NetlinkError::nft(
                "shared-owner-required",
                std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "allocation source admission requires the shared owner",
                ),
            ),
        })
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if self.targets.lock().is_some() {
            return self.shared_inbound(virt, agent_leg_c_port);
        }
        install_inbound_tproxy(virt, agent_leg_c_port)
            .map(|guard| Box::new(guard) as Box<dyn InterceptGuard>)
    }
}

#[cfg(test)]
#[allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::significant_drop_tightening,
    reason = "stateful acceptance fixture holds its private universe lock for each atomic port call"
)]
mod shared_program_rollback_acceptance {
    use std::collections::VecDeque;
    use std::error::Error as _;

    use parking_lot::Mutex;

    use super::*;
    use crate::mtls_intercept::{InterceptError, InterceptSharedRollbackOperation};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Observe,
        ReplacePresent,
        RestorePresent,
        RestoreAbsent,
    }

    struct ScriptedIo {
        observations:
            Mutex<VecDeque<std::result::Result<Option<InterceptPostcondition>, NetlinkError>>>,
        replacements: Mutex<VecDeque<std::result::Result<(), NetlinkError>>>,
        calls: Mutex<Vec<Call>>,
    }

    impl ScriptedIo {
        fn new(
            observations: Vec<std::result::Result<Option<InterceptPostcondition>, NetlinkError>>,
            replacements: Vec<std::result::Result<(), NetlinkError>>,
        ) -> Self {
            Self {
                observations: Mutex::new(observations.into()),
                replacements: Mutex::new(replacements.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().clone()
        }
    }

    impl SharedInterceptProgramIo for ScriptedIo {
        fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError> {
            self.calls.lock().push(Call::Observe);
            self.observations.lock().pop_front().expect("scripted observe result")
        }

        fn replace_atomically(
            &self,
            expected_current: Option<&InterceptPostcondition>,
            desired: Option<&InterceptPostcondition>,
        ) -> std::result::Result<(), NetlinkError> {
            let call = match (expected_current.is_some(), desired.is_some()) {
                (_, true) => Call::ReplacePresent,
                (true, false) => Call::RestorePresent,
                (false, false) => Call::RestoreAbsent,
            };
            self.calls.lock().push(call);
            self.replacements.lock().pop_front().expect("scripted replace result")
        }
    }

    fn program(port: u16) -> InterceptPostcondition {
        InterceptPostcondition::ConstantRules {
            table_and_chains: vec![
                b"table:overdrive-mtls".to_vec(),
                b"chain:prerouting".to_vec(),
                b"chain:output".to_vec(),
            ],
            sets: vec![
                b"set:managed_guest_ips:ipv4_addr".to_vec(),
                b"set:outbound_sources:ipv4_addr".to_vec(),
                b"set:inbound_destinations:ipv4_addr.inet_service".to_vec(),
            ],
            prerouting: vec![
                b"rule:prerouting:0:leg-s-exemption".to_vec(),
                format!("rule:prerouting:1:leg-f:{port}").into_bytes(),
                b"rule:prerouting:2:unregistered-source-drop".to_vec(),
                format!("rule:prerouting:3:leg-c:{}", port + 1).into_bytes(),
                b"rule:prerouting:4:managed-guest-drop".to_vec(),
            ],
            output: vec![
                b"rule:output:0:leg-s-exemption".to_vec(),
                b"rule:output:1:registered-destination-mark".to_vec(),
                b"rule:output:2:managed-guest-drop".to_vec(),
            ],
        }
    }

    fn canonical_program(forward_port: u16, capture_port: u16) -> InterceptPostcondition {
        let (table_and_chains, sets, prerouting, output) =
            overdrive_netlink::nft::SharedIpInterceptIdentity::for_listener_ports(
                forward_port,
                capture_port,
            )
            .expect("non-zero D15 targets form one canonical identity")
            .normalized_parts();
        InterceptPostcondition::ConstantRules { table_and_chains, sets, prerouting, output }
    }

    fn netlink_error(op: &'static str, errno: i32) -> NetlinkError {
        NetlinkError::nft(op, std::io::Error::from_raw_os_error(errno))
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum StatefulStep {
        ObserveCurrent,
        ObserveError { operation: &'static str, errno: i32 },
        ObserveState(Option<InterceptPostcondition>),
        ReplaceCommit,
        ReplaceError { operation: &'static str, errno: i32 },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MutationDisposition {
        Committed,
        Rejected { operation: &'static str, errno: i32 },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ConditionalMutation {
        expected_current: Option<InterceptPostcondition>,
        desired: Option<InterceptPostcondition>,
        disposition: MutationDisposition,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ObservationDisposition {
        Returned(Option<InterceptPostcondition>),
        Failed { operation: &'static str, errno: i32 },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SharedProgramUniverse {
        owned_program: Option<InterceptPostcondition>,
        dynamic_elements: [Vec<Vec<u8>>; 3],
        foreign_objects: Vec<Vec<u8>>,
        observations: Vec<ObservationDisposition>,
        conditional_mutations: Vec<ConditionalMutation>,
        remaining_fault_schedule: VecDeque<StatefulStep>,
    }

    struct StatefulIo {
        universe: Mutex<SharedProgramUniverse>,
    }

    impl StatefulIo {
        fn new(
            owned_program: Option<InterceptPostcondition>,
            steps: impl IntoIterator<Item = StatefulStep>,
        ) -> Self {
            Self {
                universe: Mutex::new(SharedProgramUniverse {
                    owned_program,
                    dynamic_elements: [
                        vec![b"managed:100.95.0.2".to_vec()],
                        vec![b"source:100.95.0.2".to_vec()],
                        vec![b"destination:100.95.0.2:8443".to_vec()],
                    ],
                    foreign_objects: vec![
                        b"table:foreign-sentinel".to_vec(),
                        b"rule:foreign-sentinel:accept".to_vec(),
                    ],
                    observations: Vec::new(),
                    conditional_mutations: Vec::new(),
                    remaining_fault_schedule: steps.into_iter().collect(),
                }),
            }
        }

        fn snapshot(&self) -> SharedProgramUniverse {
            self.universe.lock().clone()
        }

        fn next_step(
            universe: &mut SharedProgramUniverse,
            operation: &'static str,
        ) -> std::result::Result<StatefulStep, NetlinkError> {
            universe
                .remaining_fault_schedule
                .pop_front()
                .ok_or_else(|| netlink_error(operation, libc::EPROTO))
        }
    }

    impl SharedInterceptProgramIo for StatefulIo {
        fn observe(&self) -> std::result::Result<Option<InterceptPostcondition>, NetlinkError> {
            let mut universe = self.universe.lock();
            match Self::next_step(&mut universe, "unexpected-observe")? {
                StatefulStep::ObserveCurrent => {
                    let observed = universe.owned_program.clone();
                    universe.observations.push(ObservationDisposition::Returned(observed.clone()));
                    Ok(observed)
                }
                StatefulStep::ObserveError { operation, errno } => {
                    universe.observations.push(ObservationDisposition::Failed { operation, errno });
                    Err(netlink_error(operation, errno))
                }
                StatefulStep::ObserveState(observed) => {
                    universe.owned_program.clone_from(&observed);
                    universe.observations.push(ObservationDisposition::Returned(observed.clone()));
                    Ok(observed)
                }
                StatefulStep::ReplaceCommit | StatefulStep::ReplaceError { .. } => {
                    Err(netlink_error("observe-out-of-order", libc::EPROTO))
                }
            }
        }

        fn replace_atomically(
            &self,
            expected_current: Option<&InterceptPostcondition>,
            desired: Option<&InterceptPostcondition>,
        ) -> std::result::Result<(), NetlinkError> {
            let mut universe = self.universe.lock();
            let expected_current = expected_current.cloned();
            let desired = desired.cloned();
            if universe.owned_program != expected_current {
                universe.conditional_mutations.push(ConditionalMutation {
                    expected_current,
                    desired,
                    disposition: MutationDisposition::Rejected {
                        operation: "conditional-identity-mismatch",
                        errno: libc::EAGAIN,
                    },
                });
                return Err(netlink_error("conditional-identity-mismatch", libc::EAGAIN));
            }
            match Self::next_step(&mut universe, "unexpected-replace")? {
                StatefulStep::ReplaceCommit => {
                    universe.conditional_mutations.push(ConditionalMutation {
                        expected_current,
                        desired: desired.clone(),
                        disposition: MutationDisposition::Committed,
                    });
                    universe.owned_program = desired;
                    Ok(())
                }
                StatefulStep::ReplaceError { operation, errno } => {
                    universe.conditional_mutations.push(ConditionalMutation {
                        expected_current,
                        desired,
                        disposition: MutationDisposition::Rejected { operation, errno },
                    });
                    Err(netlink_error(operation, errno))
                }
                StatefulStep::ObserveCurrent
                | StatefulStep::ObserveError { .. }
                | StatefulStep::ObserveState(_) => {
                    Err(netlink_error("replace-out-of-order", libc::EPROTO))
                }
            }
        }
    }

    fn assert_nft_source(error: &NetlinkError, operation: &'static str, errno: i32) {
        assert!(
            matches!(
                error,
                NetlinkError::Nft { op, source }
                    if *op == operation && source.raw_os_error() == Some(errno)
            ),
            "expected nft {operation}/{errno}, observed {error:?}"
        );
    }

    fn assert_error_chain_source(error: &InterceptError, expected: Option<(&'static str, i32)>) {
        match (error.source(), expected) {
            (Some(source), Some((operation, errno))) => {
                let source = source
                    .downcast_ref::<NetlinkError>()
                    .expect("D15 InterceptError source remains the typed NetlinkError");
                assert_nft_source(source, operation, errno);
            }
            (None, None) => {}
            (actual, expected) => {
                panic!("wrong D15 error-chain source: expected {expected:?}, got {actual:?}")
            }
        }
    }

    fn assert_program_target_only_delta(
        prior: &InterceptPostcondition,
        requested: &InterceptPostcondition,
    ) {
        let (
            InterceptPostcondition::ConstantRules {
                table_and_chains: prior_tables,
                sets: prior_sets,
                prerouting: prior_prerouting,
                output: prior_output,
            },
            InterceptPostcondition::ConstantRules {
                table_and_chains: requested_tables,
                sets: requested_sets,
                prerouting: requested_prerouting,
                output: requested_output,
            },
        ) = (prior, requested)
        else {
            panic!("stateful shared-IP fixture uses only constant-rule identities")
        };
        assert_eq!(prior_tables, requested_tables);
        assert_eq!(prior_sets, requested_sets);
        assert_eq!(prior_output, requested_output);
        assert_eq!(prior_prerouting.len(), 5);
        assert_eq!(requested_prerouting.len(), 5);
        for index in [0, 2, 4] {
            assert_eq!(prior_prerouting[index], requested_prerouting[index]);
        }
        for index in [1, 3] {
            assert_ne!(prior_prerouting[index], requested_prerouting[index]);
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the finite table is the closed two-prior by two-trigger by four-rollback-outcome contract"
    )]
    fn shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior() {
        #[derive(Clone, Copy, Debug)]
        enum Trigger {
            SemanticMismatch,
            DesiredReadFailure,
        }

        #[derive(Clone, Copy, Debug)]
        enum RollbackOutcome {
            WriteFailure,
            ReadFailure,
            ExactRestoration,
            SemanticMismatch,
        }

        let requested = program(20_000);
        let wrong_replacement = program(21_000);
        let wrong_rollback = program(18_000);

        for prior in [None, Some(program(19_000))] {
            for trigger in [Trigger::SemanticMismatch, Trigger::DesiredReadFailure] {
                for rollback in [
                    RollbackOutcome::WriteFailure,
                    RollbackOutcome::ReadFailure,
                    RollbackOutcome::ExactRestoration,
                    RollbackOutcome::SemanticMismatch,
                ] {
                    let semantic_replacement_observed =
                        if prior.is_some() { None } else { Some(wrong_replacement.clone()) };
                    let semantic_rollback_observed =
                        if prior.is_some() { None } else { Some(wrong_rollback.clone()) };
                    let mut steps = vec![StatefulStep::ObserveCurrent, StatefulStep::ReplaceCommit];
                    let post_commit = match trigger {
                        Trigger::SemanticMismatch => {
                            steps.push(StatefulStep::ObserveState(
                                semantic_replacement_observed.clone(),
                            ));
                            semantic_replacement_observed.clone()
                        }
                        Trigger::DesiredReadFailure => {
                            steps.push(StatefulStep::ObserveError {
                                operation: "desired-read",
                                errno: libc::EIO,
                            });
                            Some(requested.clone())
                        }
                    };
                    match rollback {
                        RollbackOutcome::WriteFailure => {
                            steps.push(StatefulStep::ReplaceError {
                                operation: "rollback-write",
                                errno: libc::EBUSY,
                            });
                        }
                        RollbackOutcome::ReadFailure => {
                            steps.extend([
                                StatefulStep::ReplaceCommit,
                                StatefulStep::ObserveError {
                                    operation: "rollback-read",
                                    errno: libc::ENODATA,
                                },
                            ]);
                        }
                        RollbackOutcome::ExactRestoration => {
                            steps.extend([
                                StatefulStep::ReplaceCommit,
                                StatefulStep::ObserveCurrent,
                            ]);
                        }
                        RollbackOutcome::SemanticMismatch => {
                            steps.extend([
                                StatefulStep::ReplaceCommit,
                                StatefulStep::ObserveState(semantic_rollback_observed.clone()),
                            ]);
                        }
                    }

                    let io = Arc::new(StatefulIo::new(prior.clone(), steps));
                    let before = io.snapshot();
                    let host = HostMtlsIntercept::with_shared_program_io(io.clone());
                    let error = host
                        .replace_shared_program_for_boot(requested.clone())
                        .err()
                        .expect("every post-commit read-back fault refuses startup");

                    match (trigger, rollback, &error) {
                        (
                            Trigger::SemanticMismatch,
                            RollbackOutcome::WriteFailure | RollbackOutcome::ReadFailure,
                            InterceptError::NftSharedRollbackFailed {
                                operation,
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_read_source,
                                replacement_observed,
                                source,
                            },
                        ) => {
                            let expected_operation = match rollback {
                                RollbackOutcome::WriteFailure => {
                                    InterceptSharedRollbackOperation::RestorePrior
                                }
                                RollbackOutcome::ReadFailure => {
                                    InterceptSharedRollbackOperation::ReadBackPrior
                                }
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch => unreachable!(),
                            };
                            assert_eq!(*operation, expected_operation);
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert!(replacement_read_source.is_none());
                            assert_eq!(replacement_observed, &semantic_replacement_observed);
                            let (source_op, source_errno) = match rollback {
                                RollbackOutcome::WriteFailure => ("rollback-write", libc::EBUSY),
                                RollbackOutcome::ReadFailure => ("rollback-read", libc::ENODATA),
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch => unreachable!(),
                            };
                            assert_nft_source(source, source_op, source_errno);
                        }
                        (
                            Trigger::DesiredReadFailure,
                            RollbackOutcome::WriteFailure | RollbackOutcome::ReadFailure,
                            InterceptError::NftSharedRollbackFailed {
                                operation,
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_read_source: Some(read_source),
                                replacement_observed,
                                source,
                            },
                        ) => {
                            let expected_operation = match rollback {
                                RollbackOutcome::WriteFailure => {
                                    InterceptSharedRollbackOperation::RestorePrior
                                }
                                RollbackOutcome::ReadFailure => {
                                    InterceptSharedRollbackOperation::ReadBackPrior
                                }
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch => unreachable!(),
                            };
                            assert_eq!(*operation, expected_operation);
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert!(replacement_observed.is_none());
                            assert_nft_source(read_source, "desired-read", libc::EIO);
                            let (source_op, source_errno) = match rollback {
                                RollbackOutcome::WriteFailure => ("rollback-write", libc::EBUSY),
                                RollbackOutcome::ReadFailure => ("rollback-read", libc::ENODATA),
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch => unreachable!(),
                            };
                            assert_nft_source(source, source_op, source_errno);
                        }
                        (
                            Trigger::SemanticMismatch,
                            RollbackOutcome::ExactRestoration,
                            InterceptError::NftSharedReplacementMismatchRolledBack {
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_observed,
                            },
                        ) => {
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert_eq!(replacement_observed, &semantic_replacement_observed);
                        }
                        (
                            Trigger::DesiredReadFailure,
                            RollbackOutcome::ExactRestoration,
                            InterceptError::NftSharedReplacementReadFailedRolledBack {
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_read_source,
                            },
                        ) => {
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert_nft_source(replacement_read_source, "desired-read", libc::EIO);
                        }
                        (
                            Trigger::SemanticMismatch,
                            RollbackOutcome::SemanticMismatch,
                            InterceptError::NftSharedRollbackPostconditionMismatch {
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_read_source,
                                replacement_observed,
                                rollback_observed,
                            },
                        ) => {
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert!(replacement_read_source.is_none());
                            assert_eq!(replacement_observed, &semantic_replacement_observed);
                            assert_eq!(rollback_observed, &semantic_rollback_observed);
                        }
                        (
                            Trigger::DesiredReadFailure,
                            RollbackOutcome::SemanticMismatch,
                            InterceptError::NftSharedRollbackPostconditionMismatch {
                                prior: actual_prior,
                                requested: actual_requested,
                                replacement_read_source: Some(read_source),
                                replacement_observed,
                                rollback_observed,
                            },
                        ) => {
                            assert_eq!(actual_prior, &prior);
                            assert_eq!(actual_requested, &requested);
                            assert_nft_source(read_source, "desired-read", libc::EIO);
                            assert!(replacement_observed.is_none());
                            assert_eq!(rollback_observed, &semantic_rollback_observed);
                        }
                        _ => {
                            panic!("wrong D15 disposition for {trigger:?}/{rollback:?}: {error:?}")
                        }
                    }
                    assert_error_chain_source(
                        &error,
                        match (trigger, rollback) {
                            (_, RollbackOutcome::WriteFailure) => {
                                Some(("rollback-write", libc::EBUSY))
                            }
                            (_, RollbackOutcome::ReadFailure) => {
                                Some(("rollback-read", libc::ENODATA))
                            }
                            (
                                Trigger::SemanticMismatch,
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch,
                            ) => None,
                            (
                                Trigger::DesiredReadFailure,
                                RollbackOutcome::ExactRestoration
                                | RollbackOutcome::SemanticMismatch,
                            ) => Some(("desired-read", libc::EIO)),
                        },
                    );

                    let after = io.snapshot();
                    assert_eq!(after.dynamic_elements, before.dynamic_elements);
                    assert_eq!(after.foreign_objects, before.foreign_objects);
                    assert!(after.remaining_fault_schedule.is_empty());
                    assert_eq!(
                        after.owned_program,
                        match rollback {
                            RollbackOutcome::WriteFailure => post_commit,
                            RollbackOutcome::ReadFailure | RollbackOutcome::ExactRestoration => {
                                prior.clone()
                            }
                            RollbackOutcome::SemanticMismatch => {
                                semantic_rollback_observed.clone()
                            }
                        }
                    );

                    let rollback_expected = match trigger {
                        Trigger::SemanticMismatch => semantic_replacement_observed,
                        Trigger::DesiredReadFailure => Some(requested.clone()),
                    };
                    let rollback_disposition = match rollback {
                        RollbackOutcome::WriteFailure => MutationDisposition::Rejected {
                            operation: "rollback-write",
                            errno: libc::EBUSY,
                        },
                        RollbackOutcome::ReadFailure
                        | RollbackOutcome::ExactRestoration
                        | RollbackOutcome::SemanticMismatch => MutationDisposition::Committed,
                    };
                    assert_eq!(
                        after.conditional_mutations,
                        [
                            ConditionalMutation {
                                expected_current: prior.clone(),
                                desired: Some(requested.clone()),
                                disposition: MutationDisposition::Committed,
                            },
                            ConditionalMutation {
                                expected_current: rollback_expected,
                                desired: prior.clone(),
                                disposition: rollback_disposition,
                            },
                        ]
                    );
                }
            }
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one state-delta narrative covers refusal, create, retarget, reapply, and unpublished cleanup"
    )]
    fn shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta()
    {
        let prior = program(19_000);
        let requested = program(20_000);
        assert_program_target_only_delta(&prior, &requested);

        // A rejected desired batch preserves every owned and complementary
        // byte and never manufactures a rollback mutation.
        {
            let io = Arc::new(StatefulIo::new(
                Some(prior.clone()),
                [
                    StatefulStep::ObserveCurrent,
                    StatefulStep::ReplaceError { operation: "desired-replace", errno: libc::EBUSY },
                ],
            ));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("rejected desired batch refuses without rollback");
            match &error {
                InterceptError::NftSharedReplaceFailed {
                    prior: Some(actual_prior),
                    requested: actual_requested,
                    source,
                } => {
                    assert_eq!(actual_prior, &prior);
                    assert_eq!(actual_requested, &requested);
                    assert_nft_source(source, "desired-replace", libc::EBUSY);
                }
                other => panic!("wrong rejected-batch disposition: {other:?}"),
            }
            assert_error_chain_source(&error, Some(("desired-replace", libc::EBUSY)));
            let after = io.snapshot();
            assert_eq!(after.owned_program, before.owned_program);
            assert_eq!(after.dynamic_elements, before.dynamic_elements);
            assert_eq!(after.foreign_objects, before.foreign_objects);
            assert!(after.remaining_fault_schedule.is_empty());
            assert_eq!(
                after.conditional_mutations,
                [ConditionalMutation {
                    expected_current: Some(prior.clone()),
                    desired: Some(requested.clone()),
                    disposition: MutationDisposition::Rejected {
                        operation: "desired-replace",
                        errno: libc::EBUSY,
                    },
                }]
            );
        }

        // Genuine absence creates the requested program, reads it back, and
        // arms one conditional unpublished cleanup on guard Drop.
        {
            let io = Arc::new(StatefulIo::new(
                None,
                [
                    StatefulStep::ObserveCurrent,
                    StatefulStep::ReplaceCommit,
                    StatefulStep::ObserveCurrent,
                    StatefulStep::ReplaceCommit,
                ],
            ));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested.clone())
                .expect("fresh create returns an armed unpublished guard");
            assert_eq!(io.snapshot().owned_program.as_ref(), Some(&requested));
            drop(guard);
            let after = io.snapshot();
            assert_eq!(after.owned_program, None);
            assert_eq!(after.dynamic_elements, before.dynamic_elements);
            assert_eq!(after.foreign_objects, before.foreign_objects);
            assert!(after.remaining_fault_schedule.is_empty());
            assert_eq!(
                after.conditional_mutations,
                [
                    ConditionalMutation {
                        expected_current: None,
                        desired: Some(requested.clone()),
                        disposition: MutationDisposition::Committed,
                    },
                    ConditionalMutation {
                        expected_current: Some(requested.clone()),
                        desired: None,
                        disposition: MutationDisposition::Committed,
                    },
                ]
            );
        }

        // Existing-prior retarget changes only the two listener registers.
        {
            let io = Arc::new(StatefulIo::new(
                Some(prior.clone()),
                [
                    StatefulStep::ObserveCurrent,
                    StatefulStep::ReplaceCommit,
                    StatefulStep::ObserveCurrent,
                    StatefulStep::ReplaceCommit,
                ],
            ));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested.clone())
                .expect("target-only replacement reads back exact");
            assert_eq!(io.snapshot().owned_program.as_ref(), Some(&requested));
            drop(guard);
            let after = io.snapshot();
            assert_eq!(after.owned_program, None);
            assert_eq!(after.dynamic_elements, before.dynamic_elements);
            assert_eq!(after.foreign_objects, before.foreign_objects);
            assert!(after.remaining_fault_schedule.is_empty());
            assert_eq!(
                after.conditional_mutations,
                [
                    ConditionalMutation {
                        expected_current: Some(prior),
                        desired: Some(requested.clone()),
                        disposition: MutationDisposition::Committed,
                    },
                    ConditionalMutation {
                        expected_current: Some(requested.clone()),
                        desired: None,
                        disposition: MutationDisposition::Committed,
                    },
                ]
            );
        }

        // Exact reapply performs no desired write, but the adopted
        // unpublished guard remains responsible for conditional cleanup.
        {
            let io = Arc::new(StatefulIo::new(
                Some(requested.clone()),
                [StatefulStep::ObserveCurrent, StatefulStep::ReplaceCommit],
            ));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested.clone())
                .expect("exact identity is adopted without a desired write");
            assert!(io.snapshot().conditional_mutations.is_empty());
            drop(guard);
            let after = io.snapshot();
            assert_eq!(after.owned_program, None);
            assert_eq!(after.dynamic_elements, before.dynamic_elements);
            assert_eq!(after.foreign_objects, before.foreign_objects);
            assert!(after.remaining_fault_schedule.is_empty());
            assert_eq!(
                after.conditional_mutations,
                [ConditionalMutation {
                    expected_current: Some(requested),
                    desired: None,
                    disposition: MutationDisposition::Committed,
                }]
            );
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement() {
        let actual = program(19_000);
        let stale_caller_prior = program(18_000);
        let io = Arc::new(StatefulIo::new(Some(actual.clone()), [StatefulStep::ObserveCurrent]));
        let before = io.snapshot();
        let host = HostMtlsIntercept::with_shared_program_io(io.clone());

        let error = host
            .converge_shared(
                Some(&stale_caller_prior),
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 20_000),
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 20_001),
            )
            .err()
            .expect("a stale caller snapshot refuses before mutation");
        assert!(matches!(
            &error,
            InterceptError::PostconditionMismatch {
                expected,
                observed: Some(observed),
            } if expected == &stale_caller_prior && observed == &actual
        ));
        assert_error_chain_source(&error, None);

        let after = io.snapshot();
        assert_eq!(after.owned_program, before.owned_program);
        assert_eq!(after.dynamic_elements, before.dynamic_elements);
        assert_eq!(after.foreign_objects, before.foreign_objects);
        assert!(after.conditional_mutations.is_empty());
        assert!(after.remaining_fault_schedule.is_empty());

        for (leg_f, leg_c, label) in [
            (
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 20_001),
                "leg F",
            ),
            (
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 20_000),
                SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
                "leg C",
            ),
        ] {
            let io =
                Arc::new(StatefulIo::new(Some(actual.clone()), [StatefulStep::ObserveCurrent]));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .converge_shared(Some(&actual), leg_f, leg_c)
                .err()
                .unwrap_or_else(|| panic!("zero {label} port must refuse before I/O"));
            assert!(matches!(
                &error,
                InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", .. }
            ));
            let source =
                error.source().expect("zero-port refusal retains its semantic-construction source");
            assert!(source.is::<NetlinkError>());
            assert_eq!(
                io.snapshot(),
                before,
                "zero {label} refusal consumes no observation, fault, or mutation"
            );
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one finite table keeps the five disjoint source-honest rollback dispositions together"
    )]
    #[ignore = "superseded by D15 stateful shared-IP evidence"]
    fn replacement_and_every_rollback_disposition_preserve_exact_identity_and_source() {
        let requested = program(20_000);
        let wrong_replacement = program(20_100);

        // Clean first boot: successful rollback must restore absence, not a
        // fabricated empty program.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(None), Ok(Some(wrong_replacement.clone())), Ok(None)],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("replacement mismatch refuses startup after exact rollback");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplacementMismatchRolledBack {
                    prior: None,
                    requested: ref actual,
                    replacement_observed: Some(ref observed),
                } if actual == &requested && observed == &wrong_replacement
            ));
            assert_eq!(
                io.calls(),
                [
                    Call::Observe,
                    Call::ReplacePresent,
                    Call::Observe,
                    Call::RestoreAbsent,
                    Call::Observe
                ]
            );
        }

        let prior = program(19_000);

        // Atomic replacement rejection preserves the exact prior and retains
        // the real netlink source without attempting rollback.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone()))],
                vec![Err(netlink_error("replace", libc::EBUSY))],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("replacement rejection refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplaceFailed {
                    prior: Some(ref actual_prior),
                    requested: ref actual_requested,
                    source: NetlinkError::Nft { .. },
                } if actual_prior == &prior && actual_requested == &requested
            ));
            assert_eq!(io.calls(), [Call::Observe, Call::ReplacePresent]);
        }

        // Rollback write failure names RestorePrior and retains the direct
        // netlink source plus prior/requested/replacement observations.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone())), Ok(Some(wrong_replacement.clone()))],
                vec![Ok(()), Err(netlink_error("restore-prior", libc::EIO))],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("rollback write failure refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackFailed {
                    operation: InterceptSharedRollbackOperation::RestorePrior,
                    source: NetlinkError::Nft { .. },
                    ..
                }
            ));
        }

        // Rollback read failure names ReadBackPrior and retains its own real
        // source rather than fabricating a semantic mismatch.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![
                    Ok(Some(prior.clone())),
                    Ok(Some(wrong_replacement.clone())),
                    Err(netlink_error("read-back-prior", libc::EIO)),
                ],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("rollback read failure refuses startup");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackFailed {
                    operation: InterceptSharedRollbackOperation::ReadBackPrior,
                    source: NetlinkError::Nft { .. },
                    ..
                }
            ));
        }

        // Successful rollback I/O with the wrong identity is the source-less
        // semantic mismatch variant and retains both observations.
        {
            let wrong_rollback = program(18_000);
            let io = Arc::new(ScriptedIo::new(
                vec![
                    Ok(Some(prior.clone())),
                    Ok(Some(wrong_replacement.clone())),
                    Ok(Some(wrong_rollback.clone())),
                ],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io);
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("wrong rollback identity refuses startup without a fake source");
            assert!(matches!(
                error,
                InterceptError::NftSharedRollbackPostconditionMismatch {
                    prior: Some(ref actual_prior),
                    requested: ref actual_requested,
                    replacement_observed: Some(ref replacement),
                    rollback_observed: Some(ref rollback),
                    ..
                } if actual_prior == &prior
                    && actual_requested == &requested
                    && replacement == &wrong_replacement
                    && rollback == &wrong_rollback
            ));
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "superseded by D15 stateful shared-IP evidence"]
    fn fresh_replace_exact_prior_rollback_and_idempotent_reapply_are_complete() {
        let requested = program(20_000);

        // Fresh absence installs and reads back the exact requested identity.
        {
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(None), Ok(Some(requested.clone()))],
                vec![Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested.clone())
                .expect("fresh shared program replacement succeeds");
            assert_eq!(io.calls(), [Call::Observe, Call::ReplacePresent, Call::Observe]);
            drop(guard);
        }

        // A mismatched replacement restores an exact present prior and returns
        // the source-less rolled-back disposition.
        {
            let prior = program(19_000);
            let wrong = program(21_000);
            let io = Arc::new(ScriptedIo::new(
                vec![Ok(Some(prior.clone())), Ok(Some(wrong.clone())), Ok(Some(prior.clone()))],
                vec![Ok(()), Ok(())],
            ));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error = host
                .replace_shared_program_for_boot(requested.clone())
                .err()
                .expect("wrong replacement is rolled back to the exact prior");
            assert!(matches!(
                error,
                InterceptError::NftSharedReplacementMismatchRolledBack {
                    prior: Some(actual_prior),
                    requested: actual_requested,
                    replacement_observed: Some(actual_wrong),
                } if actual_prior == prior
                    && actual_requested == requested
                    && actual_wrong == wrong
            ));
            assert_eq!(
                io.calls(),
                [
                    Call::Observe,
                    Call::ReplacePresent,
                    Call::Observe,
                    Call::RestorePresent,
                    Call::Observe
                ]
            );
        }

        // Reapplying an already exact identity adopts it without rewriting.
        {
            let io = Arc::new(ScriptedIo::new(vec![Ok(Some(requested.clone()))], Vec::new()));
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let guard = host
                .replace_shared_program_for_boot(requested)
                .expect("exact prior identity is an idempotent adoption");
            assert_eq!(io.calls(), [Call::Observe]);
            drop(guard);
        }
    }

    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn runtime_present_wrong_target_and_observe_error_are_non_mutating() {
        let wrong_target = canonical_program(20_100, 20_101);
        let io =
            Arc::new(StatefulIo::new(Some(wrong_target.clone()), [StatefulStep::ObserveCurrent]));
        let before = io.snapshot();
        let host = HostMtlsIntercept::with_shared_program_io(io.clone());
        assert_eq!(
            host.observe_shared().expect("canonical wrong target remains a valid observation"),
            Some(wrong_target.clone())
        );
        let after = io.snapshot();
        assert_eq!(after.owned_program, before.owned_program);
        assert_eq!(after.dynamic_elements, before.dynamic_elements);
        assert_eq!(after.foreign_objects, before.foreign_objects);
        assert_eq!(after.observations, [ObservationDisposition::Returned(Some(wrong_target))]);
        assert!(after.conditional_mutations.is_empty());
        assert!(after.remaining_fault_schedule.is_empty());

        for (label, operation, errno) in [
            ("partial", "observe-partial", libc::ENODATA),
            ("foreign", "observe-foreign", libc::EEXIST),
            ("duplicate", "observe-duplicate", libc::EBUSY),
            ("malformed", "observe-malformed", libc::EINVAL),
            ("lower", "observe-lower", libc::EIO),
        ] {
            let io = Arc::new(StatefulIo::new(
                Some(canonical_program(20_100, 20_101)),
                [StatefulStep::ObserveError { operation, errno }],
            ));
            let before = io.snapshot();
            let host = HostMtlsIntercept::with_shared_program_io(io.clone());
            let error =
                host.observe_shared().expect_err("invalid or failed observation remains typed");
            assert!(
                matches!(
                    &error,
                    InterceptError::NftRuleInstallFailed {
                        op: "observe-shared",
                        source,
                    } if matches!(
                        source,
                        NetlinkError::Nft { op, source }
                            if *op == operation && source.raw_os_error() == Some(errno)
                    )
                ),
                "{label} observation did not retain its exact typed source: {error:?}"
            );
            assert_error_chain_source(&error, Some((operation, errno)));
            let after = io.snapshot();
            assert_eq!(after.owned_program, before.owned_program);
            assert_eq!(after.dynamic_elements, before.dynamic_elements);
            assert_eq!(after.foreign_objects, before.foreign_objects);
            assert_eq!(after.observations, [ObservationDisposition::Failed { operation, errno }]);
            assert!(after.conditional_mutations.is_empty());
            assert!(after.remaining_fault_schedule.is_empty());
        }
    }
}
