//! `DnsResponder` — the node-local dial-by-name DNS host adapter (ADR-0072
//! REV-2, GH #243; roadmap 02-01 / DDN-4 / DDN-5 / DDN-6).
//!
//! # What it is
//!
//! The `DnsResponder` is the socket-loop host adapter that answers
//! `<job>.svc.overdrive.local` queries on UDP `:53`. It is the bind +
//! `recvmsg`/`sendmsg` `IP_PKTINFO` loop around the pure decision substrate the
//! prior slices landed GREEN:
//!
//! - [`super::wire`] — decode the inbound query / encode the A · NODATA-SOA ·
//!   NXDOMAIN-SOA reply (the `hickory-proto` anti-corruption boundary).
//! - [`super::answer::answer_for`] — the pure `(name, qtype, &index) →
//!   NameAnswer` decision (the `Records` arm answers the stable frontend `F`).
//! - [`super::name_index::NameIndex`] — the List-then-Watch resolvability index
//!   built INTERNALLY by [`DnsResponder::probe`] from `(store, frontend)` and
//!   queried by [`DnsResponder::serve`] via `frontend_for`.
//!
//! The responder NEVER constructs its own [`FrontendAddrAllocator`] and NEVER
//! derives `F` independently — it holds the ONE `Arc`-shared allocator the
//! composition root injects into BOTH this responder AND the re-keyed
//! `MtlsResolve` (DDN-2 single-owner invariant), so the `F` it answers is
//! byte-identical to the `F` the resolve path's `by_frontend` recognizes.
//!
//! # Bind strategy — wildcard first, per-gateway-addr fallback (DDN-5)
//!
//! [`probe`](DnsResponder::probe) binds the wildcard `0.0.0.0:53`
//! (`SO_REUSEADDR` + `IP_PKTINFO`) FIRST — the spike-validated shape that
//! coexists with systemd-resolved's specific `127.0.0.53:53` / `127.0.0.54:53`
//! binds. On `EADDRINUSE` (the appliance-image case where a wildcard `:53`
//! holder already exists) it FALLS BACK to one `:53` socket per assigned
//! gateway addr, re-derived from
//! [`NetSlotAllocator::snapshot`](crate::veth_provisioner::NetSlotAllocator::snapshot)
//! via [`responder_addr_for_slot`](crate::veth_provisioner::responder_addr_for_slot).
//! The per-gateway-addr socket set is bound ONCE at probe time, from the slot
//! snapshot as it exists then; there is NO converge tick in v1. Live slot-churn
//! tracking (add-if-missing as a slot is assigned / drop-if-absent as it is
//! released — the reconcilers.md Bar-1 shape) is deferred to
//! <https://github.com/overdrive-sh/overdrive/issues/247>.
//!
//! # Source-pin (DDN-5, the spike litmus)
//!
//! Each reply is source-pinned to the queried gateway via the inbound
//! datagram's `IP_PKTINFO` `ipi_spec_dst` — the multi-homed `0.0.0.0:53` socket
//! would otherwise reply from the host's primary addr, which `getaddrinfo` /
//! `getent` rejects (the spike finding: `dig` accepts a missing source-pin,
//! `getent` does not). The `getent` path is the acceptance litmus.
//!
//! # Earned-Trust gate (DDN-6 — wire → probe → use)
//!
//! [`probe`](DnsResponder::probe) binds AND List-seeds the internal
//! [`NameIndex`](super::name_index::NameIndex) (`name_index.probe()`). A bind
//! failure ([`DnsResponderError::Bind`]) or an unreadable store at List-seed
//! ([`DnsResponderError::ListSeed`]) returns `Err` so the composition root
//! REFUSES boot with a structured `health.startup.refused` event — a responder
//! that bound lazily could start and THEN fail to answer (the silent-degradation
//! footgun). Each [`DnsResponderError`] variant maps to a DISTINCT refusal
//! reason (`.claude/rules/development.md` § "Never flatten a typed error to
//! `Internal(String)`"); there is NO `Internal(String)` variant.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use nix::sys::socket::sockopt::{Ipv4PacketInfo, ReceiveTimeout, ReuseAddr};
use nix::sys::socket::{
    AddressFamily, ControlMessage, ControlMessageOwned, MsgFlags, SockFlag, SockProtocol, SockType,
    SockaddrIn, bind, recvmsg, sendmsg, setsockopt, socket,
};
use nix::sys::time::TimeVal;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::observation_store::ObservationStore;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::answer::answer_for;
use super::frontend_addr_allocator::FrontendAddrAllocator;
use super::name_index::NameIndex;
use super::wire;
use crate::veth_provisioner::{NetSlotAllocator, responder_addr_for_slot};

