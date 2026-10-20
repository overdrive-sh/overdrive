//! Guest transport token and setup — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Guest adaptation contract — `overdrive-init` + guest programs" and
//! § DG-5 ("Transport token and setup", module `transport`), with the guest
//! kit's forwarder module (EVMOD-LOAD, EVMOD-GUEST-KIT) and its context
//! (EVMOD-CTL, EVMOD-LIFE). Every item below is the one those sections pin; the
//! binary's pre-READY path (`complete_pre_ready_init`) calls
//! `parse_transport_token` and `configure_guest_transport` (DG-2 row 16,
//! replacing `overdrive.net=`).
//!
//! RED scaffold (DISTILL, Mandate 7): `parse_transport_token`,
//! `configure_guest_transport`, the [`KernelRelease`] codec and the production
//! `KernelGuestTransportOps` panic until DELIVER step 5 lands them. The error
//! enum with its pinned `Display` texts and the label enums with their
//! `as_str` are plain data and are written for real.

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER step 5).
#![cfg_attr(not(test), expect(dead_code, reason = "RED scaffold — DELIVER step 5 lands it"))]
#![cfg_attr(test, allow(dead_code, reason = "RED scaffold — DELIVER step 5 lands it"))]
#![allow(
    clippy::redundant_pub_crate,
    reason = "DG-5 pins `pub(crate)` visibility for these binary-crate items"
)]
#![allow(
    clippy::doc_markdown,
    reason = "protocol leg names (V_g, …) and kernel names are prose, not code"
)]

// In-guest real-kernel cells (D-UP-18): the native metal microVM lane only.
#[cfg(all(test, feature = "integration-tests", feature = "kvm-tests"))]
mod real_kernel;
#[cfg(test)]
mod tests;

use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;

use overdrive_core::guest_transport::LaunchToken;
use overdrive_core::guest_transport::forwarder::ForwarderError;
use overdrive_core::vm::config::VsockPort;

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
    /// `beacon=`, `stream=`, `control=` (GUEST-PORTS, ADR-0217).
    pub(crate) ports: GuestHostPorts,
}

/// GUEST-PORTS (ADR-0217): the host listener ports this boot connects to.
/// Built only by `parse_transport_token`; the three differ and each lies in
/// 1024..=u32::MAX - 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestHostPorts {
    /// `beacon=`: the node's beacon intake port (READY's `port=`).
    pub(crate) beacon: VsockPort,
    /// `stream=`: the guest-flow stream listener port.
    pub(crate) stream: VsockPort,
    /// `control=`: the guest-flow control listener port.
    pub(crate) control: VsockPort,
}

/// Parses the one `overdrive.transport=` token of `cmdline` (§ *Guest
/// adaptation contract*). Fields are positional: `vsock2`, `addr=`, `dns=`,
/// `launch=`, `beacon=`, `stream=`, `control=`. The first position whose key
/// or value is invalid names the field (`Version`, `Addr`, `Dns`, `Launch`,
/// `Beacon`, `Stream`, `Control`); a port value is invalid when it is not a
/// decimal `u32`, lies in 0..=1023, is `u32::MAX` (`VMADDR_PORT_ANY`) or
/// equals an earlier port field (the later field is named). A missing
/// trailing field names the first missing one; an eighth field is `Extra`.
/// Other kernel parameters are ignored.
pub(crate) fn parse_transport_token(
    _cmdline: &str,
) -> Result<GuestTransportToken, GuestTransportSetupError> {
    panic!("{SCAFFOLD}")
}

/// The setup steps of G-V7, one method per step; each `Err` maps to exactly
/// one `GuestTransportSetupError` variant (named per method). Stateful: the
/// production ops keep every socket they bind, every program they load and
/// the forwarder context they open, and hand them to the owner's seam
/// (`KernelGuestOwnerIo::new(ops)`).
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
    /// `Err` → `GuestTransportSetupError::ProgramLoad`.
    fn load_program(&mut self, program: GuestProgram) -> Result<(), aya::EbpfError>;
    /// `Err` → `GuestTransportSetupError::ProgramAttach`.
    fn attach_program(&mut self, program: GuestProgram) -> Result<(), aya::programs::ProgramError>;
    /// EVMOD-GUEST-KIT: reads the release of the module at
    /// `GUEST_FORWARDER_MODULE_PATH` and of the running kernel, and on a match
    /// calls `finit_module`. The three typed outcomes are the three
    /// `ForwarderModule*` variants; an `EEXIST` (already loaded) is Ok.
    fn load_forwarder_module(&mut self) -> Result<(), GuestTransportSetupError>;
    /// `KernelFlowForwarder::open()` and its `DeviceOpen` / `Version` probe
    /// stages; keeps the context. `Err` → `GuestTransportSetupError::Forwarder`.
    fn open_forwarder(&mut self) -> Result<(), ForwarderError>;
    /// `Err` → `GuestTransportSetupError::InboundListenerBind`.
    fn bind_inbound_vsock(&mut self) -> std::io::Result<()>;
}

