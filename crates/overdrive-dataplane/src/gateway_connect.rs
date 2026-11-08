//! `GatewayConnectDataplane` for [`EbpfDataplane`] (public-ingress-gateway,
//! ADR-0228 / ADR-0232 / ADR-0233 / ADR-0243).
//!
//! The dataplane prepares the gateway's upstream socket (marks it
//! `MTLS_TPROXY_MARK`, loopback-binds it, reads its socket cookie) and
//! registers `(cookie, ServiceKey)` in `GATEWAY_CONNECT_INTENT_MAP`; the
//! separate gateway selection `cgroup/connect4` program writes the receipt into
//! `GATEWAY_SELECTION_RECEIPT_MAP`; `take_receipt` consumes it. The applied
//! identity of each backend is published by `update_service` after the outer
//! `SERVICE_MAP` swap under the commit guard, so a `Selected` receipt can always
//! resolve its identity and never an uncommitted one.
//!
//! `probe`, `open_upstream`, `take_receipt`, `selected_backend_identity` and the
//! cleanup capability are implemented. The commit-guard internals (`BackendIdentityState`,
//! `ServiceApplyUndo`, `ServiceCleanupDebt`, the `service_apply_commit` guard) are
//! private structure and are the crafter's.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU16;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::Instant;

use async_trait::async_trait;
use overdrive_core::dataplane::{MTLS_TPROXY_MARK, Proto, ServiceFrontend};
use overdrive_core::id::{BackendId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::{ServiceKey, SocketCookie};
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectCleanupSweep, GatewayConnectDataplane,
    GatewayConnectDataplaneError, GatewayConnectIntent, GatewaySelectionReceipt, PreparedUpstream,
};

use overdrive_host::socket_cookie::{SocketCookieReadError, SocketCookieReader};

use crate::EbpfDataplane;

/// The applied-identity state of one `BackendId` (ADR-0233, ADR-0241).
///
/// `Reserved` is inserted before any map work of an `update_service` and is not
/// readable; `Applied` replaces it after the outer swap. An `Applied` identity is
/// never changed or removed for the process lifetime.
pub(crate) enum BackendIdentityState {
    Reserved { spiffe_id: SpiffeId },
    Applied { spiffe_id: SpiffeId },
}

impl BackendIdentityState {
    /// The allocation this id denotes, in either state.
    pub(crate) const fn spiffe_id(&self) -> &SpiffeId {
        match self {
            Self::Reserved { spiffe_id } | Self::Applied { spiffe_id } => spiffe_id,
        }
    }
}

/// Receipt outcome: no usable backend.
pub(crate) const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
/// Receipt outcome: a nonzero `BackendId` was selected.
pub(crate) const GATEWAY_RECEIPT_SELECTED: u32 = 2;

/// Registered intent value: exactly the Service-map key, host-order.
/// Mirrors `overdrive_bpf::maps::gateway_connect::GatewayConnectIntentPod`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

/// Selection receipt value. Mirrors
/// `overdrive_bpf::maps::gateway_connect::GatewaySelectionReceiptPod`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct GatewaySelectionReceiptPod {
    pub outcome: u32,
    pub backend_id: u32,
}

// SAFETY: `repr(C)`, eight bytes, every byte initialised (`_pad` is always
// zero), no interior pointers.
unsafe impl aya::Pod for GatewayConnectIntentPod {}
// SAFETY: `repr(C)`, eight bytes, two `u32` fields, no padding.
unsafe impl aya::Pod for GatewaySelectionReceiptPod {}

const _: () = assert!(core::mem::size_of::<GatewayConnectIntentPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewayConnectIntentPod>() == 4);
const _: () = assert!(core::mem::size_of::<GatewaySelectionReceiptPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewaySelectionReceiptPod>() == 4);

