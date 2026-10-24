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
//! `parse_transport_token` validates the boot token and
//! `configure_guest_transport` applies its ordered setup through
//! [`GuestTransportOps`]. [`KernelGuestTransportOps`] provides the production
//! network, capture-program, module, and forwarder operations. The typed
//! failures retain the stages pinned by the feature delta.

#![cfg_attr(test, allow(dead_code, reason = "test scenarios use only relevant setup projections"))]
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
use overdrive_core::guest_transport::{
    GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE, GUEST_UDP_SLOTS, guest_reserved_ports,
};
use overdrive_core::vm::config::VsockPort;

pub(crate) const GUEST_TPROXY_MARK: u32 = 0x295;

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
    cmdline: &str,
) -> Result<GuestTransportToken, GuestTransportSetupError> {
    let mut tokens = cmdline
        .split_ascii_whitespace()
        .filter_map(|part| part.strip_prefix("overdrive.transport="));
    let token = tokens.next().ok_or(GuestTransportSetupError::TokenMissing)?;
    if tokens.next().is_some() {
        return Err(GuestTransportSetupError::TokenDuplicate);
    }

    let fields: Vec<&str> = token.split(',').collect();
    let required = |index: usize, key: &str, field: TransportTokenField| {
        let value =
            fields.get(index).copied().ok_or(GuestTransportSetupError::TokenMalformed { field })?;
        if index == 0 {
            return Ok(value);
        }
        if value.starts_with(&format!("{key}=")) {
            Ok(value)
        } else {
            Err(GuestTransportSetupError::TokenMalformed { field })
        }
    };
    if required(0, "", TransportTokenField::Version)? != "vsock2" {
        return Err(GuestTransportSetupError::TokenMalformed {
            field: TransportTokenField::Version,
        });
    }
    let workload_addr = required(1, "addr", TransportTokenField::Addr)?
        .strip_prefix("addr=")
        .and_then(|value| value.parse::<Ipv4Addr>().ok())
        .ok_or(GuestTransportSetupError::TokenMalformed { field: TransportTokenField::Addr })?;
    let gateway = required(2, "dns", TransportTokenField::Dns)?
        .strip_prefix("dns=")
        .and_then(|value| value.parse::<Ipv4Addr>().ok())
        .ok_or(GuestTransportSetupError::TokenMalformed { field: TransportTokenField::Dns })?;
    let launch = required(3, "launch", TransportTokenField::Launch)?
        .strip_prefix("launch=")
        .and_then(|value| LaunchToken::from_str(value).ok())
        .ok_or(GuestTransportSetupError::TokenMalformed { field: TransportTokenField::Launch })?;

    let mut port_values = [0u32; 3];
    for (index, (key, field)) in [
        ("beacon", TransportTokenField::Beacon),
        ("stream", TransportTokenField::Stream),
        ("control", TransportTokenField::Control),
    ]
    .into_iter()
    .enumerate()
    {
        let value = required(index + 4, key, field)?
            .strip_prefix(&format!("{key}="))
            .filter(|raw| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|port| (1024..u32::MAX).contains(port))
            .filter(|port| !port_values[..index].contains(port))
            .ok_or(GuestTransportSetupError::TokenMalformed { field })?;
        port_values[index] = value;
    }
    if fields.len() > 7 {
        return Err(GuestTransportSetupError::TokenMalformed { field: TransportTokenField::Extra });
    }
    Ok(GuestTransportToken {
        workload_addr,
        gateway,
        launch,
        ports: GuestHostPorts {
            beacon: VsockPort::new(port_values[0]),
            stream: VsockPort::new(port_values[1]),
            control: VsockPort::new(port_values[2]),
        },
    })
}