/// The well-known DNS port the responder binds.
const DNS_PORT: u16 = 53;

/// Max inbound DNS datagram size we read (UDP DNS messages are ≤ 512 bytes
/// without EDNS0; a 1500-byte MTU-sized buffer is comfortably large enough for
/// the v1 A / SOA replies and any EDNS0-padded query).
const RECV_BUF_LEN: usize = 1500;

/// `recvmsg` poll cadence — `SO_RCVTIMEO` so the blocking recv wakes every
/// 500ms to re-check the serve loop's stop flag, making the loop cancellable
/// (a permanently-parked recvmsg cannot be aborted and would leak at teardown).
fn recv_poll_timeout() -> TimeVal {
    TimeVal::new(0, 500_000)
}

/// Typed errors for the [`DnsResponder`] bind / List-seed / probe / socket
/// legs (ADR-0072 DDN-6).
///
/// Each variant maps to a DISTINCT `health.startup.refused` reason at the
/// composition root — a mutant collapsing two variants onto one reason flips
/// S-DBN-BIND-03. There is NO `Internal(String)` variant
/// (`.claude/rules/development.md` § "Never flatten a typed error to
/// `Internal(String)`"): each failure mode carries its own structured cause so
/// the CLI / §12 investigation agent can branch on it without `Display`-grepping.
#[derive(Debug, thiserror::Error)]
pub enum DnsResponderError {
    /// No bindable `:53` socket — the wildcard `0.0.0.0:53` AND every
    /// per-gateway-addr `:53` candidate are already held. The node refuses to
    /// start (`health.startup.refused`, reason `dns.responder.bind`).
    #[error("DNS responder bind failed for {addr}: {source}")]
    Bind {
        /// The address the failing bind targeted (the wildcard `0.0.0.0:53` or
        /// a per-gateway-addr `:53`).
        addr: SocketAddr,
        /// The underlying `io::Error` (`EADDRINUSE`, `EACCES`, …).
        #[source]
        source: std::io::Error,
    },

    /// The `service_backends` observation surface was unreadable at the
    /// internal [`NameIndex`](super::name_index::NameIndex) List-seed leg
    /// (`name_index.probe()`). The node refuses to start
    /// (`health.startup.refused`, reason `dns.responder.listseed`).
    #[error("DNS responder List-seed failed: {reason}")]
    ListSeed {
        /// The underlying store-read failure rendered for diagnostics.
        reason: String,
    },

    /// A non-bind, non-List-seed probe failure (the catch-all probe leg, e.g.
    /// the internal index watch-open). Distinct from [`Self::Bind`] /
    /// [`Self::ListSeed`] so the operator sees WHICH probe leg refused.
    #[error("DNS responder probe failed: {reason}")]
    Probe {
        /// The underlying probe failure rendered for diagnostics.
        reason: String,
    },

    /// A socket-level configuration failure (`SO_REUSEADDR` / `IP_PKTINFO`
    /// setsockopt, `recvmsg`/`sendmsg` setup) distinct from the bind itself —
    /// the socket was created but could not be configured for the multi-homed
    /// source-pinned loop.
    #[error("DNS responder socket configuration failed: {source}")]
    Socket {
        /// The underlying `io::Error` from the failing setsockopt / socket op.
        #[source]
        source: std::io::Error,
    },
}

