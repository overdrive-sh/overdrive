//! `SimMtlsIntercept` — in-memory
//! [`MtlsIntercept`](overdrive_worker::mtls_intercept_port::MtlsIntercept)
//! double with standing per-method fault scripting (GH #250/GH #295,
//! ADR-0076 § 4.6).
//!
//! The sim counterpart to `overdrive_worker::mtls_intercept_port::HostMtlsIntercept`.
//! It exists for ONE reason: nothing in the tree can make
//! `MtlsInterceptWorker::start_alloc` fail on demand, so the security-relevant
//! call-site ordering the action-shim's fail-closed arms depend on (a
//! now-`Failed` allocation must never release its exit watcher) is otherwise
//! wholly untested. This double is the mechanism that makes it exercisable.
//!
//! # What it models, and what it does not
//!
//! - **Fault arms are PURE** — an armed fault short-circuits before any
//!   syscall, so a test that drives only fault arms performs ZERO I/O and
//!   belongs in the DEFAULT lane.
//! - **The `Ok` arm of `bind_transparent` binds a REAL, PLAIN
//!   (non-`IP_TRANSPARENT`) loopback listener** (DFS-5). The worker's `Ok` path
//!   consumes a live listener it accepts on, and there is no way to fabricate a
//!   [`std::net::TcpListener`] without a syscall. **Any test that drives this
//!   `Ok` arm binds a socket and is therefore INTEGRATION-lane** per
//!   `.claude/rules/testing.md` § "Integration vs unit gating".
//! - **The `Ok` arm of the two allocation installs records the dynamic members
//!   the shared owner's host adapter would add** (one managed-guest and one
//!   outbound-source member per outbound install; one inbound-destination
//!   member per inbound install) under a process-local element token. The
//!   guard's `Drop` removes its members unless
//!   [`remove_allocation_elements`](MtlsIntercept::remove_allocation_elements)
//!   retired its token first. `converge_shared`'s guard is inert. No nft rule
//!   exists, so the member model is observable only through
//!   [`observe_shared_state`](MtlsIntercept::observe_shared_state).
//! - **The listener surface** ([`SimInterceptListener`], [`SimAcceptScript`])
//!   opens no socket, descriptor, thread, task, or timer: a fabricated port,
//!   a weak live-listener table, and scripted accept outcomes. Until the step
//!   that changes `bind_transparent`'s return type (DELIVER 05-01),
//!   `bind_transparent`'s `Ok` arm still returns a real socket, so the table
//!   stays empty and both scripting calls return `false`.
//!
//! # Determinism
//!
//! Holds no clock, no entropy, no store, and no collection whose iteration
//! order is observed. Each method's outcome is a pure function of its armed
//! fault. Shared convergence additionally records one substrate-neutral
//! identity so caller-side audit and recovery ordering are observable without
//! pretending to implement or prove the host nft algorithm.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use overdrive_worker::mtls_intercept::{
    InterceptElementKey, InterceptElementOperation, InterceptError, InterceptPostcondition,
    InterceptSet, NetlinkError, Result,
};
use overdrive_worker::mtls_intercept_port::{
    InterceptAcceptError, InterceptAccepted, InterceptGuard, InterceptListener, InterceptMembers,
    InterceptState, MtlsIntercept,
};
use parking_lot::Mutex;
use tokio::sync::Notify;

/// A scripted intercept-install fault, expressed in the REAL error shapes the
/// production substrate produces (research Finding 5.3 — inject errors that
/// naturally occur, not a generic boolean "fail now").
///
/// `Clone` + `Copy` because a scripted fault is STANDING, not one-shot (see
/// [`SimMtlsIntercept`]): the same fault must be re-materialised on every call
/// while armed, and [`InterceptError`] is not `Clone` (it carries
/// [`std::io::Error`]). Every field is now a small `Copy` value — an `errno`
/// (`i32`) and a `&'static str` op — so the descriptor is trivially `Copy`; the
/// per-call `materialise` rebuilds the fresh, non-`Copy` [`InterceptError`] from
/// it (a raw errno-bearing `io::Error` / [`NetlinkError`]).
#[derive(Debug, Clone, Copy)]
pub enum SimInterceptFault {
    /// Materialises `InterceptError::TransparentListener { addr, source:
    /// io::Error::from_raw_os_error(errno) }` — the missing-`CAP_NET_ADMIN`
    /// (`libc::EPERM`), unsupported-setopt (`libc::ENOPROTOOPT`), and
    /// address-in-use (`libc::EADDRINUSE`) shapes. `addr` is the address the
    /// faulted call was made with.
    TransparentListener {
        /// The `errno` the real syscall would have returned.
        errno: i32,
    },
    /// Materialises `InterceptError::NftRuleInstallFailed { op, source }` — the
    /// hand-rolled nftables `NETLINK_NETFILTER` op failure (ADR-0085 D3). The
    /// `source` is a real errno-carrying [`NetlinkError::Nft`], exactly as the
    /// production `nft::append_rule` / `ensure_*` path produces it (missing
    /// `CAP_NET_ADMIN` → `EPERM`, ruleset lock contention → `EBUSY`).
    NftRuleInstall {
        /// The failing nft op the real path names (`append-egress` /
        /// `append-inbound` / `ensure-table` / …).
        op: &'static str,
        /// The kernel `errno` the real `NLMSG_ERROR` would carry.
        errno: i32,
    },
    /// Materialises `InterceptError::NftElementUpdateFailed { set, operation,
    /// key, source }` — refusal 3 of the pinned install partition (DISTILL gap
    /// B-8): the element insert or its mandatory read-back failed. `set` and
    /// `key` are those of the call's first element, as the host reports them
    /// (an outbound install: `ManagedGuestIps` and the source address; an
    /// inbound install: `InboundDestinations` and the destination). The
    /// `source` is a real errno-carrying `NetlinkError::Nft`. Sanctioned on
    /// `install_outbound` and `install_inbound` only; armed on any other
    /// method it materialises as that method's call key would, which models a
    /// substrate the real one cannot exhibit (a test defect).
    NftElementUpdate {
        /// `Insert` or `ReadBack`, the two install-time element operations.
        operation: InterceptElementOperation,
        /// The kernel `errno` the element batch or read-back would carry.
        errno: i32,
    },
}

/// In-memory [`MtlsIntercept`] double with per-method fault scripting.
///
/// # Fault lifetime — STANDING, not one-shot
///
/// An armed fault fires on EVERY subsequent call to that method until re-armed
/// or [`clear_faults`](Self::clear_faults). This deliberately diverges from
/// `SimMtlsResolve`'s consume-on-use `.take()` shape, because the faults differ
/// in kind: a poisoned store handle is transient, whereas a missing
/// `CAP_NET_ADMIN` or an absent `nft` binary fails EVERY call. Standing faults
/// also remove call-order dependence — `start_alloc` calls `bind_transparent`
/// twice (leg-F then leg-C), and a consume-on-use fault would make "which leg
/// failed" an artifact of ordering rather than the test's explicit choice.
///
/// # Out-of-contract fault pairings
///
/// [`SimInterceptFault`] is one type shared by every scripting helper, so the
/// compiler permits arming a shape on a method whose contract says it never
/// returns that shape. Doing so models a substrate the real one cannot exhibit.
/// The SANCTIONED pairings are:
///
/// - `bind_transparent` ⇔ [`TransparentListener`](SimInterceptFault::TransparentListener);
/// - `converge_shared` and `observe_shared` ⇔
///   [`NftRuleInstall`](SimInterceptFault::NftRuleInstall) with op
///   `observe-shared`, the host's one failure before any write (DISTILL gap
///   B-8);
/// - `install_outbound` and `install_inbound` ⇔
///   [`NftElementUpdate`](SimInterceptFault::NftElementUpdate), refusal 3 of
///   the pinned install partition (DISTILL gap B-8).
///
/// Arming any other pairing is a test defect, not a supported scenario.
#[allow(
    clippy::struct_field_names,
    reason = "the shared `_fault` postfix is load-bearing: each field is the STANDING fault slot \
              backing one trait method, and the prefix names that method. Dropping it would leave \
              `bind` / `outbound` / `inbound`, which read as the installs themselves rather than \
              as the faults armed against them. Names pinned verbatim by ADR-0076 § 4.6."
)]
pub struct SimMtlsIntercept {
    /// Standing fault for [`converge_shared`](MtlsIntercept::converge_shared).
    converge_shared_fault: Mutex<Option<SimInterceptFault>>,
    /// Standing fault for [`observe_shared`](MtlsIntercept::observe_shared).
    observe_shared_fault: Mutex<Option<SimInterceptFault>>,
    /// The complete shared identity last installed through the sim port.
    shared_observation: Mutex<Option<InterceptPostcondition>>,
    /// Standing fault for [`bind_transparent`](MtlsIntercept::bind_transparent).
    bind_fault: Mutex<Option<SimInterceptFault>>,
    /// Standing fault for [`install_outbound`](MtlsIntercept::install_outbound).
    outbound_fault: Mutex<Option<SimInterceptFault>>,
    /// Standing fault for [`install_inbound`](MtlsIntercept::install_inbound).
    inbound_fault: Mutex<Option<SimInterceptFault>>,
    /// The owned program's dynamic members and their live element tokens.
    members: Arc<Mutex<SimMemberState>>,
    /// Weak live-listener table keyed by bound address. The adapter never
    /// extends a listener's life.
    listeners: Mutex<BTreeMap<SocketAddrV4, Weak<SimInterceptListener>>>,
}

/// The inert guard `converge_shared`'s `Ok` arm returns. No rule was
/// installed, so `Drop` removes nothing. Private — consumers see only
/// `Box<dyn InterceptGuard>`.
struct InertGuard;

impl InterceptGuard for InertGuard {}

/// One dynamic member of the owned program's three sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SimMember {
    ManagedGuest(Ipv4Addr),
    OutboundSource(Ipv4Addr),
    InboundDestination(SocketAddrV4),
}

/// The member sets plus the process-local element tokens that own them.
#[derive(Debug, Default)]
struct SimMemberState {
    members: InterceptMembers,
    /// Live token per member: `(generation, holders)`. A retired token is
    /// absent, so a guard holding its generation removes nothing.
    tokens: BTreeMap<SimMember, (u64, usize)>,
    next_generation: u64,
}

impl SimMemberState {
    fn insert(&mut self, member: SimMember) {
        match member {
            SimMember::ManagedGuest(address) => {
                self.members.managed_guest_ips.insert(address);
            }
            SimMember::OutboundSource(address) => {
                self.members.outbound_sources.insert(address);
            }
            SimMember::InboundDestination(destination) => {
                self.members.inbound_destinations.insert(destination);
            }
        }
    }