/// `errno` of a full hash map (`E2BIG`).
fn is_map_full(error: &aya::maps::MapError) -> bool {
    matches!(
        error,
        aya::maps::MapError::SyscallError(syscall) if syscall.io_error.raw_os_error() == Some(libc::E2BIG)
    )
}

/// A delete of an absent key. aya reports a missing key on `remove` as the
/// `ENOENT` syscall error and on `get` as `KeyNotFound`.
pub(crate) fn is_key_absent(error: &aya::maps::MapError) -> bool {
    match error {
        aya::maps::MapError::KeyNotFound => true,
        aya::maps::MapError::SyscallError(syscall) => {
            syscall.io_error.raw_os_error() == Some(libc::ENOENT)
        }
        _ => false,
    }
}

/// The Service-map key as the intent value, host-order. IPv4 only: a
/// `ServiceKey` projects from a `ServiceFrontend`, which is IPv4 by
/// construction.
fn intent_pod(key: &ServiceKey) -> Result<GatewayConnectIntentPod, GatewayConnectDataplaneError> {
    let Some(vip) = key.vip().try_as_ipv4() else {
        return Err(GatewayConnectDataplaneError::Unavailable);
    };
    Ok(GatewayConnectIntentPod {
        vip_host: u32::from(vip),
        port_host: key.port().get(),
        proto: key.protocol().as_u8(),
        _pad: 0,
    })
}

/// The `ServiceKey` an intent value denotes.
fn service_key_of(
    pod: GatewayConnectIntentPod,
) -> Result<ServiceKey, GatewayConnectDataplaneError> {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::from(pod.vip_host)))
        .map_err(|_| GatewayConnectDataplaneError::Kernel)?;
    let port = NonZeroU16::new(pod.port_host).ok_or(GatewayConnectDataplaneError::Kernel)?;
    let proto = Proto::try_from(pod.proto).map_err(|_| GatewayConnectDataplaneError::Kernel)?;
    let frontend =
        ServiceFrontend::new(vip, port, proto).map_err(|_| GatewayConnectDataplaneError::Kernel)?;
    Ok(ServiceKey::from_frontend(frontend))
}

/// Decode the two canonical receipt value shapes; anything else is a mismatch.
fn decode_receipt(
    receipt: GatewaySelectionReceiptPod,
    socket_cookie: SocketCookie,
    service_key: ServiceKey,
) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
    match (receipt.outcome, receipt.backend_id) {
        (GATEWAY_RECEIPT_SELECTED, raw) if raw != 0 => {
            let backend_id =
                BackendId::new(raw).map_err(|_| GatewayConnectDataplaneError::ReceiptMismatch)?;
            Ok(GatewaySelectionReceipt::Selected { socket_cookie, service_key, backend_id })
        }
        (GATEWAY_RECEIPT_NO_BACKEND, 0) => {
            Ok(GatewaySelectionReceipt::NoBackend { socket_cookie, service_key })
        }
        _ => Err(GatewayConnectDataplaneError::ReceiptMismatch),
    }
}