/// Runs the steps in this order and stops at the first failure: dummy
/// device, workload address, fwmark rule, `fwmark_reflect`, reserved ports,
/// TCP intake, every UDP slot (n ascending), each `GuestProgram` load then
/// attach (enum order), forwarder module, forwarder context, inbound
/// listener. Today's `lo` bring-up and resolver write keep their place and
/// error handling.
pub(crate) fn configure_guest_transport(
    _token: &GuestTransportToken,
    _ops: &mut dyn GuestTransportOps,
) -> Result<(), GuestTransportSetupError> {
    panic!("{SCAFFOLD}")
}

/// The production [`GuestTransportOps`] over the guest kernel (netlink,
/// sysctl, sockets, aya, `finit_module`, `/dev/ovd_fwd`). Keeps every socket
/// it binds, every program it loads and the forwarder context it opens for
/// `KernelGuestOwnerIo::new`.
pub(crate) struct KernelGuestTransportOps {
    _scaffold: (),
}

impl KernelGuestTransportOps {
    /// Production ops holding nothing yet: every socket, program and the
    /// forwarder context are acquired by the setup steps.
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

    fn load_program(&mut self, _program: GuestProgram) -> Result<(), aya::EbpfError> {
        panic!("{SCAFFOLD}")
    }

    fn attach_program(
        &mut self,
        _program: GuestProgram,
    ) -> Result<(), aya::programs::ProgramError> {
        panic!("{SCAFFOLD}")
    }

    fn load_forwarder_module(&mut self) -> Result<(), GuestTransportSetupError> {
        panic!("{SCAFFOLD}")
    }

    fn open_forwarder(&mut self) -> Result<(), ForwarderError> {
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
    /// EVMOD-GUEST-KIT: no module file at `GUEST_FORWARDER_MODULE_PATH`.
    #[error(
        "the guest forwarder module {} is missing; the VM's kernel and root filesystem must \
         come from one guest kit",
        path.display()
    )]
    ForwarderModuleMissing {
        /// The module path.
        path: PathBuf,
    },
    /// EVMOD-GUEST-KIT: the module was built for another kernel release; the
    /// message names both and says that the VM's kernel and root filesystem
    /// must come from one guest kit.
    #[error(
        "the guest forwarder module was built for kernel {module}, but the guest runs \
         {kernel}; the VM's kernel and root filesystem must come from one guest kit"
    )]
    ForwarderModuleRelease {
        /// The release the module was built for (its `vermagic`'s first word).
        module: KernelRelease,
        /// The running kernel's release (`uname(2)`).
        kernel: KernelRelease,
    },
    /// EVMOD-GUEST-KIT: any other refusal of `finit_module` (a same-release
    /// build the kernel refuses by `vermagic` or a CRC, a signature refusal
    /// under enforced module signing).
    #[error("loading the guest forwarder module failed: {source}")]
    ForwarderModuleLoad {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// The forwarder context could not be opened or failed its `DeviceOpen` /
    /// `Version` probe stages (EVMOD-CTL, EVMOD-LIFE).
    #[error("opening the guest forwarder failed: {source}")]
    Forwarder {
        /// The forwarder's error.
        source: ForwarderError,
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
    /// Position 5: `beacon=` (GUEST-PORTS).
    Beacon,
    /// Position 6: `stream=` (GUEST-PORTS).
    Stream,
    /// Position 7: `control=` (GUEST-PORTS).
    Control,
    /// An eighth field.
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
            Self::Beacon => "beacon",
            Self::Stream => "stream",
            Self::Control => "control",
            Self::Extra => "extra",
        }
    }
}

/// The guest capture programs, in load-and-attach order.
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
}

impl GuestProgram {
    /// Every program, in load-and-attach (enum) order.
    pub(crate) const ALL: [Self; 5] =
        [Self::Connect4, Self::Sendmsg4, Self::Getpeername4, Self::Recvmsg4, Self::SockRelease];

    /// The program's lower snake case label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Connect4 => "connect4",
            Self::Sendmsg4 => "sendmsg4",
            Self::Getpeername4 => "getpeername4",
            Self::Recvmsg4 => "recvmsg4",
            Self::SockRelease => "sock_release",
        }
    }
}

/// A Linux kernel release string (`UTS_RELEASE`): non-empty, at most 64 bytes
/// (`__NEW_UTS_LEN`, `include/uapi/linux/utsname.h:28 @ v7.0`), no whitespace
/// or NUL. `overdrive-init`-private (EVMOD-GUEST-KIT): the release of the
/// forwarder module (the first word of its `vermagic`) and of the running
/// kernel (`uname(2)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KernelRelease(String);

/// The release as written.
impl fmt::Display for KernelRelease {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Validated: non-empty, at most 64 bytes, no whitespace or NUL.
impl FromStr for KernelRelease {
    type Err = KernelReleaseError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a string is not a [`KernelRelease`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum KernelReleaseError {
    /// The empty string.
    #[error("kernel release is empty")]
    Empty,
    /// Longer than `__NEW_UTS_LEN` (64) bytes.
    #[error("kernel release is longer than 64 bytes")]
    TooLong,
    /// Contains whitespace or NUL.
    #[error("kernel release contains whitespace or NUL")]
    InvalidCharacter,
}
