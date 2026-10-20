//! Guest-prefix steering — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — guest-prefix steering
//! `overdrive-dataplane::guest_steering` (D8a-LOOKUP, ADR-0171)". Every public
//! item below is the one that section pins.
//!
//! Boot steering has two kernel steps (D8a-LOAD-SWAP): `load` (the kernel
//! verifier accepts this binary's program, attached nowhere) and `attach` (the
//! program is swapped into the pinned link, or a first link is attached and
//! pinned). The type states keep that order: `load` returns an unattached
//! [`GuestSteeringCandidate`]; only its `attach` returns a
//! [`GuestPrefixSteering`]; only that type has `steer`.
//!
//! Removed mechanism: guest-prefix steering was withdrawn by the 2026-10-11
//! rulings (feature-delta § "Bridge-era code for DELIVER to delete", list C,
//! the whole module). No DELIVER step implements these bodies; they panic
//! until DELIVER deletes the module. The tests that defended it were deleted
//! with the mechanism (deletion discipline).

// SCAFFOLD: true — netns-density-295 DISTILL (list C: deleted by DELIVER, never implemented).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lints self-trip when DELIVER lands the bodies"
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::BorrowedFd;
use std::path::PathBuf;

use ipnet::Ipv4Net;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The host root network namespace's guest-prefix steering.
///
/// One `sk_lookup` program, its netns link (pinned under
/// `GUEST_PREFIX_STEERING_PIN_DIR`) and that program's socket map (not
/// pinned). Node infrastructure
/// (ADR-0171): no method detaches, unpins or clears it; dropping the handle
/// closes only this process's descriptors, and the link pin keeps the link,
/// its program and that program's map alive.
/// Constructed only by `GuestSteeringCandidate::attach`.
pub struct GuestPrefixSteering {
    _scaffold: (),
}

/// This binary's steering program, accepted by the kernel verifier.
///
/// It has its own new, empty socket map, is loaded for one prefix and
/// attached nowhere, and carries the pinned link if one exists. Not `Clone`. Dropping it frees the
/// program and the map; nothing attached changes.
pub struct GuestSteeringCandidate {
    _scaffold: (),
}

/// Where the steering's netns link is pinned. The map's `max_entries` is
/// `overdrive_core::guest_transport::GUEST_INTAKE_STEERING_MAX` (262,144;
/// D8a-CAP); no second constant exists.
pub const GUEST_PREFIX_STEERING_PIN_DIR: &str = "/sys/fs/bpf/overdrive/guest_prefix_steering";

/// Key length of the steering socket map (G-PL-1, DG-14).
///
/// The map is a kernel object the `prefix_landing` conformance test reads
/// independently, so its layout is a contract: an 8-byte key and
/// `value_size = 8` (a dump reports the socket cookie, host byte order).
pub const GUEST_STEERING_KEY_LEN: usize = 8;

/// The one encoder of a steering map key (G-PL-1, DG-14).
///
/// Bytes 0..4 the IPv4 address in network order, 4..6 the declared port in
/// network order, 6..8 zero. The steering program and the conformance test
/// reader use it.
pub fn guest_steering_key(_intake: SocketAddrV4) -> [u8; GUEST_STEERING_KEY_LEN] {
    panic!("{SCAFFOLD}")
}

impl GuestPrefixSteering {
    /// Boot step 1 (Load). Loads this binary's program for `prefix`,
    /// excluding `gateway`, with a new, empty socket map of
    /// `GUEST_INTAKE_STEERING_MAX` entries; the kernel verifier checks it
    /// here (a rejection is `Load`). Opens the pinned link, if any, and
    /// carries it in the candidate for `attach`; no pin → the candidate will
    /// attach a new link. Attaches, replaces and pins nothing: on any return
    /// the attached steering (if any) and the route are unchanged.
    pub fn load(
        _prefix: Ipv4Net,
        _gateway: Ipv4Addr,
    ) -> Result<GuestSteeringCandidate, GuestSteeringError> {
        panic!("{SCAFFOLD}")
    }

    /// Makes `listener` the socket every new host-local TCP connection to
    /// `intake` — the workload address and the *declared* port — reaches.
    /// Preconditions: `intake.ip()` in the prefix and not the gateway
    /// (`IntakeOutsideGuestPrefix`); `listener` is a listening TCPv4 socket
    /// bound to `intake.ip()` on any port (INTAKE-BIND: the owner binds
    /// `workload_addr:guest_intake_host_port(i)`, a port of the intake host
    /// block, not `intake.port()`) (`UnsupportedSocket` otherwise);
    /// the caller has admitted `intake`'s port (D8a-CAP), so the map has room.
    /// The program assigns by entry, whatever port the listener is bound to
    /// (P-40); a connection to the listener's own bound port is decided like
    /// any other prefix lookup — by its own entry, if one exists, else dropped.
    /// `Ok`: the entry is present in the kernel now; repeating it for the same
    /// listener is `Ok` with no change. An entry naming another open listener
    /// → `IntakeAlreadySteered`, no change. `KernelMemory`: the kernel could
    /// not allocate the entry or the listener's psock; the entry is absent.
    /// Any other `Err`: the entry is absent; with the preconditions held it
    /// is an Overdrive defect. While the entry is absent, connections to
    /// `intake` are refused. There is no removal method: closing `listener`
    /// removes the entry, and frees its slot, in the same kernel step.
    pub fn steer(
        &self,
        _intake: SocketAddrV4,
        _listener: BorrowedFd<'_>,
    ) -> Result<(), GuestSteeringError> {
        panic!("{SCAFFOLD}")
    }
}