/// An unconnected `AF_INET` stream socket with `SO_MARK = MTLS_TPROXY_MARK`,
/// `IP_BIND_ADDRESS_NO_PORT` and a bind to `127.0.0.1:0`, all set before
/// `connect(2)` (ADR-0190: mark alone fails `ENETUNREACH`, bind alone `EINVAL`;
/// both route the dial through table 100 with a loopback source).
fn prepared_socket() -> Result<tokio::net::TcpSocket, GatewayConnectDataplaneError> {
    fn set_int(
        fd: std::os::fd::RawFd,
        level: libc::c_int,
        name: libc::c_int,
        value: libc::c_int,
    ) -> Result<(), std::io::ErrorKind> {
        let len = libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>())
            .map_err(|_| std::io::ErrorKind::InvalidInput)?;
        // SAFETY: `fd` is a live socket owned by the caller; `value` is valid for
        // reads of `size_of::<c_int>()` bytes for the duration of the call.
        let rc =
            unsafe { libc::setsockopt(fd, level, name, std::ptr::from_ref(&value).cast(), len) };
        if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().kind()) }
    }

    // SAFETY: plain `socket(2)`; the result is checked before use.
    let raw = unsafe {
        libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0)
    };
    if raw < 0 {
        return Err(GatewayConnectDataplaneError::SocketCreate(
            std::io::Error::last_os_error().kind(),
        ));
    }
    // SAFETY: `raw` is a freshly created descriptor with no other owner; the
    // `OwnedFd` closes it on every error path below.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };

    #[allow(clippy::cast_possible_wrap)]
    let mark = MTLS_TPROXY_MARK as libc::c_int;
    set_int(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_MARK, mark)
        .map_err(GatewayConnectDataplaneError::SocketOption)?;
    set_int(fd.as_raw_fd(), libc::SOL_IP, libc::IP_BIND_ADDRESS_NO_PORT, 1)
        .map_err(GatewayConnectDataplaneError::SocketOption)?;

    // SAFETY: an all-zero `sockaddr_in` is a valid value; the fields set below
    // make it the IPv4 loopback address with an unspecified port.
    let mut local: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    local.sin_family = libc::sa_family_t::try_from(libc::AF_INET)
        .map_err(|_| GatewayConnectDataplaneError::SocketBind(std::io::ErrorKind::InvalidInput))?;
    local.sin_addr.s_addr = u32::from_ne_bytes(Ipv4Addr::LOCALHOST.octets());
    let local_len = libc::socklen_t::try_from(std::mem::size_of_val(&local))
        .map_err(|_| GatewayConnectDataplaneError::SocketBind(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: `local` is valid for reads of the length passed.
    let bind_rc =
        unsafe { libc::bind(fd.as_raw_fd(), std::ptr::from_ref(&local).cast(), local_len) };
    if bind_rc != 0 {
        return Err(GatewayConnectDataplaneError::SocketBind(
            std::io::Error::last_os_error().kind(),
        ));
    }

    // SAFETY: the descriptor is an unconnected, nonblocking `AF_INET` stream
    // socket; ownership moves into the `TcpSocket`.
    Ok(unsafe { tokio::net::TcpSocket::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(fd)) })
}

/// The sentinel the boot probe dials: a TEST-NET-1 address and the discard
/// port, never programmed as a Service.
const SENTINEL_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const SENTINEL_PORT: u16 = 9;

fn sentinel_service_key() -> Result<ServiceKey, GatewayConnectDataplaneError> {
    let vip = ServiceVip::new(IpAddr::V4(SENTINEL_ADDRESS))
        .map_err(|_| GatewayConnectDataplaneError::Unavailable)?;
    let port = NonZeroU16::new(SENTINEL_PORT).ok_or(GatewayConnectDataplaneError::Unavailable)?;
    let frontend = ServiceFrontend::new(vip, port, Proto::Tcp)
        .map_err(|_| GatewayConnectDataplaneError::Unavailable)?;
    Ok(ServiceKey::from_frontend(frontend))
}

impl EbpfDataplane {
    /// Retry the reverse-NAT keys earlier cleanups could not delete (R-61). A
    /// key that still fails stays in `service_cleanup_debt` as closed debt; a
    /// key its owner has since re-claimed is dropped from the debt untouched.
    pub(crate) fn retry_service_cleanup_debt(&self) {
        // Serialised with `update_service`, which holds this guard for its whole
        // map work.
        let _serialised = self.service_apply_commit.write();
        self.settle_reverse_nat_debt(std::collections::BTreeSet::new());
    }
}