impl DnsResponderError {
    /// The DISTINCT `health.startup.refused` reason string for this variant —
    /// the enum owns its own refusal vocabulary (`.claude/rules/development.md`
    /// § "Label enums own their string representation"). The composition root
    /// (`run_server`) calls this when a `probe()` failure refuses the boot, so
    /// the per-variant reason is the SSOT here rather than an inline `match`
    /// scattered at the call site. A mutant collapsing two variants onto one
    /// reason flips the Tier-1 mapping test (`boot_refusal_reason_*` in this
    /// module's `#[cfg(test)]`) AND the Tier-3 `run_server`-refusal test
    /// (`dns_responder_bind.rs`).
    #[must_use]
    pub(crate) const fn boot_refusal_reason(&self) -> &'static str {
        match self {
            Self::Bind { .. } => "dns.responder.bind",
            Self::ListSeed { .. } => "dns.responder.listseed",
            Self::Probe { .. } => "dns.responder.probe",
            Self::Socket { .. } => "dns.responder.socket",
        }
    }
}

/// Result alias used throughout the `responder` module (crate convention:
/// every error type ships a matching `Result` alias — `CLAUDE.md` § "Rust
/// library conventions").
pub type Result<T, E = DnsResponderError> = std::result::Result<T, E>;

/// The node-local dial-by-name DNS host adapter (ADR-0072 REV-2). Binds UDP
/// `:53` (wildcard-first, per-gateway-addr fallback), List-seeds the internal
/// [`NameIndex`](super::name_index::NameIndex), and answers
/// `<job>.svc.overdrive.local` with the stable frontend `F` the ONE shared
/// [`FrontendAddrAllocator`] binds. See the module rustdoc for the full
/// contract.
pub struct DnsResponder {
    /// The injected [`Clock`] — the SOA SERIAL source for
    /// [`wire::encode`](super::wire) replies. Mandatory; never wall-clock.
    clock: Arc<dyn Clock>,
    /// The per-`AllocationId` [`NetSlotAllocator`] — the DDN-5 per-gateway-addr
    /// fallback source (`snapshot()` → `responder_addr_for_slot`). DISTINCT
    /// from [`Self::frontend`]: this is the reply-source-pin / per-addr
    /// fallback allocator, a DIFFERENT concern.
    slots: NetSlotAllocator,
    /// The internal List-then-Watch resolvability index, built in
    /// [`Self::new`] from `(store, frontend)` and List-seeded by
    /// [`Self::probe`]. It HOLDS the ONE `Arc`-shared
    /// [`FrontendAddrAllocator`] (DDN-2 single-owner) — the SAME instance
    /// injected into the re-keyed `MtlsResolve`'s `by_frontend` — and answers
    /// `F` from a pure read of its snapshot. The serve loop answers each query
    /// via `name_index.frontend_for` → [`answer_for`](super::answer::answer_for).
    /// The responder NEVER constructs its own allocator or derives `F`
    /// independently — it reads only what the index exposes.
    name_index: NameIndex,
    /// The bound `:53` sockets, populated by [`Self::probe`] and consumed by
    /// [`Self::serve`]. `recvmsg`-mode `OwnedFd`s (each with `SO_REUSEADDR` +
    /// `IP_PKTINFO`); the wildcard path holds exactly one, the per-gateway-addr
    /// fallback holds one per assigned slot. Behind a `Mutex` so the serve loop
    /// can `take()` ownership at start (the loop is `self: Arc<Self>`).
    sockets: Mutex<Vec<OwnedFd>>,
    /// Stop flag for the serve loop. The `SO_RCVTIMEO`-bounded `recvmsg` wakes
    /// every `recv_poll_timeout()` and re-checks this; set `true` by
    /// [`Self::stop`] (called on `ServerHandle` shutdown / test teardown) so the
    /// blocking loop exits promptly rather than leaking an uncancellable
    /// syscall.
    stop: Arc<AtomicBool>,
}