    fn remove(&mut self, member: SimMember) {
        match member {
            SimMember::ManagedGuest(address) => {
                self.members.managed_guest_ips.remove(&address);
            }
            SimMember::OutboundSource(address) => {
                self.members.outbound_sources.remove(&address);
            }
            SimMember::InboundDestination(destination) => {
                self.members.inbound_destinations.remove(&destination);
            }
        }
    }

    /// Acquire one holder of each member's token, adding absent members; a
    /// re-install adopts the live token rather than duplicating the member.
    fn acquire(&mut self, members: &[SimMember]) -> Vec<(SimMember, u64)> {
        members
            .iter()
            .map(|member| {
                let generation = if let Some((generation, holders)) = self.tokens.get_mut(member) {
                    *holders += 1;
                    *generation
                } else {
                    self.next_generation += 1;
                    let generation = self.next_generation;
                    self.tokens.insert(*member, (generation, 1));
                    self.insert(*member);
                    generation
                };
                (*member, generation)
            })
            .collect()
    }
}

/// The guard an allocation install's `Ok` arm returns: its `Drop` releases one
/// holder of each member token it acquired and removes a member whose last
/// holder dropped, unless the token was retired meanwhile.
struct SimElementGuard {
    state: Arc<Mutex<SimMemberState>>,
    keys: Vec<(SimMember, u64)>,
}

impl InterceptGuard for SimElementGuard {}

impl Drop for SimElementGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock();
        for (member, generation) in &self.keys {
            let Some((live, holders)) = state.tokens.get_mut(member) else {
                continue;
            };
            if *live != *generation {
                continue;
            }
            if *holders > 1 {
                *holders -= 1;
            } else {
                state.tokens.remove(member);
                state.remove(*member);
            }
        }
    }
}

/// One scripted outcome for the next `accept` of a [`SimInterceptListener`].
#[derive(Debug)]
pub enum SimAcceptScript {
    /// The next `accept` returns `Ok(accepted)`.
    Connection(InterceptAccepted),
    /// The next `accept` returns `Err(OriginalDestination { source })`, with
    /// `io::Error::from_raw_os_error(errno)`; the listener stays usable.
    OriginalDestinationFailure { errno: i32 },
    /// That and every later `accept` returns `Err(Accept { source })`, with
    /// `io::Error::from_raw_os_error(errno)`: the listener accepts nothing more.
    ListenerLost { errno: i32 },
}

/// The socket-free [`InterceptListener`] of [`SimMtlsIntercept`].
///
/// Live from its bind until its last `Arc` drops; the adapter holds only a
/// `Weak` reference. It holds no socket, descriptor, thread, task, timer,
/// clock, or entropy.
#[derive(Debug)]
pub struct SimInterceptListener {
    addr: SocketAddrV4,
    state: Mutex<SimListenerState>,
    wake: Notify,
}

#[derive(Debug, Default)]
struct SimListenerState {
    /// Scripted outcomes, FIFO.
    scripts: VecDeque<SimAcceptScript>,
    /// Set once a `ListenerLost` script is taken: every later accept fails.
    lost: Option<i32>,
    /// Set by `script_local_addr_failure`: every later `local_addr` fails.
    local_addr_errno: Option<i32>,
    /// Accept futures that have been polled, are pending, and are not dropped.
    parked: usize,
}

/// Decrements the parked count when a pending accept completes or is dropped.
struct ParkedAccept<'a> {
    listener: &'a SimInterceptListener,
}

impl Drop for ParkedAccept<'_> {
    fn drop(&mut self) {
        self.listener.state.lock().parked -= 1;
    }
}

impl SimInterceptListener {
    fn new(addr: SocketAddrV4) -> Self {
        Self { addr, state: Mutex::new(SimListenerState::default()), wake: Notify::new() }
    }

    /// Take the next outcome, if one is decided: a lost listener, or the head
    /// of the script FIFO.
    fn take_outcome(&self) -> Option<std::result::Result<InterceptAccepted, InterceptAcceptError>> {
        let script = {
            let mut state = self.state.lock();
            if let Some(errno) = state.lost {
                return Some(Err(InterceptAcceptError::Accept {
                    source: std::io::Error::from_raw_os_error(errno),
                }));
            }
            let script = state.scripts.pop_front()?;
            if let SimAcceptScript::ListenerLost { errno } = script {
                state.lost = Some(errno);
            }
            script
        };
        Some(match script {
            SimAcceptScript::Connection(accepted) => Ok(accepted),
            SimAcceptScript::OriginalDestinationFailure { errno } => {
                Err(InterceptAcceptError::OriginalDestination {
                    source: std::io::Error::from_raw_os_error(errno),
                })
            }
            SimAcceptScript::ListenerLost { errno } => Err(InterceptAcceptError::Accept {
                source: std::io::Error::from_raw_os_error(errno),
            }),
        })
    }
}

#[async_trait]
impl InterceptListener for SimInterceptListener {
    fn local_addr(&self) -> std::io::Result<SocketAddrV4> {
        let errno = self.state.lock().local_addr_errno;
        errno.map_or(Ok(self.addr), |errno| Err(std::io::Error::from_raw_os_error(errno)))
    }

    async fn accept(&self) -> std::result::Result<InterceptAccepted, InterceptAcceptError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(InterceptAcceptError::Accept {
                source: std::io::Error::other(
                    "sim intercept listener accept polled with no current Tokio runtime",
                ),
            });
        }
        let mut parked: Option<ParkedAccept<'_>> = None;
        loop {
            // Created before the outcome is read, so a script appended
            // between the read and the await still wakes this future.
            let notified = self.wake.notified();
            if let Some(outcome) = self.take_outcome() {
                drop(parked);
                return outcome;
            }
            if parked.is_none() {
                self.state.lock().parked += 1;
                parked = Some(ParkedAccept { listener: self });
            }
            notified.await;
        }
    }
}

