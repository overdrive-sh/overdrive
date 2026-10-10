//! Guest transport token and setup — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Guest adaptation contract — `overdrive-init` + guest programs (D6,
//! D10, D15)" and § DG-5 ("Transport token and setup", module `transport`).
//! Every item below is the one those sections pin; the binary's pre-READY path
//! (`complete_pre_ready_init`) calls `parse_transport_token` and
//! `configure_guest_transport` (DG-2 row 16, replacing `overdrive.net=`).
//!
//! RED scaffold (DISTILL, Mandate 7): `parse_transport_token`,
//! `configure_guest_transport` and the production `KernelGuestTransportOps`
//! panic until DELIVER vsock new-step 5 lands them. The error enum with its
//! pinned `Display` texts and the label enums with their `as_str` are plain
//! data and are written for real.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-step 5).
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "RED scaffold — DELIVER vsock new-step 5 wires it into main")
)]
#![cfg_attr(
    test,
    allow(dead_code, reason = "RED scaffold — DELIVER vsock new-step 5 wires it into main")
)]
#![allow(
    clippy::redundant_pub_crate,
    reason = "DG-5 pins `pub(crate)` visibility for these binary-crate items"
)]
#![allow(
    clippy::doc_markdown,
    reason = "protocol leg names (V_g, T_g, Q_g, …) and kernel names are prose, not code"
)]

#[cfg(test)]
mod tests;

use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::RangeInclusive;

use overdrive_core::guest_transport::LaunchToken;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The parsed `overdrive.transport=` kernel command-line token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestTransportToken {
    /// `addr=`: the allocation's workload address.
    pub(crate) workload_addr: Ipv4Addr,
    /// `dns=`: the guest gateway (resolver `nameserver`).
    pub(crate) gateway: Ipv4Addr,
    /// `launch=`: the assignment's launch token (ATTR-TOKEN).
    pub(crate) launch: LaunchToken,
}

/// Parses the one `overdrive.transport=` token of `cmdline` (§ *Guest
/// adaptation contract*). Fields are positional: `vsock2`, `addr=`, `dns=`,
/// `launch=`. The first position whose key or value is invalid names the
/// field (`Version`, `Addr`, `Dns`, `Launch`); a missing trailing field names
/// the first missing one; a fifth field is `Extra`. Other kernel parameters
/// are ignored.
pub(crate) fn parse_transport_token(
    _cmdline: &str,
) -> Result<GuestTransportToken, GuestTransportSetupError> {
    panic!("{SCAFFOLD}")
}