impl DnsResponder {
    /// Construct the responder from its REQUIRED dependencies — all mandatory,
    /// no builder. The internal [`NameIndex`](super::name_index::NameIndex) is
    /// built from `(store, frontend)` inside [`probe`](Self::probe); the
    /// responder does NOT take a `NameIndex` (DDN-2: the `name_index` reads the
    /// SAME shared `frontend` allocator the resolve path keys `by_frontend`
    /// from).
    #[must_use]
    pub fn new(
        store: Arc<dyn ObservationStore>,
        clock: Arc<dyn Clock>,
        slots: NetSlotAllocator,
        frontend: FrontendAddrAllocator,
    ) -> Self {
        // Build the internal NameIndex from the SAME store + the SHARED
        // allocator (DDN-2): the index answers `F` from this allocator's
        // snapshot, byte-identical to the `F` the re-keyed `MtlsResolve` keys
        // `by_frontend` from.
        let name_index = NameIndex::new(store, frontend);
        Self {
            clock,
            slots,
            name_index,
            sockets: Mutex::new(Vec::new()),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal the serve loop to stop. The `SO_RCVTIMEO`-bounded `recvmsg` wakes
    /// within `recv_poll_timeout()` and the loop exits. Idempotent. Called on
    /// `ServerHandle::shutdown` / test teardown so the blocking serve tasks do
    /// not leak an uncancellable syscall and hang runtime teardown.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Bind UDP `:53` (wildcard-first, per-gateway-addr fallback) and List-seed
    /// the internal [`NameIndex`](super::name_index::NameIndex) — the
    /// Earned-Trust "wire → probe → use" gate (DDN-6).
    ///
    /// # Errors
    ///
    /// - [`DnsResponderError::Bind`] when no `:53` socket can be bound (the
    ///   wildcard AND every per-gateway-addr candidate are held).
    /// - [`DnsResponderError::ListSeed`] when the `service_backends` surface is
    ///   unreadable at the internal index's List-seed.
    /// - [`DnsResponderError::Socket`] on a setsockopt / socket-config failure.
    ///
    /// On any `Err` the composition root REFUSES boot with a structured
    /// `health.startup.refused` event.
    pub async fn probe(&self) -> Result<()> {
        // (1) Bind the wildcard `0.0.0.0:53` FIRST (SO_REUSEADDR + IP_PKTINFO).
        // On EADDRINUSE fall back to one socket per assigned gateway addr,
        // re-derived from the NetSlotAllocator snapshot (DDN-5). The bound
        // sockets are stashed for `serve`.
        let bound = match bind_one(Ipv4Addr::UNSPECIFIED) {
            Ok(fd) => vec![fd],
            Err(err) if is_addr_in_use(&err) => self.bind_per_gateway_addr()?,
            Err(source) => {
                return Err(DnsResponderError::Bind {
                    addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DNS_PORT)),
                    source,
                });
            }
        };
        // Silent-deaf-responder guard (N2): the fallback path with an empty slot
        // snapshot binds ZERO sockets, so `probe()` returns Ok, `serve` spawns
        // nothing, and the responder answers nothing — indistinguishable from a
        // healthy boot in the logs. With no converge tick (deferred to #247) it
        // stays deaf for the process lifetime. Emit a structured warning so the
        // degraded boot is observable. (The wildcard path always binds exactly
        // one socket, so an empty `bound` is unambiguously the fallback branch.)
        if bound.is_empty() {
            tracing::warn!(
                name: "dns.responder.fallback.zero_sockets",
                "dial-by-name DNS responder fell back to per-gateway-addr binding but the slot \
                 snapshot was empty — bound ZERO sockets and is currently DEAF (answers no \
                 queries). With no converge tick (https://github.com/overdrive-sh/overdrive/issues/247) \
                 this persists for the process lifetime; restart the node once a gateway slot is \
                 assigned, or land #247."
            );
        }
        *self.sockets.lock() = bound;

        // (2) List-seed the internal NameIndex (the Earned-Trust List-then-Watch
        // leg). An unreadable `service_backends` surface → ListSeed refusal.
        self.name_index
            .probe()
            .await
            .map_err(|source| DnsResponderError::ListSeed { reason: source.to_string() })?;
        Ok(())
    }