impl GatewayConnectCleanup for EbpfDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let cookie = intent.socket_cookie.get().get();
        // Both removals run under both locks (intent, then receipt: the order
        // `live_receipts` takes), so a snapshot never sees a receipt whose intent
        // is already gone. An absent key is already clean.
        let mut intents = self.gateway_intent_map.lock();
        let mut receipts = self.gateway_receipt_map.lock();
        let intent_removed = remove_key(&mut intents, cookie);
        let receipt_removed = remove_key(&mut receipts, cookie);
        drop(receipts);
        drop(intents);
        intent_removed.and(receipt_removed)
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        // Both locks are held across the sweep, in the intent-then-receipt order.
        let mut intents = self.gateway_intent_map.lock();
        let mut receipts = self.gateway_receipt_map.lock();
        let removed_intents = remove_all(&mut intents)?;
        let removed_receipts = remove_all(&mut receipts)?;
        drop(receipts);
        drop(intents);
        // The boot probe and the shutdown sweep both retry the closed
        // reverse-NAT debt (R-61).
        self.retry_service_cleanup_debt();
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        self.gateway_intent_map
            .lock()
            .keys()
            .try_fold(0_u32, |count, key| key.map(|_| count.saturating_add(1)))
            .map_err(|_| GatewayConnectDataplaneError::Kernel)
    }

    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        let intents = self.gateway_intent_map.lock();
        let receipts = self.gateway_receipt_map.lock();
        let mut snapshot = BTreeMap::new();
        for entry in receipts.iter() {
            let (raw_cookie, receipt) = entry.map_err(|_| GatewayConnectDataplaneError::Kernel)?;
            let cookie = SocketCookie::new(raw_cookie)
                .map_err(|_| GatewayConnectDataplaneError::ReceiptMismatch)?;
            let bound = intents
                .get(&raw_cookie, 0)
                .map_err(|_| GatewayConnectDataplaneError::ReceiptMismatch)?;
            snapshot.insert(cookie, decode_receipt(receipt, cookie, service_key_of(bound)?)?);
        }
        drop(receipts);
        drop(intents);
        Ok(snapshot)
    }
}

/// Remove `cookie`; an absent key is success, any other failure is `Cleanup`.
fn remove_key<V: aya::Pod>(
    map: &mut aya::maps::HashMap<aya::maps::MapData, u64, V>,
    cookie: u64,
) -> Result<(), GatewayConnectDataplaneError> {
    match map.remove(&cookie) {
        Ok(()) => Ok(()),
        Err(error) if is_key_absent(&error) => Ok(()),
        Err(_) => Err(GatewayConnectDataplaneError::Cleanup),
    }
}

/// Remove every key and return the exact removed count.
fn remove_all<V: aya::Pod>(
    map: &mut aya::maps::HashMap<aya::maps::MapData, u64, V>,
) -> Result<u32, GatewayConnectDataplaneError> {
    let keys = map
        .keys()
        .collect::<Result<Vec<u64>, _>>()
        .map_err(|_| GatewayConnectDataplaneError::Cleanup)?;
    let mut removed: u32 = 0;
    for key in keys {
        match map.remove(&key) {
            Ok(()) => removed = removed.saturating_add(1),
            Err(error) if is_key_absent(&error) => {}
            Err(_) => return Err(GatewayConnectDataplaneError::Cleanup),
        }
    }
    Ok(removed)
}