impl SimMtlsIntercept {
    /// A double with NO fault armed — every method takes its `Ok` arm. Faults
    /// are armed explicitly through the scripting helpers below (no builder
    /// over a *dependency*; these script OUTCOMES, mirroring `SimMtlsResolve`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            converge_shared_fault: Mutex::new(None),
            observe_shared_fault: Mutex::new(None),
            shared_observation: Mutex::new(None),
            bind_fault: Mutex::new(None),
            outbound_fault: Mutex::new(None),
            inbound_fault: Mutex::new(None),
            members: Arc::new(Mutex::new(SimMemberState::default())),
            listeners: Mutex::new(BTreeMap::new()),
        }
    }

    /// Append `script` to the FIFO of the live listener at `at` and wake a
    /// parked `accept`. `false`, recording nothing, when no live listener
    /// holds `at`.
    #[must_use]
    pub fn script_accept(&self, at: SocketAddrV4, script: SimAcceptScript) -> bool {
        let Some(listener) = self.live_listener(at) else {
            return false;
        };
        listener.state.lock().scripts.push_back(script);
        listener.wake.notify_waiters();
        true
    }

    /// Make the live listener at `at` report
    /// `Err(io::Error::from_raw_os_error(errno))` from `local_addr()` from now
    /// on. `false`, recording nothing, when no live listener holds `at`.
    #[must_use]
    pub fn script_local_addr_failure(&self, at: SocketAddrV4, errno: i32) -> bool {
        let Some(listener) = self.live_listener(at) else {
            return false;
        };
        listener.state.lock().local_addr_errno = Some(errno);
        true
    }

    /// The addresses of every live listener, ascending.
    #[must_use]
    pub fn live_listeners(&self) -> Vec<SocketAddrV4> {
        self.listeners
            .lock()
            .iter()
            .filter(|(_, listener)| listener.strong_count() > 0)
            .map(|(addr, _)| *addr)
            .collect()
    }

    /// The live listener at `at`'s pending, polled, not-dropped `accept`
    /// futures; 0 when no live listener holds `at`.
    #[must_use]
    pub fn parked_accepts(&self, at: SocketAddrV4) -> usize {
        self.live_listener(at).map_or(0, |listener| listener.state.lock().parked)
    }

    fn live_listener(&self, at: SocketAddrV4) -> Option<Arc<SimInterceptListener>> {
        self.listeners.lock().get(&at).and_then(Weak::upgrade)
    }

    /// Register a socket-free listener: port 0 takes the smallest port ≥ 49152
    /// no live listener of this adapter holds at that IP; a non-zero address is
    /// honoured exactly; an address a live listener holds is refused with
    /// `EADDRINUSE`.
    #[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-01 (B-7)")]
    #[allow(
        clippy::result_large_err,
        reason = "returns the exact InterceptError the port's bind_transparent returns"
    )]
    fn register_listener(&self, addr: SocketAddrV4) -> Result<Arc<SimInterceptListener>> {
        let mut listeners = self.listeners.lock();
        listeners.retain(|_, listener| listener.strong_count() > 0);
        let bound = if addr.port() == 0 {
            let port = (49_152..=u16::MAX)
                .find(|port| !listeners.contains_key(&SocketAddrV4::new(*addr.ip(), *port)))
                .ok_or_else(|| InterceptError::TransparentListener {
                    addr,
                    source: std::io::Error::from_raw_os_error(libc::EADDRINUSE),
                })?;
            SocketAddrV4::new(*addr.ip(), port)
        } else if listeners.contains_key(&addr) {
            return Err(InterceptError::TransparentListener {
                addr,
                source: std::io::Error::from_raw_os_error(libc::EADDRINUSE),
            });
        } else {
            addr
        };
        let listener = Arc::new(SimInterceptListener::new(bound));
        listeners.insert(bound, Arc::downgrade(&listener));
        drop(listeners);
        Ok(listener)
    }

    /// One snapshot of the owned program and its members, or `None` when no
    /// program has been converged.
    fn state_snapshot(&self) -> Option<InterceptState> {
        let program = self.shared_observation.lock().clone()?;
        let members = self.members.lock().members.clone();
        Some(InterceptState { program, policy_route: true, intercept_mark_guard: true, members })
    }

    /// The typed refusal for a member effect while no program is published —
    /// the shape the host adapter returns.
    fn program_not_published() -> InterceptError {
        InterceptError::NftRuleInstallFailed {
            op: "shared-element-owner",
            source: NetlinkError::nft(
                "shared-element-owner",
                std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "shared constant program is not published",
                ),
            ),
        }
    }

    /// Arm a STANDING fault on `bind_transparent`. Fires on every subsequent
    /// call until re-armed or [`clear_faults`](Self::clear_faults).
    pub fn script_bind_fault(&self, fault: SimInterceptFault) {
        *self.bind_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `converge_shared`.
    ///
    /// This is a deterministic caller-reaction seam. It does not claim to
    /// prove the host adapter's replacement/read-back/rollback algorithm,
    /// which is exercised above the private host I/O seam.
    pub fn script_converge_shared_fault(&self, fault: SimInterceptFault) {
        *self.converge_shared_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `observe_shared`.
    ///
    /// This scripts only the public port result used by composition tests;
    /// host read-back correctness remains source-local to the host adapter.
    pub fn script_observe_shared_fault(&self, fault: SimInterceptFault) {
        *self.observe_shared_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `install_outbound`.
    pub fn script_outbound_fault(&self, fault: SimInterceptFault) {
        *self.outbound_fault.lock() = Some(fault);
    }

    /// Arm a STANDING fault on `install_inbound`.
    pub fn script_inbound_fault(&self, fault: SimInterceptFault) {
        *self.inbound_fault.lock() = Some(fault);
    }

    /// Disarm every standing fault.
    pub fn clear_faults(&self) {
        *self.converge_shared_fault.lock() = None;
        *self.observe_shared_fault.lock() = None;
        *self.bind_fault.lock() = None;
        *self.outbound_fault.lock() = None;
        *self.inbound_fault.lock() = None;
    }
}

impl Default for SimMtlsIntercept {
    fn default() -> Self {
        Self::new()
    }
}

/// Materialise an armed fault descriptor into a FRESH [`InterceptError`].
///
/// A fresh error per call is what makes the STANDING lifetime possible:
/// [`InterceptError`] is not `Clone` (it carries [`std::io::Error`]), so the
/// slot holds the re-materialisable descriptor and this function rebuilds the
/// error on every fire. `addr` is the address the faulted call was made with —
/// it is carried only by the `TransparentListener` shape.
fn materialise(fault: SimInterceptFault, addr: SocketAddrV4) -> InterceptError {
    match fault {
        SimInterceptFault::TransparentListener { errno } => InterceptError::TransparentListener {
            addr,
            source: std::io::Error::from_raw_os_error(errno),
        },
        // A hand-rolled nft op failure, rebuilt as the real errno-carrying
        // `NetlinkError::Nft` the production `nft::append_rule` path returns.
        SimInterceptFault::NftRuleInstall { op, errno } => InterceptError::NftRuleInstallFailed {
            op,
            source: NetlinkError::nft(op, std::io::Error::from_raw_os_error(errno)),
        },
        // Keyed by the address the call was made with: a destination tuple,
        // as an inbound install's first element is.
        SimInterceptFault::NftElementUpdate { operation, errno } => element_update_failure(
            InterceptSet::InboundDestinations,
            operation,
            InterceptElementKey::Destination(addr),
            errno,
        ),
    }
}

/// Materialise an armed fault on `install_outbound(source_addr, _)`. An
/// element-update fault is keyed as the host keys an outbound install's first
/// element: the managed-guest set and the source address. Every other shape
/// materialises as [`materialise`] does against the loopback wildcard.
fn materialise_outbound(fault: SimInterceptFault, source_addr: Ipv4Addr) -> InterceptError {
    match fault {
        SimInterceptFault::NftElementUpdate { operation, errno } => element_update_failure(
            InterceptSet::ManagedGuestIps,
            operation,
            InterceptElementKey::Address(source_addr),
            errno,
        ),
        other => materialise(other, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)),
    }
}

/// The host's refusal-3 shape: `NftElementUpdateFailed` carrying a real
/// errno-bearing `NetlinkError::Nft`.
fn element_update_failure(
    set: InterceptSet,
    operation: InterceptElementOperation,
    key: InterceptElementKey,
    errno: i32,
) -> InterceptError {
    let op = match operation {
        InterceptElementOperation::ReadBack => "shared-element-readback",
        InterceptElementOperation::Insert | InterceptElementOperation::Delete => {
            "shared-element-insert"
        }
    };
    InterceptError::NftElementUpdateFailed {
        set,
        operation,
        key,
        source: NetlinkError::nft(op, std::io::Error::from_raw_os_error(errno)),
    }
}

/// Read the armed fault out of `slot` WITHOUT consuming it (DFS-4 — the fault
/// is standing, so it must survive the call that fires it). Copying the
/// descriptor out rather than `.take()`ing it is the whole mechanism: a
/// `.take()` here would let the second call fall through to the `Ok` arm.
///
/// The copy lands in a local before the caller branches, so the lock guard is
/// released at the end of this function rather than held across the branch.
fn armed(slot: &Mutex<Option<SimInterceptFault>>) -> Option<SimInterceptFault> {
    *slot.lock()
}

impl MtlsIntercept for SimMtlsIntercept {
    fn bind_transparent(&self, addr: SocketAddrV4) -> Result<std::net::TcpListener> {
        // The fault arm is PURE — it short-circuits BEFORE the bind below, so a
        // test driving only this arm performs zero I/O and stays default-lane.
        if let Some(fault) = armed(&self.bind_fault) {
            return Err(materialise(fault, addr));
        }

        // DFS-5: the `Ok` arm binds a REAL, PLAIN (non-`IP_TRANSPARENT`)
        // listener. There is no way to fabricate a `TcpListener` without a
        // syscall, and the worker's `Ok` path consumes a live listener it
        // accepts on. Any test reaching here is INTEGRATION-lane.
        std::net::TcpListener::bind(addr)
            .map_err(|source| InterceptError::TransparentListener { addr, source })
    }

    fn converge_shared(
        &self,
        _prior: Option<&InterceptPostcondition>,
        leg_f: SocketAddrV4,
        leg_c: SocketAddrV4,
    ) -> Result<Box<dyn InterceptGuard>> {
        if let Some(fault) = armed(&self.converge_shared_fault) {
            return Err(materialise(fault, leg_f));
        }

        // The sim records a stable, complete identity that is deliberately
        // substrate-neutral: it proves the owner passes both exact listener
        // targets and later observes the same fact, not the host nft encoding.
        let encode = |addr: SocketAddrV4| {
            let mut bytes = Vec::with_capacity(6);
            bytes.extend_from_slice(&addr.ip().octets());
            bytes.extend_from_slice(&addr.port().to_be_bytes());
            bytes
        };
        *self.shared_observation.lock() = Some(InterceptPostcondition::ConstantRules {
            table_and_chains: vec![b"sim-shared-table-and-chains".to_vec()],
            sets: vec![b"sim-shared-sets".to_vec()],
            prerouting: vec![encode(leg_f)],
            output: vec![encode(leg_c)],
        });
        Ok(Box::new(InertGuard))
    }

    fn observe_shared(&self) -> Result<Option<InterceptPostcondition>> {
        if let Some(fault) = armed(&self.observe_shared_fault) {
            return Err(materialise(fault, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)));
        }
        Ok(self.shared_observation.lock().clone())
    }

    fn install_outbound(
        &self,
        source_addr: std::net::Ipv4Addr,
        _agent_leg_f_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if let Some(fault) = armed(&self.outbound_fault) {
            // An element-update fault is keyed by the source address, as the
            // host keys it. No socket address is in scope on this method, so a
            // `TransparentListener` descriptor armed here — an OUT-OF-CONTRACT
            // pairing (see the type docs: a test defect, not a supported
            // scenario) — materialises against the loopback wildcard.
            return Err(materialise_outbound(fault, source_addr));
        }

        // The double installs no nft rule; it records the two members the
        // host adapter adds, and the guard owns exactly those.
        let keys = self.members.lock().acquire(&[
            SimMember::ManagedGuest(source_addr),
            SimMember::OutboundSource(source_addr),
        ]);
        Ok(Box::new(SimElementGuard { state: Arc::clone(&self.members), keys }))
    }

    fn install_inbound(
        &self,
        virt: SocketAddrV4,
        _agent_leg_c_port: u16,
    ) -> Result<Box<dyn InterceptGuard>> {
        if let Some(fault) = armed(&self.inbound_fault) {
            return Err(materialise(fault, virt));
        }

        let keys = self.members.lock().acquire(&[SimMember::InboundDestination(virt)]);
        Ok(Box::new(SimElementGuard { state: Arc::clone(&self.members), keys }))
    }

    fn observe_shared_state(&self) -> Result<Option<InterceptState>> {
        Ok(self.state_snapshot())
    }

    fn converge_allocation_elements(
        &self,
        expected: &InterceptMembers,
    ) -> Result<Option<InterceptState>> {
        if self.shared_observation.lock().is_none() {
            return Ok(None);
        }
        self.members.lock().members = expected.clone();
        Ok(self.state_snapshot())
    }

    fn remove_allocation_elements(
        &self,
        source_addr: Ipv4Addr,
        destinations: &[SocketAddrV4],
    ) -> Result<InterceptState> {
        let mut seen = BTreeSet::new();
        if let Some(rejected) = destinations
            .iter()
            .find(|destination| destination.port() == 0 || !seen.insert(**destination))
        {
            return Err(InterceptError::NftElementUpdateFailed {
                set: InterceptSet::InboundDestinations,
                operation: InterceptElementOperation::Delete,
                key: InterceptElementKey::Destination(*rejected),
                source: NetlinkError::nft(
                    "shared-element-remove",
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "duplicate or zero-port destination",
                    ),
                ),
            });
        }
        if self.shared_observation.lock().is_none() {
            return Err(Self::program_not_published());
        }
        let requested =
            [SimMember::ManagedGuest(source_addr), SimMember::OutboundSource(source_addr)]
                .into_iter()
                .chain(
                    destinations
                        .iter()
                        .map(|destination| SimMember::InboundDestination(*destination)),
                );
        let mut state = self.members.lock();
        for member in requested {
            // Convergent: an absent member is already its postcondition.
            // Retiring the token makes every guard over it a no-op.
            state.remove(member);
            state.tokens.remove(&member);
        }
        drop(state);
        self.state_snapshot().ok_or_else(Self::program_not_published)
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::Ipv4Addr;

    use overdrive_worker::mtls_intercept::InterceptLeg;

    use super::*;

    /// The canonical leg-F / leg-C bind address the production caller uses.
    const LEG_ADDR: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);

    /// A declared Service listener address — the `virt` shape `install_inbound`
    /// is called with.
    const VIRT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 8080);

    /// The leg-F target every install row converges before it installs; the
    /// outbound rows install with its port.
    const INSTALL_LEG_F: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4001);

    /// The leg-C target every install row converges before it installs; the
    /// inbound rows install with its port.
    const INSTALL_LEG_C: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4002);

    /// A guest source address the member rows install.
    const MEMBER_SOURCE: Ipv4Addr = Ipv4Addr::new(100, 95, 71, 2);

    /// Converge a fresh adapter at the install rows' targets, as the production
    /// caller does before any install (DISTILL gap B-8): the prior is the
    /// adapter's own observation. The caller holds the returned node guard for
    /// as long as it installs.
    fn converge_at_install_ports(sut: &SimMtlsIntercept) -> Box<dyn InterceptGuard> {
        let prior = sut.observe_shared().expect("the adapter observes its own program");
        sut.converge_shared(prior.as_ref(), INSTALL_LEG_F, INSTALL_LEG_C)
            .expect("a fresh adapter converges its program at non-zero targets")
    }

    /// The sim's substrate-neutral program identity for two targets, as its
    /// `converge_shared` records it (module docs, *Determinism*).
    fn sim_program(leg_f: SocketAddrV4, leg_c: SocketAddrV4) -> InterceptPostcondition {
        let encode = |addr: SocketAddrV4| {
            let mut bytes = Vec::from(addr.ip().octets());
            bytes.extend_from_slice(&addr.port().to_be_bytes());
            bytes
        };
        InterceptPostcondition::ConstantRules {
            table_and_chains: vec![b"sim-shared-table-and-chains".to_vec()],
            sets: vec![b"sim-shared-sets".to_vec()],
            prerouting: vec![encode(leg_f)],
            output: vec![encode(leg_c)],
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Convergence records both exact targets for a non-repairing observation,
    /// and the drop of its node guard, with no member present, removes the
    /// unchanged modeled program (D15's conditional delete, which the sim
    /// models under DISTILL gap B-8): every observation then reads `Ok(None)`.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn shared_convergence_records_both_exact_targets_for_non_repairing_observation() {
        let sut = SimMtlsIntercept::new();
        let leg_f = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 40_001);
        let leg_c = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 40_002);

        let guard = sut
            .converge_shared(None, leg_f, leg_c)
            .expect("healthy sim convergence publishes one shared identity");
        let observation = sut.observe_shared().expect("non-repairing observation succeeds");

        let Some(InterceptPostcondition::ConstantRules { prerouting, output, .. }) = observation
        else {
            panic!("shared convergence must publish the complete ConstantRules identity");
        };
        let encode = |addr: SocketAddrV4| {
            let mut bytes = Vec::from(addr.ip().octets());
            bytes.extend_from_slice(&addr.port().to_be_bytes());
            bytes
        };
        assert_eq!(prerouting, [encode(leg_f)]);
        assert_eq!(output, [encode(leg_c)]);
        drop(guard);
        assert_eq!(
            sut.observe_shared().expect("an observation after the drop succeeds"),
            None,
            "the dropped guard's identity equals the modeled program and no member exists, so \
             its drop removes the program"
        );
        assert_eq!(
            sut.observe_shared_state().expect("a state observation after the drop succeeds"),
            None,
            "the member-aware observation reads the removed program as absent"
        );
        assert_eq!(
            sut.converge_allocation_elements(&InterceptMembers::default())
                .expect("member convergence without a program succeeds"),
            None,
            "member convergence reads the removed program as absent and writes nothing"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A node guard dropped while a member exists keeps the program: its
    /// element guard is relinquished first (the guard-ordering rule's one
    /// sanctioned way to hold a member past a node-guard drop). The drop still
    /// withdraws the record, so a later install and a later removal are refused
    /// with `SharedProgramNotConverged` (DISTILL gap B-8).
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn a_node_guard_dropped_while_a_member_exists_keeps_the_program_and_withdraws_the_record() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        let member = sut
            .install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port())
            .expect("install one member against the recorded program");
        std::mem::forget(member);

        drop(node_guard);
        assert_eq!(
            sut.observe_shared().expect("an observation after the drop succeeds"),
            program,
            "a node guard dropped while a member exists removes nothing"
        );
        let state = sut
            .observe_shared_state()
            .expect("a state observation after the drop succeeds")
            .expect("the kept program is observable");
        assert_eq!(
            state.members.outbound_sources,
            BTreeSet::from([MEMBER_SOURCE]),
            "the relinquished member is still present"
        );
        assert!(
            matches!(
                sut.install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port()).map(|_guard| ()),
                Err(InterceptError::SharedProgramNotConverged)
            ),
            "the drop withdrew the record, so an install is refused"
        );
        assert!(
            matches!(
                sut.remove_allocation_elements(MEMBER_SOURCE, &[]),
                Err(InterceptError::SharedProgramNotConverged)
            ),
            "the drop withdrew the record, so a removal is refused"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A converge whose `prior` is the modeled program and whose targets equal
    /// it adopts the program: it succeeds with a member present (no replacement
    /// is attempted over it) and leaves the program and every member unchanged.
    #[test]
    fn converging_to_the_recorded_program_adopts_it_and_keeps_every_member() {
        let sut = SimMtlsIntercept::new();
        let first_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        let member = sut
            .install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port())
            .expect("install one member against the recorded program");
        let before = sut.observe_shared_state().expect("observe the state before the adopt");

        let adopted = sut
            .converge_shared(program.as_ref(), INSTALL_LEG_F, INSTALL_LEG_C)
            .expect("an equal identity is adopted, even with a member present");

        assert_eq!(sut.observe_shared().expect("observe after the adopt"), program);
        assert_eq!(
            sut.observe_shared_state().expect("observe the state after the adopt"),
            before,
            "an adopt changes no member"
        );
        drop(member);
        drop(adopted);
        drop(first_guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A program replacement is refused while a member exists
    /// (`NftSharedReplaceFailed` whose `prior` is the modeled program and whose
    /// source has the host's strict-observation shape), and it changes neither
    /// the program, the members, nor the record.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn a_program_replacement_is_refused_while_a_member_exists() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        let member = sut
            .install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port())
            .expect("install one member against the recorded program");
        let before = sut.observe_shared_state().expect("observe the state before");
        let retargeted_f = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5001);
        let retargeted_c = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5002);

        let refused = sut
            .converge_shared(program.as_ref(), retargeted_f, retargeted_c)
            .map(|_guard| ())
            .expect_err("a replacement over a live member is refused");
        match &refused {
            InterceptError::NftSharedReplaceFailed { prior, requested, source } => {
                assert_eq!(prior, &program, "the refusal names the modeled program as prior");
                assert_eq!(
                    requested,
                    &sim_program(retargeted_f, retargeted_c),
                    "the refusal names the requested identity"
                );
                assert!(
                    matches!(
                        source,
                        NetlinkError::Nft { op, source }
                            if *op == "shared-ip-observe"
                                && source.kind() == std::io::ErrorKind::InvalidData
                    ),
                    "the source has the host's strict-observation shape, got {source:?}"
                );
            }
            other => panic!("expected NftSharedReplaceFailed, got {other:?}"),
        }
        assert_eq!(sut.observe_shared().expect("observe after the refusal"), program);
        assert_eq!(sut.observe_shared_state().expect("observe the state after"), before);
        let still_recorded = sut
            .install_inbound(VIRT, INSTALL_LEG_C.port())
            .expect("the record is unchanged: an install at the recorded port succeeds");
        drop(still_recorded);
        drop(member);
        drop(node_guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A converge whose `prior` is not the modeled program is refused with
    /// `PostconditionMismatch { expected, observed }` — `expected` the caller's
    /// prior, or the requested identity when the prior is `None` — and changes
    /// nothing.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn a_convergence_from_a_stale_prior_is_refused_and_changes_nothing() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        let stale = sim_program(
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6001),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6002),
        );

        for (label, prior, expected) in [
            ("absent prior", None, sim_program(INSTALL_LEG_F, INSTALL_LEG_C)),
            ("different prior", Some(stale.clone()), stale),
        ] {
            let refused = sut
                .converge_shared(prior.as_ref(), INSTALL_LEG_F, INSTALL_LEG_C)
                .map(|_guard| ())
                .expect_err("a stale prior is refused");
            match refused {
                InterceptError::PostconditionMismatch { expected: got_expected, observed } => {
                    assert_eq!(got_expected, expected, "[{label}] the expected identity");
                    assert_eq!(observed, program, "[{label}] the observed identity");
                }
                other => panic!("[{label}] expected PostconditionMismatch, got {other:?}"),
            }
            assert_eq!(
                sut.observe_shared().expect("observe after the refusal"),
                program,
                "[{label}] the refusal changes nothing"
            );
        }
        drop(node_guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// `converge_shared` refuses in the host's order: a zero port before an
    /// armed fault, an armed fault before a stale prior, a stale prior before
    /// the replace-over-members refusal. Each refusal changes no modeled state.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn shared_convergence_refuses_in_the_hosts_order() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        let member =
            sut.install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port()).expect("install one member");
        let before = sut.observe_shared_state().expect("observe the state before");
        let retargeted_f = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5001);
        let retargeted_c = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5002);
        sut.script_converge_shared_fault(SimInterceptFault::NftRuleInstall {
            op: "observe-shared",
            errno: libc::EIO,
        });

        // 1. A zero port precedes the armed fault.
        let zero = sut
            .converge_shared(program.as_ref(), LEG_ADDR, retargeted_c)
            .map(|_guard| ())
            .expect_err("a zero port is refused");
        assert!(
            matches!(
                &zero,
                InterceptError::NftRuleInstallFailed { op: "shared-ip-expected", source }
                    if matches!(
                        source,
                        NetlinkError::Nft { op, source }
                            if *op == "shared-ip-identity"
                                && source.kind() == std::io::ErrorKind::InvalidData
                    )
            ),
            "a zero port refuses before the armed fault, got {zero:?}"
        );

        // 2. The armed fault precedes a stale prior.
        let faulted = sut
            .converge_shared(None, retargeted_f, retargeted_c)
            .map(|_guard| ())
            .expect_err("the armed fault fires");
        assert_err_shape(
            &faulted,
            ExpectedErr::NftRuleInstall { op: "observe-shared", errno: libc::EIO },
        );

        // 3. A stale prior precedes the replace-over-members refusal.
        sut.clear_faults();
        let stale = sut
            .converge_shared(None, retargeted_f, retargeted_c)
            .map(|_guard| ())
            .expect_err("a stale prior is refused");
        assert!(
            matches!(stale, InterceptError::PostconditionMismatch { .. }),
            "a stale prior refuses before the replace-over-members refusal, got {stale:?}"
        );

        assert_eq!(sut.observe_shared().expect("observe after the refusals"), program);
        assert_eq!(sut.observe_shared_state().expect("observe the state after"), before);
        drop(member);
        drop(node_guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The two standing shared slots are independent: the converge fault
    /// stands for the host's one failure before any write (the observation,
    /// `op: "observe-shared"`), fires on every call while armed, and never
    /// leaks into the observe slot; `clear_faults` disarms both.
    #[test]
    fn shared_converge_and_observe_faults_are_independent_standing_slots() {
        let sut = SimMtlsIntercept::new();
        let converge_fault =
            SimInterceptFault::NftRuleInstall { op: "observe-shared", errno: libc::EBUSY };
        let observe_fault =
            SimInterceptFault::NftRuleInstall { op: "observe-shared", errno: libc::EIO };
        sut.script_converge_shared_fault(converge_fault);
        sut.script_observe_shared_fault(observe_fault);

        for _ in 0..2 {
            let converge = sut
                .converge_shared(None, INSTALL_LEG_F, INSTALL_LEG_C)
                .map(|_guard| ())
                .expect_err("the armed converge fault fires");
            assert_err_shape(
                &converge,
                ExpectedErr::NftRuleInstall { op: "observe-shared", errno: libc::EBUSY },
            );
            let observe = sut.observe_shared().expect_err("the armed observe fault fires");
            assert_err_shape(
                &observe,
                ExpectedErr::NftRuleInstall { op: "observe-shared", errno: libc::EIO },
            );
        }
        sut.clear_faults();
        let guard = sut
            .converge_shared(None, INSTALL_LEG_F, INSTALL_LEG_C)
            .expect("clear_faults disarms shared convergence");
        assert!(sut.observe_shared().expect("shared observation is healthy").is_some());
        drop(guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A `converge_shared` that fails leaves the record as it was (DISTILL gap
    /// B-8, the failed-converge clause): after a converge, an armed converge
    /// fault's `Err` changes neither the program nor the record, so an install
    /// at the recorded port still succeeds and a removal is not refused for
    /// want of a record.
    #[test]
    fn a_failed_convergence_keeps_the_recorded_program() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        let program = sut.observe_shared().expect("observe the converged program");
        sut.script_converge_shared_fault(SimInterceptFault::NftRuleInstall {
            op: "observe-shared",
            errno: libc::EIO,
        });

        let failed = sut
            .converge_shared(program.as_ref(), INSTALL_LEG_F, INSTALL_LEG_C)
            .map(|_guard| ())
            .expect_err("the armed converge fault fires");
        assert_err_shape(
            &failed,
            ExpectedErr::NftRuleInstall { op: "observe-shared", errno: libc::EIO },
        );

        assert_eq!(
            sut.observe_shared().expect("observe after the failed converge"),
            program,
            "a failed converge changes no program"
        );
        let member = sut
            .install_outbound(MEMBER_SOURCE, INSTALL_LEG_F.port())
            .expect("an install at the recorded port succeeds after a failed converge");
        drop(member);
        let removed = sut
            .remove_allocation_elements(MEMBER_SOURCE, &[])
            .expect("a removal is not refused for want of a record after a failed converge");
        assert_eq!(removed.program, program.expect("the program is present"));
        drop(node_guard);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-71 — Protection is installed only against the node's own converged program.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// The install partition is checked in order (DISTILL gap B-8): with an
    /// install fault armed, a call before any converge is refused with
    /// `SharedProgramNotConverged`; after a converge, a call at the other
    /// leg's port is refused with `SharedListenerPortMismatch { leg, expected,
    /// actual }`; only a call at the recorded port reaches the armed fault.
    /// No refusal records a member.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-71)"]
    fn an_armed_install_fault_fires_only_after_the_record_and_port_checks() {
        for (method, fault, leg, recorded, crossed, expected) in [
            (
                Method::InstallOutbound,
                SimInterceptFault::NftElementUpdate {
                    operation: InterceptElementOperation::Insert,
                    errno: libc::EIO,
                },
                InterceptLeg::F,
                INSTALL_LEG_F.port(),
                INSTALL_LEG_C.port(),
                ExpectedErr::ElementUpdate {
                    set: InterceptSet::ManagedGuestIps,
                    operation: InterceptElementOperation::Insert,
                    key: ExpectedKey::Address(Ipv4Addr::LOCALHOST),
                    errno: libc::EIO,
                },
            ),
            (
                Method::InstallInbound,
                SimInterceptFault::NftElementUpdate {
                    operation: InterceptElementOperation::ReadBack,
                    errno: libc::EIO,
                },
                InterceptLeg::C,
                INSTALL_LEG_C.port(),
                INSTALL_LEG_F.port(),
                ExpectedErr::ElementUpdate {
                    set: InterceptSet::InboundDestinations,
                    operation: InterceptElementOperation::ReadBack,
                    key: ExpectedKey::Destination(VIRT),
                    errno: libc::EIO,
                },
            ),
        ] {
            let sut = SimMtlsIntercept::new();
            arm(&sut, method, fault);

            let unconverged = drive_expecting_err(&sut, method);
            assert!(
                matches!(unconverged, InterceptError::SharedProgramNotConverged),
                "{method:?}: before any converge the precondition refuses first, got \
                 {unconverged:?}"
            );

            let node_guard = converge_at_install_ports(&sut);
            let mismatched = match method {
                Method::InstallOutbound => {
                    sut.install_outbound(Ipv4Addr::LOCALHOST, crossed).map(|_guard| ())
                }
                Method::InstallInbound => sut.install_inbound(VIRT, crossed).map(|_guard| ()),
                Method::BindTransparent => unreachable!("only the two installs are driven"),
            }
            .expect_err("a port other than the recorded one is refused");
            match mismatched {
                InterceptError::SharedListenerPortMismatch {
                    leg: got_leg,
                    expected: got_expected,
                    actual,
                } => {
                    assert_eq!(got_leg, leg, "{method:?}: the mismatch names the leg");
                    assert_eq!(got_expected, recorded, "{method:?}: expected is the recorded port");
                    assert_eq!(actual, crossed, "{method:?}: actual is the passed port");
                }
                other => panic!(
                    "{method:?}: the port check refuses before the armed fault, got {other:?}"
                ),
            }

            assert_err_shape(&drive_expecting_err(&sut, method), expected);
            assert_eq!(
                sut.observe_shared_state()
                    .expect("observe the state after the refusals")
                    .expect("the converged program is present")
                    .members,
                InterceptMembers::default(),
                "{method:?}: no refusal records a member"
            );
            drop(node_guard);
        }
    }

    /// Which trait method a S-MIF-06 case drives.
    #[derive(Debug, Clone, Copy)]
    enum Method {
        BindTransparent,
        InstallOutbound,
        InstallInbound,
    }

    /// The element key an [`ExpectedErr::ElementUpdate`] names (a `Copy`
    /// mirror of `InterceptElementKey`).
    #[derive(Debug, Clone, Copy)]
    enum ExpectedKey {
        Address(Ipv4Addr),
        Destination(SocketAddrV4),
    }

    /// The `Err` payload a S-MIF-06 case expects, in the shape the REAL
    /// substrate reports it.
    #[derive(Debug, Clone, Copy)]
    enum ExpectedErr {
        /// `InterceptError::TransparentListener` carrying exactly this `addr`
        /// and whose `source.raw_os_error()` is exactly this `errno`.
        ///
        /// `addr` is asserted, not ignored: ADR-0076 § 4.6 pins it as "the
        /// address the faulted call was made with". Dropping the assertion
        /// would let `materialise`'s address argument be swapped for any other
        /// value undetected.
        TransparentListener { addr: SocketAddrV4, errno: i32 },
        /// `InterceptError::NftRuleInstallFailed` carrying exactly this `op`,
        /// and whose `source.errno()` is exactly `-errno` (the netlink
        /// negative-errno convention the `NetlinkError::Nft` shape re-negates
        /// the positive kernel `errno` into).
        NftRuleInstall { op: &'static str, errno: i32 },
        /// `InterceptError::NftElementUpdateFailed` carrying exactly this set,
        /// operation, and key, and whose `source.errno()` is exactly `-errno`
        /// (refusal 3 of the pinned install partition, DISTILL gap B-8).
        ElementUpdate {
            set: InterceptSet,
            operation: InterceptElementOperation,
            key: ExpectedKey,
            errno: i32,
        },
    }

    /// The 3 SANCTIONED S-MIF-06 pairings — `(method, armed fault, expected
    /// error)` — one per scriptable method, each in the shape the pinned
    /// partition of that method contains (DISTILL gap B-8: an install's only
    /// scriptable refusal is the element update). The UNSANCTIONED pairings
    /// (an install fault on `bind_transparent`, a `TransparentListener` fault
    /// on an install) model a substrate the real one cannot exhibit and are a
    /// documented test defect, so they are deliberately absent.
    fn sanctioned_pairings() -> [(Method, SimInterceptFault, ExpectedErr); 3] {
        [
            (
                Method::BindTransparent,
                SimInterceptFault::TransparentListener { errno: libc::EPERM },
                ExpectedErr::TransparentListener { addr: LEG_ADDR, errno: libc::EPERM },
            ),
            // Missing `CAP_NET_ADMIN` on the element insert; ruleset lock
            // contention on the inbound read-back — the real errno shapes the
            // hand-rolled nft `NETLINK_NETFILTER` path returns.
            (
                Method::InstallOutbound,
                SimInterceptFault::NftElementUpdate {
                    operation: InterceptElementOperation::Insert,
                    errno: libc::EPERM,
                },
                ExpectedErr::ElementUpdate {
                    set: InterceptSet::ManagedGuestIps,
                    operation: InterceptElementOperation::Insert,
                    key: ExpectedKey::Address(Ipv4Addr::LOCALHOST),
                    errno: libc::EPERM,
                },
            ),
            (
                Method::InstallInbound,
                SimInterceptFault::NftElementUpdate {
                    operation: InterceptElementOperation::ReadBack,
                    errno: libc::EBUSY,
                },
                ExpectedErr::ElementUpdate {
                    set: InterceptSet::InboundDestinations,
                    operation: InterceptElementOperation::ReadBack,
                    key: ExpectedKey::Destination(VIRT),
                    errno: libc::EBUSY,
                },
            ),
        ]
    }

    /// Drive `method` on `sut` and return the `Err` it must produce. Every
    /// caller here has a fault armed, so an `Ok` is a test failure — and for
    /// `bind_transparent` an `Ok` would additionally bind a real socket, which
    /// is exactly the default-lane I/O this suite must not perform. The
    /// installs use the recorded ports of [`converge_at_install_ports`].
    fn drive_expecting_err(sut: &SimMtlsIntercept, method: Method) -> InterceptError {
        match method {
            // The `Ok` payload is mapped away before `expect_err` so this
            // compiles on both sides of the B-7 step: from that step on it is
            // an `Arc<dyn InterceptListener>`, which is not `Debug`.
            Method::BindTransparent => sut
                .bind_transparent(LEG_ADDR)
                .map(|_listener| ())
                .expect_err("an armed bind fault short-circuits before any syscall"),
            // `Box<dyn InterceptGuard>` is not `Debug` (the guard's entire
            // contract is its `Drop`), so the `Ok` payload is mapped away
            // before `expect_err`.
            Method::InstallOutbound => sut
                .install_outbound(Ipv4Addr::LOCALHOST, INSTALL_LEG_F.port())
                .map(|_guard| ())
                .expect_err("an armed outbound fault short-circuits"),
            Method::InstallInbound => sut
                .install_inbound(VIRT, INSTALL_LEG_C.port())
                .map(|_guard| ())
                .expect_err("an armed inbound fault short-circuits"),
        }
    }

    /// Arm `fault` on the slot backing `method`.
    fn arm(sut: &SimMtlsIntercept, method: Method, fault: SimInterceptFault) {
        match method {
            Method::BindTransparent => sut.script_bind_fault(fault),
            Method::InstallOutbound => sut.script_outbound_fault(fault),
            Method::InstallInbound => sut.script_inbound_fault(fault),
        }
    }

    /// Assert `got` is exactly the [`ExpectedErr`] shape — the `Err`
    /// discriminant AND its payload.
    fn assert_err_shape(got: &InterceptError, expected: ExpectedErr) {
        match expected {
            ExpectedErr::TransparentListener { addr, errno } => match got {
                InterceptError::TransparentListener { addr: got_addr, source } => {
                    assert_eq!(
                        *got_addr, addr,
                        "TransparentListener must carry the address the faulted call was made with",
                    );
                    assert_eq!(
                        source.raw_os_error(),
                        Some(errno),
                        "TransparentListener must carry the armed errno",
                    );
                }
                other => panic!("expected TransparentListener, got {other:?}"),
            },
            ExpectedErr::NftRuleInstall { op, errno } => match got {
                InterceptError::NftRuleInstallFailed { op: got_op, source } => {
                    assert_eq!(*got_op, op, "NftRuleInstallFailed must carry the armed nft op");
                    assert_eq!(
                        source.errno(),
                        Some(-errno.abs()),
                        "NftRuleInstallFailed source must carry the armed errno (netlink -errno convention)",
                    );
                }
                other => panic!("expected NftRuleInstallFailed, got {other:?}"),
            },
            ExpectedErr::ElementUpdate { set, operation, key, errno } => match got {
                InterceptError::NftElementUpdateFailed {
                    set: got_set,
                    operation: got_operation,
                    key: got_key,
                    source,
                } => {
                    assert_eq!(*got_set, set, "NftElementUpdateFailed must name the call's set");
                    assert_eq!(
                        *got_operation, operation,
                        "NftElementUpdateFailed must carry the armed operation"
                    );
                    let expected_key = match key {
                        ExpectedKey::Address(address) => InterceptElementKey::Address(address),
                        ExpectedKey::Destination(destination) => {
                            InterceptElementKey::Destination(destination)
                        }
                    };
                    assert_eq!(
                        *got_key, expected_key,
                        "NftElementUpdateFailed must name the call's key"
                    );
                    assert_eq!(
                        source.errno(),
                        Some(-errno.abs()),
                        "NftElementUpdateFailed source must carry the armed errno (netlink -errno convention)",
                    );
                }
                other => panic!("expected NftElementUpdateFailed, got {other:?}"),
            },
        }
    }

    /// S-MIF-06 — an armed fault surfaces as exactly the error the REAL
    /// substrate produces, across the 3 sanctioned pairings.
    ///
    /// Universe (port-exposed): the `Result` the trait method returns — its
    /// `Err` discriminant, plus `addr` and `source.raw_os_error()` for
    /// `TransparentListener`, and the set, operation, key, and
    /// `source.errno()` for `NftElementUpdateFailed`. Nothing reads a private
    /// slot; the scripting helpers are the only writes and the trait method the
    /// only read. Each case converges first, as the production caller does, so
    /// an install reaches its armed fault rather than the B-8 precondition.
    ///
    /// Realism criterion (research Finding 5.3, DFS-4): the faults are armed in
    /// the REAL shapes the substrate produces — `libc::EPERM` is the
    /// missing-`CAP_NET_ADMIN` shape — never a generic "fail now".
    #[test]
    fn armed_fault_surfaces_as_the_real_substrate_error() {
        for (method, fault, expected) in sanctioned_pairings() {
            let sut = SimMtlsIntercept::new();
            let node_guard = converge_at_install_ports(&sut);
            arm(&sut, method, fault);

            let got = drive_expecting_err(&sut, method);

            assert_err_shape(&got, expected);
            drop(node_guard);
        }
    }

    /// S-MIF-07 — an armed fault is STANDING (DFS-4): it fires on BOTH of two
    /// consecutive calls with the same cause, and does not decay.
    ///
    /// Universe: the two `Result`s from the two consecutive `bind_transparent`
    /// calls — both `Err`, both `TransparentListener`, both carrying the bind
    /// `addr` and the armed `raw_os_error()`.
    ///
    /// Two calls, not `n`: two is the minimum that distinguishes standing from
    /// one-shot, and it is the exact cardinality the production caller exhibits
    /// (`start_alloc` binds leg-F then leg-C). A `.take()`-instead-of-clone
    /// regression makes the SECOND call take the `Ok` arm — which for
    /// `bind_transparent` would additionally drag a real socket bind into the
    /// default lane.
    #[test]
    fn armed_fault_is_standing_and_fires_on_every_call() {
        let sut = SimMtlsIntercept::new();
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EADDRINUSE });

        let first = drive_expecting_err(&sut, Method::BindTransparent);
        let second = drive_expecting_err(&sut, Method::BindTransparent);

        let expected = ExpectedErr::TransparentListener { addr: LEG_ADDR, errno: libc::EADDRINUSE };
        assert_err_shape(&first, expected);
        assert_err_shape(&second, expected);
    }

    /// S-MIF-08 — `clear_faults` disarms ALL three slots, and clearing an
    /// already-disarmed double is a benign no-op.
    ///
    /// Universe: the `Result`s from `install_outbound` / `install_inbound`
    /// after the first `clear_faults()` (both `Ok`, each handing back a guard),
    /// and the same two after a SECOND `clear_faults()` (still `Ok`).
    ///
    /// `bind_transparent` is not re-driven here. That `clear_faults` also
    /// disarms the BIND slot is asserted by
    /// [`clear_faults_also_disarms_the_bind_slot`] (S-ND295-70), which
    /// re-drives `bind_transparent` after the clear and reads back the
    /// socket-free listener its `Ok` arm returns. That body stays pending
    /// until the DELIVER step that carries B-7 (05-01) makes the `Ok` arm
    /// socket-free; until then the `Ok` arm binds a real plain loopback
    /// socket (DFS-5), the body is ignored, and deleting `clear_faults`'s
    /// `*self.bind_fault.lock() = None;` line survives the active suite.
    ///
    /// The second `clear_faults()` is the fault state machine's
    /// illegal-event-from-the-disarmed-state case (C2b), asserted as a benign
    /// no-op rather than a panic or a state flip.
    #[test]
    fn clear_faults_disarms_every_slot_and_is_idempotent() {
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
        sut.script_outbound_fault(SimInterceptFault::NftElementUpdate {
            operation: InterceptElementOperation::Insert,
            errno: libc::EPERM,
        });
        sut.script_inbound_fault(SimInterceptFault::NftElementUpdate {
            operation: InterceptElementOperation::ReadBack,
            errno: libc::EPERM,
        });

        sut.clear_faults();
        sut.install_outbound(Ipv4Addr::LOCALHOST, INSTALL_LEG_F.port())
            .expect("clear_faults disarms the outbound slot");
        sut.install_inbound(VIRT, INSTALL_LEG_C.port())
            .expect("clear_faults disarms the inbound slot");

        sut.clear_faults();
        sut.install_outbound(Ipv4Addr::LOCALHOST, INSTALL_LEG_F.port())
            .expect("a second clear_faults leaves the outbound slot disarmed");
        sut.install_inbound(VIRT, INSTALL_LEG_C.port())
            .expect("a second clear_faults leaves the inbound slot disarmed");
        drop(node_guard);
    }

    /// S-MIF-13 — the three fault slots are INDEPENDENT: arming exactly one
    /// leaves the others on their success arms, across the three I/O-free
    /// directions.
    ///
    /// | Armed slot | `install_outbound` | `install_inbound` |
    /// |---|---|---|
    /// | `bind_fault` | `Ok(guard)` | `Ok(guard)` |
    /// | `outbound_fault` | `Err` | `Ok(guard)` |
    /// | `inbound_fault` | `Ok(guard)` | `Err` |
    ///
    /// Universe: the two `Result`s from `install_outbound` / `install_inbound`
    /// per direction.
    ///
    /// This is the ONLY scenario separating the slots. An implementation
    /// sharing one slot across all three methods, or a copy-paste bug pointing
    /// two scripting helpers at the same field, would still pass
    /// S-MIF-06/07/08 — each of those arms exactly one slot and reads back the
    /// same method.
    ///
    /// The FOURTH direction — arm an install fault, confirm
    /// `bind_transparent` still takes its `Ok` arm — is
    /// [`an_install_fault_leaves_bind_on_its_success_arm`] (S-ND295-70). It
    /// stays pending until the DELIVER step that carries B-7 (05-01) makes
    /// that `Ok` arm socket-free; before that step the arm binds a real socket
    /// (DFS-5), so the direction is not covered by the active suite.
    #[test]
    fn arming_one_slot_leaves_the_others_on_their_success_arms() {
        let outbound_fault = SimInterceptFault::NftElementUpdate {
            operation: InterceptElementOperation::Insert,
            errno: libc::EPERM,
        };
        let inbound_fault = SimInterceptFault::NftElementUpdate {
            operation: InterceptElementOperation::ReadBack,
            errno: libc::EPERM,
        };

        // Direction 1 — arming the BIND slot leaks to neither install.
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
        sut.install_outbound(Ipv4Addr::LOCALHOST, INSTALL_LEG_F.port())
            .expect("a bind fault does not leak into install_outbound");
        sut.install_inbound(VIRT, INSTALL_LEG_C.port())
            .expect("a bind fault does not leak into install_inbound");
        drop(node_guard);

        // Direction 2 — arming the OUTBOUND slot refuses only `install_outbound`.
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        sut.script_outbound_fault(outbound_fault);
        let got = drive_expecting_err(&sut, Method::InstallOutbound);
        assert_err_shape(
            &got,
            ExpectedErr::ElementUpdate {
                set: InterceptSet::ManagedGuestIps,
                operation: InterceptElementOperation::Insert,
                key: ExpectedKey::Address(Ipv4Addr::LOCALHOST),
                errno: libc::EPERM,
            },
        );
        sut.install_inbound(VIRT, INSTALL_LEG_C.port())
            .expect("an outbound fault does not leak into install_inbound");
        drop(node_guard);

        // Direction 3 — arming the INBOUND slot refuses only `install_inbound`.
        let sut = SimMtlsIntercept::new();
        let node_guard = converge_at_install_ports(&sut);
        sut.script_inbound_fault(inbound_fault);
        sut.install_outbound(Ipv4Addr::LOCALHOST, INSTALL_LEG_F.port())
            .expect("an inbound fault does not leak into install_outbound");
        let got = drive_expecting_err(&sut, Method::InstallInbound);
        assert_err_shape(
            &got,
            ExpectedErr::ElementUpdate {
                set: InterceptSet::InboundDestinations,
                operation: InterceptElementOperation::ReadBack,
                key: ExpectedKey::Destination(VIRT),
                errno: libc::EPERM,
            },
        );
        drop(node_guard);
    }

    // -----------------------------------------------------------------------
    // S-ND295-70 — the socket-free listener surface (B-7, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned contract through the `SimMtlsIntercept` contract)).
    //
    // Until the DELIVER step that carries B-7 (05-01), `bind_transparent`'s
    // `Ok` arm returns a real `std::net::TcpListener` and registers nothing,
    // so every body below is pending that step. The bodies reach the bound
    // listener only through `LegListener`, so they compile on both sides of
    // it and that step edits none of them.
    // -----------------------------------------------------------------------

    /// Every bounded wait in these bodies: a body that is RED fails instead
    /// of hanging.
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);

    /// A loopback address at `port`.
    const fn loopback(port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
    }

    /// The sim-local copy of the `LegListener` bridge (TS § *Intercept
    /// listener and stop-error test support*). `bind_transparent`'s `Ok`
    /// value is a real `std::net::TcpListener` before the B-7 step and an
    /// `Arc<dyn InterceptListener>` from then on. The `TcpListener`
    /// implementation is deleted at that step (its test-support line 4).
    trait LegListener {
        /// The bound IPv4 address.
        fn bound_v4(&self) -> std::io::Result<SocketAddrV4>;
        /// A further holder of the socket-free port listener, or `None` when
        /// the bind produced a real socket.
        fn port_listener(&self) -> Option<Arc<dyn InterceptListener>>;
    }

    impl LegListener for std::net::TcpListener {
        fn bound_v4(&self) -> std::io::Result<SocketAddrV4> {
            match self.local_addr()? {
                std::net::SocketAddr::V4(bound) => Ok(bound),
                std::net::SocketAddr::V6(bound) => {
                    Err(std::io::Error::other(format!("real listener bound IPv6 {bound}")))
                }
            }
        }

        fn port_listener(&self) -> Option<Arc<dyn InterceptListener>> {
            None
        }
    }

    impl LegListener for Arc<dyn InterceptListener> {
        fn bound_v4(&self) -> std::io::Result<SocketAddrV4> {
            self.local_addr()
        }

        fn port_listener(&self) -> Option<Arc<dyn InterceptListener>> {
            Some(Self::clone(self))
        }
    }

    /// Bind through the port; return the holder and its bound address.
    fn bind_live(sut: &SimMtlsIntercept, addr: SocketAddrV4) -> (impl LegListener, SocketAddrV4) {
        let held = sut
            .bind_transparent(addr)
            .unwrap_or_else(|error| panic!("bind at {addr} must succeed: {error:?}"));
        let bound = held.bound_v4().expect("a fresh listener reads its bound address");
        (held, bound)
    }

    /// A further holder of the socket-free listener `held` holds.
    fn port_of(held: &impl LegListener) -> Arc<dyn InterceptListener> {
        held.port_listener().expect(
            "bind_transparent's Ok arm returns the socket-free sim listener, not a real socket",
        )
    }

    /// The error a bind that must be refused returns.
    fn bind_refusal(sut: &SimMtlsIntercept, addr: SocketAddrV4) -> InterceptError {
        match sut.bind_transparent(addr) {
            Ok(_listener) => panic!("a bind at {addr} must be refused while a listener holds it"),
            Err(error) => error,
        }
    }

    /// `error` is the held-address refusal for `addr`.
    fn assert_eaddrinuse(error: &InterceptError, addr: SocketAddrV4) {
        assert!(
            matches!(
                error,
                InterceptError::TransparentListener { addr: refused, source }
                    if *refused == addr && source.raw_os_error() == Some(libc::EADDRINUSE)
            ),
            "a held address is refused with TransparentListener {{ {addr}, EADDRINUSE }}, got \
             {error:?}",
        );
    }

    /// A scripted accepted connection. An anonymous pipe stands in for the
    /// stream: the sim neither reads nor writes it.
    fn accepted(peer: SocketAddrV4, local: SocketAddrV4) -> InterceptAccepted {
        let (reader, _writer) =
            std::io::pipe().expect("an anonymous pipe stands in for the accepted stream");
        InterceptAccepted { stream: std::os::fd::OwnedFd::from(reader), peer, local }
    }

    /// Poll `future` exactly once in the current task's context.
    async fn poll_once<F: Future + Unpin + Send>(future: &mut F) -> std::task::Poll<F::Output> {
        std::future::poll_fn(|cx| std::task::Poll::Ready(std::pin::Pin::new(&mut *future).poll(cx)))
            .await
    }

    /// The `(peer, local)` of an accept that must return a connection.
    fn connection_of(
        outcome: std::result::Result<InterceptAccepted, InterceptAcceptError>,
    ) -> (SocketAddrV4, SocketAddrV4) {
        let accepted = outcome.expect("the accept returns the scripted connection");
        (accepted.peer, accepted.local)
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A port-0 bind takes the smallest port ≥ 49152 that no live listener of
    /// the adapter holds at the requested IP; an exact non-zero address is
    /// honoured; a released port is handed out again; a fresh adapter repeats
    /// the same sequence (no clock, no entropy).
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    fn fabricated_ports_are_deterministic_non_zero_and_distinct_among_live_listeners() {
        let sut = SimMtlsIntercept::new();
        let (first, first_at) = bind_live(&sut, LEG_ADDR);
        let (_second, second_at) = bind_live(&sut, LEG_ADDR);
        assert_eq!(
            (first_at, second_at),
            (loopback(49_152), loopback(49_153)),
            "port 0 takes 49152, then 49153 while the first is held",
        );

        let (_exact, exact_at) = bind_live(&sut, loopback(49_154));
        assert_eq!(exact_at, loopback(49_154), "an exact non-zero address is honoured");
        let (_third, third_at) = bind_live(&sut, LEG_ADDR);
        assert_eq!(third_at, loopback(49_155), "port 0 skips an exactly-bound live port");

        let other_ip = Ipv4Addr::new(10, 0, 0, 1);
        let (_remote, remote_at) = bind_live(&sut, SocketAddrV4::new(other_ip, 0));
        assert_eq!(
            remote_at,
            SocketAddrV4::new(other_ip, 49_152),
            "port 0 binds at the requested IP, whose port space is its own",
        );
        assert_eq!(
            sut.live_listeners(),
            [remote_at, loopback(49_152), loopback(49_153), loopback(49_154), loopback(49_155)],
            "live_listeners lists every live address, ascending",
        );

        drop(first);
        let (_reused, reused_at) = bind_live(&sut, LEG_ADDR);
        assert_eq!(reused_at, loopback(49_152), "a released port is the smallest free port again");

        let replay = SimMtlsIntercept::new();
        let (_replay_first, replay_first_at) = bind_live(&replay, LEG_ADDR);
        let (_replay_second, replay_second_at) = bind_live(&replay, LEG_ADDR);
        assert_eq!(
            (replay_first_at, replay_second_at),
            (loopback(49_152), loopback(49_153)),
            "a fresh adapter fabricates the same ports",
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// A held address is refused with `TransparentListener { addr,
    /// EADDRINUSE }` while any `Arc` holds its listener; the refusal acquires
    /// nothing; the address binds again once the last holder drops. The
    /// refusal is per address, not per port.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    fn a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops() {
        let sut = SimMtlsIntercept::new();
        let (held, at) = bind_live(&sut, LEG_ADDR);
        let second_holder = port_of(&held);

        assert_eaddrinuse(&bind_refusal(&sut, at), at);
        assert_eq!(sut.live_listeners(), [at], "a refused bind acquires nothing");

        let same_port_other_ip = SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), at.port());
        let (_other, other_at) = bind_live(&sut, same_port_other_ip);
        assert_eq!(other_at, same_port_other_ip, "the same port at another IP is a free address");

        drop(held);
        assert_eq!(
            sut.live_listeners(),
            [other_at, at],
            "the second holder keeps the listener live"
        );
        assert_eaddrinuse(&bind_refusal(&sut, at), at);

        drop(second_holder);
        assert_eq!(sut.live_listeners(), [other_at], "the last holder's drop releases the address");
        let (_rebound, rebound_at) = bind_live(&sut, at);
        assert_eq!(rebound_at, at, "a released exact address binds again");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// With nothing scripted, `accept` stays pending and is counted parked
    /// once however often it is polled; dropping it un-parks it and takes
    /// nothing — neither a connection scripted after the drop nor one
    /// scripted while it was parked but not yet re-polled.
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    async fn an_unscripted_accept_stays_parked_and_a_cancelled_accept_takes_nothing() {
        let sut = SimMtlsIntercept::new();
        let (held, at) = bind_live(&sut, LEG_ADDR);
        let listener = port_of(&held);
        let peer = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 40_000);
        let local = SocketAddrV4::new(Ipv4Addr::new(10, 99, 1, 7), 443);
        assert_eq!(sut.parked_accepts(at), 0, "no accept has been polled");

        let mut pending = listener.accept();
        assert!(poll_once(&mut pending).await.is_pending(), "nothing scripted: the accept waits");
        assert_eq!(sut.parked_accepts(at), 1);
        tokio::task::yield_now().await;
        assert!(poll_once(&mut pending).await.is_pending(), "it waits with no timeout");
        assert_eq!(sut.parked_accepts(at), 1, "a re-polled accept is still one parked accept");
        drop(pending);
        assert_eq!(sut.parked_accepts(at), 0, "dropping a pending accept un-parks it");

        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(peer, local))));
        let next = tokio::time::timeout(WAIT, listener.accept())
            .await
            .expect("a scripted connection completes the next accept");
        assert_eq!(connection_of(next), (peer, local), "the cancelled accept took nothing");

        let mut woken = listener.accept();
        assert!(poll_once(&mut woken).await.is_pending());
        let second_peer = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 40_001);
        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(second_peer, local))));
        drop(woken);
        assert_eq!(sut.parked_accepts(at), 0);
        let kept = tokio::time::timeout(WAIT, listener.accept())
            .await
            .expect("the connection a dropped accept was woken for stays for the next call");
        assert_eq!(connection_of(kept), (second_peer, local));
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// One row per `SimAcceptScript` outcome: each completes exactly the
    /// accept it names. `Connection` and `OriginalDestinationFailure` leave
    /// the listener usable (the next accept waits, and a script wakes it);
    /// `ListenerLost` stands for every later accept. Scripts are FIFO, a
    /// `local_addr` failure is standing and leaves `accept` alone, and scripts
    /// die with their listener.
    #[tokio::test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    #[allow(clippy::too_many_lines, reason = "one table plus the FIFO and lifetime rules")]
    async fn each_scripted_outcome_completes_exactly_the_accept_it_names() {
        #[derive(Debug, Clone, Copy)]
        enum Row {
            Connection,
            OriginalDestinationFailure,
            ListenerLost,
        }
        let peer = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 40_000);
        let local = SocketAddrV4::new(Ipv4Addr::new(10, 99, 1, 7), 443);
        let later_peer = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 40_001);

        for row in [Row::Connection, Row::OriginalDestinationFailure, Row::ListenerLost] {
            let sut = SimMtlsIntercept::new();
            let (held, at) = bind_live(&sut, LEG_ADDR);
            let listener = port_of(&held);
            let script = match row {
                Row::Connection => SimAcceptScript::Connection(accepted(peer, local)),
                Row::OriginalDestinationFailure => {
                    SimAcceptScript::OriginalDestinationFailure { errno: libc::ENOTCONN }
                }
                Row::ListenerLost => SimAcceptScript::ListenerLost { errno: libc::ECONNABORTED },
            };
            assert!(sut.script_accept(at, script), "{row:?}: a live listener takes the script");
            let first = tokio::time::timeout(WAIT, listener.accept())
                .await
                .unwrap_or_else(|_| panic!("{row:?}: the scripted outcome completes the accept"));
            match (row, first) {
                (Row::Connection, Ok(got)) => assert_eq!((got.peer, got.local), (peer, local)),
                (
                    Row::OriginalDestinationFailure,
                    Err(InterceptAcceptError::OriginalDestination { source }),
                ) => assert_eq!(source.raw_os_error(), Some(libc::ENOTCONN)),
                (Row::ListenerLost, Err(InterceptAcceptError::Accept { source })) => {
                    assert_eq!(source.raw_os_error(), Some(libc::ECONNABORTED));
                }
                (row, other) => panic!("{row:?}: wrong outcome {other:?}"),
            }

            // The next accepts, with a later connection scripted.
            if matches!(row, Row::ListenerLost) {
                assert!(
                    sut.script_accept(at, SimAcceptScript::Connection(accepted(later_peer, local)))
                );
                for later in 0..2 {
                    let mut next = listener.accept();
                    let lost = poll_once(&mut next).await;
                    assert!(
                        matches!(
                            &lost,
                            std::task::Poll::Ready(Err(InterceptAcceptError::Accept { source }))
                                if source.raw_os_error() == Some(libc::ECONNABORTED)
                        ),
                        "ListenerLost stands for later accept {later}, got {lost:?}",
                    );
                }
                assert_eq!(sut.parked_accepts(at), 0, "a lost listener parks nothing");
            } else {
                let mut next = listener.accept();
                assert!(poll_once(&mut next).await.is_pending(), "{row:?}: the listener is usable");
                assert_eq!(sut.parked_accepts(at), 1);
                assert!(
                    sut.script_accept(at, SimAcceptScript::Connection(accepted(later_peer, local)))
                );
                let woken = tokio::time::timeout(WAIT, next)
                    .await
                    .unwrap_or_else(|_| panic!("{row:?}: a script wakes the parked accept"));
                assert_eq!(connection_of(woken), (later_peer, local), "{row:?}");
            }
        }

        // FIFO: two scripts complete two accepts in script order.
        let sut = SimMtlsIntercept::new();
        let (held, at) = bind_live(&sut, LEG_ADDR);
        let listener = port_of(&held);
        assert!(sut.script_accept(
            at,
            SimAcceptScript::OriginalDestinationFailure { errno: libc::ENOTCONN }
        ));
        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(peer, local))));
        let first = tokio::time::timeout(WAIT, listener.accept()).await.expect("first script");
        assert!(matches!(first, Err(InterceptAcceptError::OriginalDestination { .. })));
        let second = tokio::time::timeout(WAIT, listener.accept()).await.expect("second script");
        assert_eq!(connection_of(second), (peer, local));

        // A `local_addr` failure is standing and leaves `accept` alone.
        assert!(sut.script_local_addr_failure(at, libc::EBADF));
        for read in 0..2 {
            let failed = listener.local_addr();
            assert!(
                matches!(&failed, Err(error) if error.raw_os_error() == Some(libc::EBADF)),
                "local_addr read {read} fails with the scripted errno, got {failed:?}",
            );
        }
        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(later_peer, local))));
        let after = tokio::time::timeout(WAIT, listener.accept()).await.expect("accept unaffected");
        assert_eq!(connection_of(after), (later_peer, local));

        // Scripts die with their listener; a new listener at the address
        // starts with none.
        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(peer, local))));
        drop(listener);
        drop(held);
        assert!(!sut.script_accept(at, SimAcceptScript::Connection(accepted(peer, local))));
        assert!(!sut.script_local_addr_failure(at, libc::EBADF));
        assert_eq!(sut.parked_accepts(at), 0, "no live listener holds the address");
        let (rebound, rebound_at) = bind_live(&sut, at);
        assert_eq!(rebound_at, at, "a fresh listener reads its address: no local_addr failure");
        let fresh = port_of(&rebound);
        let mut waiting = fresh.accept();
        assert!(
            poll_once(&mut waiting).await.is_pending(),
            "the old listener's script died with it"
        );
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Polled with no current Tokio runtime, `accept` returns `Err(Accept)`
    /// naming the missing runtime — it never panics, parks nothing, and
    /// consumes no script: the scripted connection is still delivered once a
    /// runtime polls.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    fn an_accept_polled_without_a_runtime_fails_instead_of_panicking() {
        let sut = SimMtlsIntercept::new();
        let (held, at) = bind_live(&sut, LEG_ADDR);
        let listener = port_of(&held);
        let peer = SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 2), 40_000);
        let local = SocketAddrV4::new(Ipv4Addr::new(10, 99, 1, 7), 443);
        assert!(sut.script_accept(at, SimAcceptScript::Connection(accepted(peer, local))));
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "precondition: no Tokio runtime is current",
        );

        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        for attempt in 0..2 {
            let mut accept = listener.accept();
            let polled = accept.as_mut().poll(&mut context);
            assert!(
                matches!(
                    &polled,
                    std::task::Poll::Ready(Err(InterceptAcceptError::Accept { source }))
                        if source.to_string().contains("runtime")
                ),
                "attempt {attempt}: no runtime gives Err(Accept) naming it, got {polled:?}",
            );
        }
        assert_eq!(sut.parked_accepts(at), 0, "a no-runtime accept parks nothing");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a current-thread runtime builds");
        let delivered = runtime
            .block_on(async { tokio::time::timeout(WAIT, listener.accept()).await })
            .expect("the scripted connection is still pending for a runtime accept");
        assert_eq!(connection_of(delivered), (peer, local), "the failed accepts consumed nothing");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Closes the S-MIF-08 gap: `clear_faults` disarms the BIND slot, so the
    /// next `bind_transparent` takes its socket-free `Ok` arm; a second clear
    /// leaves it disarmed.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    fn clear_faults_also_disarms_the_bind_slot() {
        let sut = SimMtlsIntercept::new();
        sut.script_bind_fault(SimInterceptFault::TransparentListener { errno: libc::EPERM });
        assert_err_shape(
            &drive_expecting_err(&sut, Method::BindTransparent),
            ExpectedErr::TransparentListener { addr: LEG_ADDR, errno: libc::EPERM },
        );
        assert!(sut.live_listeners().is_empty(), "a faulted bind registers nothing");

        sut.clear_faults();
        let (_first, first_at) = bind_live(&sut, LEG_ADDR);
        assert_eq!(
            first_at,
            loopback(49_152),
            "the cleared bind slot takes the socket-free Ok arm"
        );
        assert_eq!(sut.live_listeners(), [first_at]);

        sut.clear_faults();
        let (_second, second_at) = bind_live(&sut, LEG_ADDR);
        assert_eq!(second_at, loopback(49_153), "a second clear leaves the bind slot disarmed");
        assert_eq!(sut.live_listeners(), [first_at, second_at]);
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-70 — The node's protection listeners belong to the protection port: a
    /// simulated node opens no socket, and a listener stops when its wait is cancelled.
    /// CONTRACT_SHAPE: bounded-change.
    ///
    /// Closes the S-MIF-13 gap (its fourth direction): an armed install fault
    /// leaks into neither bind, which takes its socket-free `Ok` arm, while
    /// the armed install still refuses.
    #[test]
    #[ignore = "pending DELIVER step 05-01 (S-ND295-70)"]
    fn an_install_fault_leaves_bind_on_its_success_arm() {
        for method in [Method::InstallOutbound, Method::InstallInbound] {
            let sut = SimMtlsIntercept::new();
            let node_guard = converge_at_install_ports(&sut);
            arm(
                &sut,
                method,
                SimInterceptFault::NftElementUpdate {
                    operation: InterceptElementOperation::Insert,
                    errno: libc::EPERM,
                },
            );

            let (_held, at) = bind_live(&sut, LEG_ADDR);
            assert_eq!(at, loopback(49_152), "{method:?} fault: bind takes its socket-free Ok arm");
            assert_eq!(sut.live_listeners(), [at], "{method:?} fault: the listener is live");
            let refused = drive_expecting_err(&sut, method);
            assert!(
                matches!(refused, InterceptError::NftElementUpdateFailed { .. }),
                "{method:?} fault: the converged install still reaches its armed fault, got \
                 {refused:?}"
            );
            drop(node_guard);
        }
    }
}