    /// Bind one `:53` socket per currently-assigned gateway addr, re-derived
    /// from `self.slots.snapshot()` via `responder_addr_for_slot` (the DDN-5
    /// per-gateway-addr fallback). Each socket gets `SO_REUSEADDR` + `IP_PKTINFO`.
    /// The sockets are bound ONCE, from the slot snapshot AS IT EXISTS AT PROBE
    /// TIME — there is no converge tick in v1, so a slot assigned AFTER probe
    /// gets no socket until the process is restarted. Live slot-churn tracking
    /// (add-if-missing / drop-if-absent) is deferred to
    /// <https://github.com/overdrive-sh/overdrive/issues/247>. An empty snapshot
    /// binds nothing — a degenerate fallback the caller [`probe`](Self::probe)
    /// warns on (`dns.responder.fallback.zero_sockets`), because with no
    /// converge a zero-socket responder is permanently deaf for the process
    /// lifetime (also tracked by #247).
    fn bind_per_gateway_addr(&self) -> Result<Vec<OwnedFd>> {
        let mut bound = Vec::new();
        for slot in self.slots.snapshot().values().copied() {
            let gateway = responder_addr_for_slot(slot);
            let fd = bind_one(gateway).map_err(|source| DnsResponderError::Bind {
                addr: SocketAddr::V4(SocketAddrV4::new(gateway, DNS_PORT)),
                source,
            })?;
            bound.push(fd);
        }
        Ok(bound)
    }

    /// Run the `recvmsg`/`sendmsg` `IP_PKTINFO` serve loop: decode each inbound
    /// query, answer via the internal [`NameIndex`](super::name_index::NameIndex)
    /// (`frontend_for` → [`answer_for`](super::answer::answer_for) →
    /// [`wire::encode`](super::wire)), and `sendmsg` the reply source-pinned to
    /// the queried gateway via `ipi_spec_dst`. `serve` runs the bound socket set
    /// as captured at probe time (`std::mem::take`) — it does NOT re-derive the
    /// slot snapshot, so there is no converge of the per-gateway-addr socket set
    /// to the live slot set in v1 (deferred to
    /// <https://github.com/overdrive-sh/overdrive/issues/247>).
    ///
    /// Consumes `self: Arc<Self>` so the composition root can `tokio::spawn` the
    /// loop and hold the `JoinHandle`.
    pub async fn serve(self: Arc<Self>) {
        // Take ownership of the bound sockets (probe stashed them). Spawn one
        // blocking recv/answer/send task per socket — `recvmsg` parks the
        // thread, so each socket runs on its own `spawn_blocking` worker.
        let sockets = std::mem::take(&mut *self.sockets.lock());
        let mut handles = Vec::with_capacity(sockets.len());
        for fd in sockets {
            let responder = Arc::clone(&self);
            handles.push(tokio::task::spawn_blocking(move || responder.serve_one_socket(&fd)));
        }
        for handle in handles {
            // A socket loop only ends on an unrecoverable error; awaiting keeps
            // the serve task alive for the process lifetime (it is aborted on
            // shutdown via the held JoinHandle).
            let _ = handle.await;
        }
    }