#[async_trait]
impl GatewayConnectDataplane for EbpfDataplane {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        // 1. Sweep residue left by the previous process, then retry the closed
        //    reverse-NAT debt (R-61): a key that still fails stays in the set.
        self.cleanup_all_gateway_intents()?;
        // 2. A deadline that has already elapsed is the typed timeout; nothing
        //    has been registered, so no residue remains.
        let remaining = deadline.saturating_duration_since(clock.now());
        if remaining.is_zero() {
            return Err(GatewayConnectDataplaneError::ProbeTimeout);
        }
        // 3. The sentinel connect: an unprogrammed Service key. The registered
        //    selection hook denies it and records a `NoBackend` receipt.
        let sentinel = sentinel_service_key()?;
        let (socket, intent) = self.open_upstream(sentinel)?.into_parts();
        let target = std::net::SocketAddr::from((SENTINEL_ADDRESS, SENTINEL_PORT));
        let outcome = {
            let connect = socket.connect(target);
            tokio::pin!(connect);
            let timeout = clock.sleep(remaining);
            tokio::pin!(timeout);
            tokio::select! {
                result = &mut connect => Some(result),
                () = &mut timeout => None,
            }
        };
        let verdict = match outcome {
            None => Err(GatewayConnectDataplaneError::ProbeTimeout),
            // The sentinel is never programmed: a completed connect means the
            // hook did not deny it.
            Some(Ok(_stream)) => Err(GatewayConnectDataplaneError::Unavailable),
            Some(Err(_denied)) => match self.take_receipt(&intent) {
                Ok(GatewaySelectionReceipt::NoBackend { .. }) => Ok(()),
                Ok(GatewaySelectionReceipt::Selected { .. }) => {
                    Err(GatewayConnectDataplaneError::ReceiptMismatch)
                }
                Err(error) => Err(error),
            },
        };
        let cleaned = self.cleanup(&intent);
        verdict.and(cleaned)
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        let intent_value = intent_pod(&key)?;
        let socket = prepared_socket()?;
        let socket_cookie =
            SocketCookieReader::new().read(socket.as_fd()).map_err(|error| match error {
                SocketCookieReadError::Unsupported => {
                    GatewayConnectDataplaneError::CookieRead(std::io::ErrorKind::Unsupported)
                }
                SocketCookieReadError::Syscall { source } => {
                    GatewayConnectDataplaneError::CookieRead(source.kind())
                }
                SocketCookieReadError::Zero => {
                    GatewayConnectDataplaneError::CookieRead(std::io::ErrorKind::InvalidData)
                }
            })?;
        let raw_cookie = socket_cookie.get().get();

        // Registration rejects a live cookie, drops a stale receipt for it, then
        // inserts the intent.
        let mut intents = self.gateway_intent_map.lock();
        match intents.get(&raw_cookie, 0) {
            Ok(_) => return Err(GatewayConnectDataplaneError::IntentAlreadyRegistered),
            Err(aya::maps::MapError::KeyNotFound) => {}
            Err(_) => return Err(GatewayConnectDataplaneError::Kernel),
        }
        remove_key(&mut self.gateway_receipt_map.lock(), raw_cookie)?;
        intents.insert(raw_cookie, intent_value, 0).map_err(|error| {
            if is_map_full(&error) {
                GatewayConnectDataplaneError::IntentRegistryFull
            } else {
                GatewayConnectDataplaneError::Kernel
            }
        })?;
        drop(intents);

        Ok(PreparedUpstream::new(socket, GatewayConnectIntent { socket_cookie, service_key: key }))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let cookie = intent.socket_cookie.get().get();
        let mut receipts = self.gateway_receipt_map.lock();
        let value = match receipts.get(&cookie, 0) {
            Ok(value) => value,
            Err(aya::maps::MapError::KeyNotFound) => {
                return Err(GatewayConnectDataplaneError::ReceiptMissing);
            }
            Err(_) => return Err(GatewayConnectDataplaneError::Kernel),
        };
        let receipt = decode_receipt(value, intent.socket_cookie, intent.service_key)?;
        match receipts.remove(&cookie) {
            Ok(()) => Ok(receipt),
            Err(error) if is_key_absent(&error) => Ok(receipt),
            Err(_) => Err(GatewayConnectDataplaneError::Kernel),
        }
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        // The read waits for any `update_service` between its outer swap and its
        // `Applied` publication, so an id a receipt names always resolves.
        let _published = self.service_apply_commit.read();
        match self.backend_identities.lock().get(&backend_id.get()) {
            Some(BackendIdentityState::Applied { spiffe_id }) => Ok(spiffe_id.clone()),
            Some(BackendIdentityState::Reserved { .. }) | None => {
                Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
            }
        }
    }
}