/// The setup steps of G-V7, one method per step; each `Err` maps to exactly
/// one `GuestTransportSetupError` variant (named per method). Stateful: the
/// production ops keep every socket they bind and every program they load,
/// and hand them to the owner's seam (`KernelGuestOwnerIo::new(ops)`).
pub(crate) trait GuestTransportOps {
    /// `Err` → `GuestTransportSetupError::DummyDevice`.
    fn create_dummy_device(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::AddressAdd`.
    fn add_workload_address(&mut self, addr: Ipv4Addr) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::FwmarkRule`.
    fn add_fwmark_rule(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::FwmarkReflect`.
    fn enable_fwmark_reflect(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::ReservedPorts`.
    fn reserve_ports(&mut self, ports: RangeInclusive<u16>) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::IntakeBind { addr }`.
    fn bind_tcp_intake(&mut self, addr: SocketAddrV4) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::IntakeBind { addr }`.
    fn bind_udp_slot(&mut self, addr: SocketAddrV4) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::KernelLayoutMismatch`.
    fn check_sock_common_layout(&mut self) -> Result<(), SockCommonField>;
    /// `Err` → `GuestTransportSetupError::ProgramLoad`.
    fn load_program(&mut self, program: GuestProgram) -> Result<(), aya::EbpfError>;
    /// `Err` → `GuestTransportSetupError::ProgramAttach`.
    fn attach_program(&mut self, program: GuestProgram) -> Result<(), aya::programs::ProgramError>;
    /// Inserts every bound UDP slot socket into the slot SockHash (the
    /// intake model's "SockHash-installed from boot").
    /// `Err` → `GuestTransportSetupError::SlotInstall`.
    fn install_udp_slots(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::ListenerSeed`.
    fn seed_listener_map(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::InboundListenerBind`.
    fn bind_inbound_vsock(&mut self) -> std::io::Result<()>;
}

/// Runs the steps in this order and stops at the first failure: dummy
/// device, workload address, fwmark rule, `fwmark_reflect`, reserved ports,
/// TCP intake, every UDP slot (n ascending), layout check, each
/// `GuestProgram` load then attach (enum order), slot install, listener
/// seed, inbound listener. Today's `lo` bring-up and resolver write keep
/// their place and error handling.
pub(crate) fn configure_guest_transport(
    _token: &GuestTransportToken,
    _ops: &mut dyn GuestTransportOps,
) -> Result<(), GuestTransportSetupError> {
    panic!("{SCAFFOLD}")
}

/// The production [`GuestTransportOps`] over the guest kernel (netlink, sysctl,
/// sockets, aya). Keeps every socket it binds and every program it loads for
/// `KernelGuestOwnerIo::new`.
pub(crate) struct KernelGuestTransportOps {
    _scaffold: (),
}

impl KernelGuestTransportOps {
    /// Production ops holding nothing yet: every socket and program is
    /// acquired by the setup steps.
    pub(crate) const fn new() -> Self {
        Self { _scaffold: () }
    }
}

impl GuestTransportOps for KernelGuestTransportOps {
    fn create_dummy_device(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn add_workload_address(&mut self, _addr: Ipv4Addr) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn add_fwmark_rule(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn enable_fwmark_reflect(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn reserve_ports(&mut self, _ports: RangeInclusive<u16>) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn bind_tcp_intake(&mut self, _addr: SocketAddrV4) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn bind_udp_slot(&mut self, _addr: SocketAddrV4) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn check_sock_common_layout(&mut self) -> Result<(), SockCommonField> {
        panic!("{SCAFFOLD}")
    }

    fn load_program(&mut self, _program: GuestProgram) -> Result<(), aya::EbpfError> {
        panic!("{SCAFFOLD}")
    }

    fn attach_program(
        &mut self,
        _program: GuestProgram,
    ) -> Result<(), aya::programs::ProgramError> {
        panic!("{SCAFFOLD}")
    }

    fn install_udp_slots(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn seed_listener_map(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }

    fn bind_inbound_vsock(&mut self) -> std::io::Result<()> {
        panic!("{SCAFFOLD}")
    }
}

/// Why `overdrive-init` exits before READY (G-V7). Rendered to the console as
/// `overdrive-init: guest transport setup failed: <Display>`; the host sees the
/// existing "guest exited before READY" start failure.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GuestTransportSetupError {
    /// The kernel command line carries no `overdrive.transport=` token.
    #[error("kernel command line has no overdrive.transport= token")]
    TokenMissing,
    /// The kernel command line carries more than one token.
    #[error("kernel command line has more than one overdrive.transport= token")]
    TokenDuplicate,
    /// The first invalid (or first missing, or a fifth) field of the token.
    #[error("overdrive.transport= field {} is malformed", field.as_str())]
    TokenMalformed {
        /// The field named.
        field: TransportTokenField,
    },
    /// Creating the dummy device failed.
    #[error("creating the transport dummy device failed: {source}")]
    DummyDevice {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Adding `workload_addr/32` to the dummy device failed.
    #[error("adding the workload address failed: {source}")]
    AddressAdd {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Adding `ip rule fwmark <platform mark> lookup local` failed.
    #[error("adding the fwmark lookup rule failed: {source}")]
    FwmarkRule {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Setting `net.ipv4.fwmark_reflect=1` failed.
    #[error("setting net.ipv4.fwmark_reflect failed: {source}")]
    FwmarkReflect {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// The guest kernel's `sock_common` layout does not match the programs.
    #[error("guest kernel sock_common field {} does not match the guest programs", field.as_str())]
    KernelLayoutMismatch {
        /// The mismatching field.
        field: SockCommonField,
    },
    /// Loading a guest program failed.
    #[error("loading guest program {} failed: {source}", program.as_str())]
    ProgramLoad {
        /// The program.
        program: GuestProgram,
        /// aya's error.
        source: aya::EbpfError,
    },
    /// Attaching a guest program failed.
    #[error("attaching guest program {} failed: {source}", program.as_str())]
    ProgramAttach {
        /// The program.
        program: GuestProgram,
        /// aya's error.
        source: aya::programs::ProgramError,
    },
    /// A sock_diag dump failed (incl. NLMSG_ERROR when inet_diag/tcp_diag
    /// are missing) — never treated as an empty result.
    #[error("seeding the listener map from sock_diag failed: {source}")]
    ListenerSeed {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Binding the inbound vsock listener on `GUEST_INBOUND_PORT` failed.
    #[error("binding the inbound vsock listener failed: {source}")]
    InboundListenerBind {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// D25: reserving `guest_reserved_ports()` failed.
    #[error("reserving the guest transport ports failed: {source}")]
    ReservedPorts {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// D25: binding an intake on a reserved port failed.
    #[error("binding the guest intake on {addr} failed: {source}")]
    IntakeBind {
        /// The intake address.
        addr: SocketAddrV4,
        /// The kernel's error.
        source: std::io::Error,
    },
    /// DG-5: inserting the bound UDP slot sockets into the slot SockHash failed.
    #[error("installing the UDP slot sockets failed: {source}")]
    SlotInstall {
        /// The kernel's error.
        source: std::io::Error,
    },
}

/// The token field a `TokenMalformed` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportTokenField {
    /// Position 1: `vsock2`.
    Version,
    /// Position 2: `addr=`.
    Addr,
    /// Position 3: `dns=`.
    Dns,
    /// Position 4: `launch=`.
    Launch,
    /// A fifth field.
    Extra,
}

impl TransportTokenField {
    /// The field's lower snake case label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Addr => "addr",
            Self::Dns => "dns",
            Self::Launch => "launch",
            Self::Extra => "extra",
        }
    }
}

/// The `sock_common` fields the listen programs read, checked against the
/// guest kernel's BTF at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SockCommonField {
    /// `skc_rcv_saddr`.
    RcvSaddr,
    /// `skc_num`.
    Num,
    /// `skc_family`.
    Family,
}

impl SockCommonField {
    /// The field's lower snake case label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RcvSaddr => "rcv_saddr",
            Self::Num => "num",
            Self::Family => "family",
        }
    }
}

/// The guest programs, in load-and-attach order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuestProgram {
    /// `cgroup/connect4`: rewrites captured connects to the intakes.
    Connect4,
    /// `cgroup/sendmsg4`.
    Sendmsg4,
    /// `cgroup/getpeername4`.
    Getpeername4,
    /// `cgroup/recvmsg4`.
    Recvmsg4,
    /// `sock_release`: reports released sockets.
    SockRelease,
    /// `sock_ops`.
    SockOps,
    /// The SockHash verdict program.
    Verdict,
    /// The stream parser.
    StreamParser,
    /// The guest-`lo` egress unframe.
    UnframeLo,
    /// The drain counter.
    DrainCounter,
    /// `fexit` on `inet_csk_listen_start` (D23).
    ListenStart,
    /// `fexit` on `inet_csk_listen_stop` (D23).
    ListenStop,
}

impl GuestProgram {
    /// The program's lower snake case label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Connect4 => "connect4",
            Self::Sendmsg4 => "sendmsg4",
            Self::Getpeername4 => "getpeername4",
            Self::Recvmsg4 => "recvmsg4",
            Self::SockRelease => "sock_release",
            Self::SockOps => "sock_ops",
            Self::Verdict => "verdict",
            Self::StreamParser => "stream_parser",
            Self::UnframeLo => "unframe_lo",
            Self::DrainCounter => "drain_counter",
            Self::ListenStart => "listen_start",
            Self::ListenStop => "listen_stop",
        }
    }
}