    /// The blocking `recvmsg`/`sendmsg` `IP_PKTINFO` loop for ONE bound socket.
    /// Each inbound datagram is decoded, answered via the internal `NameIndex`
    /// (`frontend_for` → [`answer_for`] → [`wire::encode`]), and replied
    /// source-pinned to the datagram's `ipi_spec_dst` (the gateway the query
    /// was addressed to) via a `ControlMessage::Ipv4PacketInfo` on `sendmsg` —
    /// the DDN-5 source-pin the `getent` litmus requires on the multi-homed
    /// wildcard socket.
    fn serve_one_socket(&self, fd: &OwnedFd) {
        let mut buf = [0u8; RECV_BUF_LEN];
        let mut cmsg_space = nix::cmsg_space!(libc::in_pktinfo);
        loop {
            // Cancellation check — the SO_RCVTIMEO below bounds the recvmsg
            // block so this is re-evaluated every `recv_poll_timeout()`.
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            // Receive into a scoped block so the `iov` → `buf` mutable borrow
            // ends before `buf` is re-read: copy out the datagram bytes, the
            // peer, and the ipi_spec_dst. `None` on a timeout/interrupt (loop)
            // or a fatal recv error (return).
            let received: Option<(Vec<u8>, SockaddrIn, Option<Ipv4Addr>)> = {
                let mut iov = [std::io::IoSliceMut::new(&mut buf)];
                match recvmsg::<SockaddrIn>(
                    fd.as_raw_fd(),
                    &mut iov,
                    Some(&mut cmsg_space),
                    MsgFlags::empty(),
                ) {
                    Ok(recvd) => {
                        let Some(peer) = recvd.address else { continue };
                        let bytes = recvd.bytes;
                        // The dst addr the query was addressed to (ipi_spec_dst)
                        // — the source we MUST reply from on a multi-homed socket.
                        // Read the cmsgs (the last use of `recvd`, which borrows
                        // `iov`) BEFORE re-borrowing `iov` for the datagram copy,
                        // so the two borrows do not overlap.
                        let spec_dst = recvd.cmsgs().ok().and_then(|mut cmsgs| {
                            cmsgs.find_map(|cmsg| match cmsg {
                                ControlMessageOwned::Ipv4PacketInfo(info) => {
                                    Some(Ipv4Addr::from(u32::from_be(info.ipi_spec_dst.s_addr)))
                                }
                                _ => None,
                            })
                        });
                        Some((iov[0][..bytes].to_vec(), peer, spec_dst))
                    }
                    // SO_RCVTIMEO fired (no datagram this window) or an interrupt:
                    // loop back to re-check the stop flag and re-block.
                    Err(nix::errno::Errno::EINTR | nix::errno::Errno::EAGAIN) => continue,
                    // An unrecoverable error ends this socket's loop.
                    Err(_) => return,
                }
            };
            let Some((datagram, peer, spec_dst)) = received else { continue };
            let Some(reply) = self.answer_datagram(&datagram) else { continue };
            // Source-pin the reply to the queried gateway via ipi_spec_dst.
            let pktinfo = spec_dst.map(|ip| libc::in_pktinfo {
                ipi_ifindex: 0,
                ipi_spec_dst: libc::in_addr { s_addr: u32::from(ip).to_be() },
                ipi_addr: libc::in_addr { s_addr: 0 },
            });
            let send_iov = [std::io::IoSlice::new(&reply)];
            let cmsgs: Vec<ControlMessage> = pktinfo
                .as_ref()
                .map(|pi| vec![ControlMessage::Ipv4PacketInfo(pi)])
                .unwrap_or_default();
            let sent = sendmsg(fd.as_raw_fd(), &send_iov, &cmsgs, MsgFlags::empty(), Some(&peer));
            if let Err(errno) = sent {
                tracing::warn!(
                    name: "dns.responder.sendmsg_failed",
                    %errno,
                    "DNS responder reply sendmsg failed"
                );
            }
        }
    }

    /// Decode → answer → encode one inbound datagram, returning the reply bytes
    /// (or `None` when the datagram is not a decodable mesh query — a malformed
    /// or non-mesh query is silently dropped, never answered with a fabricated
    /// addr). The SOA SERIAL is read from the injected clock (`unix_now`).
    fn answer_datagram(&self, datagram: &[u8]) -> Option<Vec<u8>> {
        let query = wire::decode(datagram).ok()?;
        let answer = answer_for(&query.name, query.qtype, &self.name_index);
        Some(wire::encode(query.id, &query.name, query.qtype, &answer, self.clock.unix_now()))
    }
}