impl GuestSteeringCandidate {
    /// Boot step 2 (Swap). With an adopted link: `BPF_LINK_UPDATE` replaces
    /// the link's program with this one — atomic: every lookup runs the
    /// earlier program or this one; on `Err(Update)` the link still runs the
    /// earlier program, unchanged (K-L9, P-40: every failing update observed
    /// left the earlier program). The update is issued without
    /// `BPF_F_REPLACE`: only this boot writes the link (A-31). With no pinned
    /// link: attaches a new link to the root namespace, then pins it;
    /// `Err(Attach)` attaches nothing, and on `Err(Pin)` the unpinned link
    /// detaches when this process's descriptor of it closes (on return).
    /// Never detaches a pinned link. Crash at any point: the
    /// link runs the earlier program or this one, or (first attach, before
    /// the pin) no link remains.
    pub fn attach(self) -> Result<GuestPrefixSteering, GuestSteeringError> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a steering step failed.
#[derive(Debug, thiserror::Error)]
pub enum GuestSteeringError {
    /// From `load`; includes the kernel verifier rejecting the program.
    /// Nothing is attached.
    #[error("loading the guest-prefix steering program failed")]
    Load {
        /// The load error.
        #[source]
        source: aya::EbpfError,
    },
    /// From `attach` with no pinned link: the first attach failed; nothing
    /// is attached.
    #[error("attaching the guest-prefix steering link failed")]
    Attach {
        /// The attach error.
        #[source]
        source: aya::programs::ProgramError,
    },
    /// From `attach` with an adopted link: `BPF_LINK_UPDATE` failed (the
    /// kernel's errno); the link still runs its earlier program (K-L9). aya
    /// 0.13.1 has no link-update call for `SkLookup` links, so the adapter
    /// issues `bpf(BPF_LINK_UPDATE)` itself (STEER-PINS); the source is
    /// that call's error.
    #[error("updating the pinned guest-prefix steering link failed")]
    Update {
        /// The `bpf(BPF_LINK_UPDATE)` error.
        #[source]
        source: std::io::Error,
    },
    /// From a first `attach`: the new link is not pinned and detaches when
    /// its descriptor closes.
    #[error("pinning the guest-prefix steering link at {path} failed")]
    Pin {
        /// The pin path.
        path: PathBuf,
        /// The pin error.
        #[source]
        source: std::io::Error,
    },
    /// The intake address is outside the guest prefix or is the gateway.
    #[error("intake {intake} is outside the guest prefix or is the gateway")]
    IntakeOutsideGuestPrefix {
        /// The intake.
        intake: SocketAddrV4,
    },
    /// The intake is already steered to another open listener.
    #[error("intake {intake} is already steered to another open listener")]
    IntakeAlreadySteered {
        /// The intake.
        intake: SocketAddrV4,
    },
    /// The descriptor is not a listening TCPv4 socket bound to the intake's
    /// address.
    #[error("socket is not a listening TCPv4 socket bound to the intake's address")]
    UnsupportedSocket,
    /// From `steer`: the kernel could not allocate the map entry or the
    /// listener's psock (`ENOMEM`). The only steer failure the owner retries
    /// (D8a-CAP, D8a-CAP-RETRY).
    #[error("the kernel could not allocate the guest-prefix steering entry")]
    KernelMemory {
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// From `steer`: any other map update failure. With admission holding
    /// the capacity (D8a-CAP) and the preconditions held, an Overdrive
    /// defect; not retried.
    #[error("updating the guest-prefix steering map failed")]
    MapUpdate {
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// From `steer`: reading the entry already present for `intake` (to
    /// tell this listener from another, `IntakeAlreadySteered`) failed for
    /// a cause other than its absence. An Overdrive defect; not retried.
    #[error("reading the guest-prefix steering map failed")]
    MapLookup {
        /// The map error.
        #[source]
        source: aya::maps::MapError,
    },
    /// From `load`: opening the pinned link under
    /// `GUEST_PREFIX_STEERING_PIN_DIR` failed for a cause other than its
    /// absence (nothing attached). From `steer`: reading the listener's
    /// socket cookie or socket options failed (an Overdrive defect; not
    /// retried).
    #[error(
        "opening the pinned steering link or reading a listener's socket cookie or options failed"
    )]
    Query {
        /// The I/O error.
        #[source]
        source: std::io::Error,
    },
}