/// The setup steps of G-V7, one method per step; each `Err` maps to exactly
/// one `GuestTransportSetupError` variant (named per method). Stateful: the
/// production ops keep every socket they bind, every program they load and
/// the forwarder context they open, and hand them to the owner's seam
/// (`KernelGuestOwnerIo::new(ops)?`), which returns the correlation maps or
/// the existing `ProgramLoad { program: TcpCorrelation, source }` failure.
pub(crate) trait GuestTransportOps {
    /// `Err` → `GuestTransportSetupError::DummyDevice`.
    fn create_dummy_device(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::AddressAdd`.
    fn add_workload_address(&mut self, addr: Ipv4Addr) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::FwmarkRule`.
    fn add_fwmark_rule(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::FwmarkRoute`.
    fn add_fwmark_route(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::TcpFwmarkAccept`.
    fn enable_tcp_fwmark_accept(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::FwmarkReflect`.
    fn enable_fwmark_reflect(&mut self) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::ReservedPorts`.
    fn reserve_ports(&mut self, ports: RangeInclusive<u16>) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::IntakeBind { addr }`.
    fn bind_tcp_intake(&mut self, addr: SocketAddrV4) -> std::io::Result<()>;
    /// `Err` → `GuestTransportSetupError::IntakeBind { addr }`.
    fn bind_udp_slot(&mut self, addr: SocketAddrV4) -> std::io::Result<()>;
    /// Creates and shares the temporary active/child maps across the Rust
    /// SockOps object and C/CO-RE candidate object, then removes both pins
    /// before any program is loaded or attached.
    /// `Err` → the exact `ProgramLoad` variant for the object/setup stage.
    fn prepare_guest_programs(&mut self) -> Result<(), GuestTransportSetupError>;
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
/// device, workload address, GUEST-TRANSPARENT-ROUTE fwmark rule and
/// local-default route, `tcp_fwmark_accept`, `fwmark_reflect`, reserved ports,
/// TCP intake, every UDP slot (n ascending), map/object preparation, each
/// `GuestProgram` load then attach (enum order), forwarder module, forwarder
/// context, inbound listener. Today's `lo` bring-up and resolver write keep
/// their place and error handling.
pub(crate) fn configure_guest_transport(
    token: &GuestTransportToken,
    ops: &mut dyn GuestTransportOps,
) -> Result<(), GuestTransportSetupError> {
    ops.create_dummy_device().map_err(|source| GuestTransportSetupError::DummyDevice { source })?;
    ops.add_workload_address(token.workload_addr)
        .map_err(|source| GuestTransportSetupError::AddressAdd { source })?;
    ops.add_fwmark_rule().map_err(|source| GuestTransportSetupError::FwmarkRule { source })?;
    ops.add_fwmark_route().map_err(|source| GuestTransportSetupError::FwmarkRoute { source })?;
    ops.enable_tcp_fwmark_accept()
        .map_err(|source| GuestTransportSetupError::TcpFwmarkAccept { source })?;
    ops.enable_fwmark_reflect()
        .map_err(|source| GuestTransportSetupError::FwmarkReflect { source })?;
    ops.reserve_ports(guest_reserved_ports())
        .map_err(|source| GuestTransportSetupError::ReservedPorts { source })?;
    let tcp_addr = SocketAddrV4::new(token.workload_addr, GUEST_TCP_INTAKE_PORT);
    ops.bind_tcp_intake(tcp_addr)
        .map_err(|source| GuestTransportSetupError::IntakeBind { addr: tcp_addr, source })?;
    for offset in 0..GUEST_UDP_SLOTS {
        let addr = SocketAddrV4::new(token.workload_addr, GUEST_UDP_SLOT_BASE + offset);
        ops.bind_udp_slot(addr)
            .map_err(|source| GuestTransportSetupError::IntakeBind { addr, source })?;
    }
    ops.prepare_guest_programs()?;
    for program in GuestProgram::ALL {
        ops.load_program(program)
            .map_err(|source| GuestTransportSetupError::ProgramLoad { program, source })?;
        ops.attach_program(program)
            .map_err(|source| GuestTransportSetupError::ProgramAttach { program, source })?;
    }
    ops.load_forwarder_module()?;
    ops.open_forwarder().map_err(|source| GuestTransportSetupError::Forwarder { source })?;
    ops.bind_inbound_vsock()
        .map_err(|source| GuestTransportSetupError::InboundListenerBind { source })?;
    Ok(())
}

mod kernel;
pub(crate) use kernel::KernelGuestTransportOps;

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
    /// Adding the priority-32765 full-mask guest mark rule to table 295 failed.
    #[error("adding the fwmark lookup rule failed: {source}")]
    FwmarkRule {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Adding the table-295 host-scope local-default route on `lo` failed.
    #[error("adding the fwmark local route failed: {source}")]
    FwmarkRoute {
        /// The kernel's error.
        source: std::io::Error,
    },
    /// Setting `net.ipv4.tcp_fwmark_accept=1` failed.
    #[error("setting net.ipv4.tcp_fwmark_accept failed: {source}")]
    TcpFwmarkAccept {
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
    /// `sock_ops`: carries connect identity through the final ACK.
    TcpCorrelation,
    /// `fentry/tcp_check_req`: records the pre-validation Fast Open option.
    TcpCorrelationCandidate,
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
    pub(crate) const ALL: [Self; 7] = [
        Self::TcpCorrelation,
        Self::TcpCorrelationCandidate,
        Self::Connect4,
        Self::Sendmsg4,
        Self::Getpeername4,
        Self::Recvmsg4,
        Self::SockRelease,
    ];

    /// The program's lower snake case label.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TcpCorrelation => "tcp_correlation",
            Self::TcpCorrelationCandidate => "tcp_correlation_candidate",
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validated: non-empty, at most 64 bytes, no whitespace or NUL.
impl FromStr for KernelRelease {
    type Err = KernelReleaseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(KernelReleaseError::Empty);
        }
        if s.len() > 64 {
            return Err(KernelReleaseError::TooLong);
        }
        if s.chars().any(|ch| ch.is_whitespace() || ch == '\0') {
            return Err(KernelReleaseError::InvalidCharacter);
        }
        Ok(Self(s.to_owned()))
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