/// Bind ONE UDP `:53` socket to `addr` with `SO_REUSEADDR` (coexist with
/// systemd-resolved's specific binds) + `IP_PKTINFO` (so `recvmsg` surfaces the
/// `ipi_spec_dst` the reply is source-pinned to). Returns the bound `OwnedFd`.
///
/// # Errors
///
/// Propagates the underlying `io::Error` — the caller maps `EADDRINUSE` to the
/// per-gateway-addr fallback and any other error to [`DnsResponderError::Bind`].
fn bind_one(addr: Ipv4Addr) -> std::io::Result<OwnedFd> {
    let fd = socket(AddressFamily::Inet, SockType::Datagram, SockFlag::empty(), SockProtocol::Udp)
        .map_err(std::io::Error::from)?;
    // SO_REUSEADDR — coexist with systemd-resolved's specific 127.0.0.53:53 /
    // 127.0.0.54:53 binds (the spike-validated wildcard coexistence shape).
    setsockopt(&fd, ReuseAddr, &true).map_err(std::io::Error::from)?;
    // IP_PKTINFO — recvmsg surfaces ipi_spec_dst (the dst the query was
    // addressed to) so the reply can be source-pinned on a multi-homed socket
    // (the DDN-5 source-pin the getent litmus requires).
    setsockopt(&fd, Ipv4PacketInfo, &true).map_err(std::io::Error::from)?;
    // SO_RCVTIMEO — bound the `recvmsg` block so the serve loop is NOT a
    // permanently-uncancellable syscall: it wakes every RECV_POLL_TIMEOUT,
    // re-checks the abort flag, and re-blocks. Without this a `spawn_blocking`
    // recvmsg parks forever and the runtime cannot shut down (the loop leaks at
    // teardown / on `ServerHandle::shutdown`).
    setsockopt(&fd, ReceiveTimeout, &recv_poll_timeout()).map_err(std::io::Error::from)?;
    bind(fd.as_raw_fd(), &SockaddrIn::from(SocketAddrV4::new(addr, DNS_PORT)))
        .map_err(std::io::Error::from)?;
    Ok(fd)
}

/// Whether an `io::Error` from `bind` is `EADDRINUSE` (the wildcard-already-held
/// case that triggers the per-gateway-addr fallback).
fn is_addr_in_use(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::AddrInUse || err.raw_os_error() == Some(libc::EADDRINUSE)
}

#[cfg(test)]
mod tests {
    use super::DnsResponderError;

    /// Each [`DnsResponderError`] variant maps to its OWN
    /// `health.startup.refused` reason — the four reasons are distinct, so a
    /// mutant collapsing any two arms to one literal (the inline-match flatten
    /// the `run_server` call site used to carry) flips this in-process. This is
    /// the Tier-1 half of the D2 fix: the reason mapping is no longer an
    /// untested inline match the Tier-3 `probe()` path bypasses.
    #[test]
    fn boot_refusal_reason_maps_each_variant_to_a_distinct_reason() {
        let bind = DnsResponderError::Bind {
            addr: std::net::SocketAddr::V4(std::net::SocketAddrV4::new(
                std::net::Ipv4Addr::UNSPECIFIED,
                super::DNS_PORT,
            )),
            source: std::io::Error::from(std::io::ErrorKind::AddrInUse),
        };
        let list_seed = DnsResponderError::ListSeed { reason: "store unreadable".to_owned() };
        let probe = DnsResponderError::Probe { reason: "watch open failed".to_owned() };
        let socket = DnsResponderError::Socket {
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };

        // (1) each variant carries its own distinct reason string.
        assert_eq!(bind.boot_refusal_reason(), "dns.responder.bind");
        assert_eq!(list_seed.boot_refusal_reason(), "dns.responder.listseed");
        assert_eq!(probe.boot_refusal_reason(), "dns.responder.probe");
        assert_eq!(socket.boot_refusal_reason(), "dns.responder.socket");

        // (2) no two variants collapse onto the same reason — the property a
        // flatten-mutant violates (all four reasons distinct).
        let reasons = [
            bind.boot_refusal_reason(),
            list_seed.boot_refusal_reason(),
            probe.boot_refusal_reason(),
            socket.boot_refusal_reason(),
        ];
        let unique: std::collections::BTreeSet<&str> = reasons.iter().copied().collect();
        assert_eq!(unique.len(), reasons.len(), "all four refusal reasons must be distinct");
    }
}
