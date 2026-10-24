//! P4 (guest `overdrive-init`) — guest network configuration before READY: the
//! transport-token grammar (`launch=`), the `configure_guest_transport` order
//! with stop-at-first-failure (capture programs, then the forwarder module and
//! its context, then the inbound listener), the three `ForwarderModule*`
//! failures as pre-READY start failures, and every pinned
//! `GuestTransportSetupError` `Display` text (G-V7 rows 1/2/4; feature-delta
//! § *Guest adaptation contract*, DG-5 "Transport token and setup",
//! EVMOD-LOAD, EVMOD-GUEST-KIT).
//!
//! Re-authored from the orphaned drafts at `91f9436ff`
//! (`tests/integration/vsock_guest_ready_config.rs`, the token half of
//! `tests/acceptance/vsock_guest_udp_slots.rs`): `overdrive-init` has no
//! library target, so the driving ports (`parse_transport_token`,
//! `configure_guest_transport`) are exercised source-locally, the kernel
//! command line is passed as a string, and the kernel steps go through a
//! test-local recording `GuestTransportOps`. Nothing here calls `main`'s live
//! path or reaches `reboot(RB_POWER_OFF)`. The genuine in-guest configuration
//! on the real guest kernel (D-UP-18) is `transport::real_kernel`. Every
//! `GuestTransportSetupError` variant is triggered by name through its step
//! (C6b) and renders its pinned text.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::items_after_statements,
    clippy::doc_markdown
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;

use overdrive_core::guest_transport::forwarder::{ForwarderError, ForwarderProbeStage};
use overdrive_core::guest_transport::{GUEST_FORWARDER_MODULE_PATH, LaunchToken};
use overdrive_core::vm::config::VsockPort;
use proptest::prelude::*;

use super::*;

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
/// GUEST-PORTS (ADR-0217): the three host listener ports the VM driver
/// appends. Deliberately not the production defaults 1234 / 1240 / 1243, so
/// a parser holding a host-port constant cannot pass.
const BEACON: u32 = 50_234;
const STREAM: u32 = 50_240;
const CONTROL: u32 = 50_243;
const PORTS: &str = ",beacon=50234,stream=50240,control=50243";
const VALID: &str = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef,beacon=50234,stream=50240,control=50243";
/// The first four positions of [`VALID`]; a case appends its own port
/// fields.
const HEAD: &str = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef";

/// The literal reserved range (`GUEST_TCP_INTAKE_PORT ..= GUEST_UDP_SLOT_BASE +
/// GUEST_UDP_SLOTS - 1`), spelled out so the oracle does not depend on the
/// scaffolded `guest_reserved_ports()`.
const RESERVED: RangeInclusive<u16> = 61_000..=62_024;
const ALL_PROGRAMS: [GuestProgram; 7] = [
    GuestProgram::TcpCorrelation,
    GuestProgram::TcpCorrelationCandidate,
    GuestProgram::Connect4,
    GuestProgram::Sendmsg4,
    GuestProgram::Getpeername4,
    GuestProgram::Recvmsg4,
    GuestProgram::SockRelease,
];

fn launch() -> LaunchToken {
    LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token is 32 lowercase hex")
}

fn host_ports() -> GuestHostPorts {
    GuestHostPorts {
        beacon: VsockPort::new(BEACON),
        stream: VsockPort::new(STREAM),
        control: VsockPort::new(CONTROL),
    }
}

fn token() -> GuestTransportToken {
    GuestTransportToken { workload_addr: WL, gateway: GW, launch: launch(), ports: host_ports() }
}

fn malformed(cmdline: &str) -> TransportTokenField {
    match parse_transport_token(cmdline) {
        Err(GuestTransportSetupError::TokenMalformed { field }) => field,
        other => panic!("expected TokenMalformed for {cmdline:?}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Recording ops (test-local `GuestTransportOps`)
// ---------------------------------------------------------------------------

/// One setup step, with the argument the step was called with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    DummyDevice,
    AddressAdd(Ipv4Addr),
    FwmarkRule,
    FwmarkRoute,
    TcpFwmarkAccept,
    FwmarkReflect,
    ReservePorts(RangeInclusive<u16>),
    BindTcpIntake(SocketAddrV4),
    BindUdpSlot(SocketAddrV4),
    PrepareGuestPrograms,
    Load(GuestProgram),
    Attach(GuestProgram),
    LoadForwarderModule,
    OpenForwarder,
    BindInboundVsock,
}

/// How the scripted `load_forwarder_module` step fails.
#[derive(Debug, Clone, Copy)]
enum ModuleFailure {
    Missing,
    Release,
    Load,
}

/// Which embedded object the scripted preparation step refuses.
#[derive(Debug, Clone, Copy)]
enum PrepareFailure {
    RustObject,
    CandidateObject,
}

/// Records every step and fails the one equal to `fail_on`.
#[derive(Default)]
struct RecordingOps {
    calls: Vec<Step>,
    fail_on: Option<Step>,
    module_failure: Option<ModuleFailure>,
    prepare_failure: Option<PrepareFailure>,
}

impl RecordingOps {
    fn failing_on(step: Step) -> Self {
        Self { calls: Vec::new(), fail_on: Some(step), module_failure: None, prepare_failure: None }
    }

    fn failing_module(failure: ModuleFailure) -> Self {
        Self {
            calls: Vec::new(),
            fail_on: Some(Step::LoadForwarderModule),
            module_failure: Some(failure),
            prepare_failure: None,
        }
    }

    fn failing_prepare(failure: PrepareFailure) -> Self {
        Self {
            calls: Vec::new(),
            fail_on: Some(Step::PrepareGuestPrograms),
            module_failure: None,
            prepare_failure: Some(failure),
        }
    }

    fn record(&mut self, step: Step) -> bool {
        let fail = self.fail_on.as_ref() == Some(&step);
        self.calls.push(step);
        fail
    }

    fn io(&mut self, step: Step) -> io::Result<()> {
        if self.record(step) {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    }
}

/// The module release the scripted mismatch names.
fn module_release() -> KernelRelease {
    KernelRelease::from_str("6.18.0-overdrive").expect("a kernel release")
}

/// The running kernel release the scripted mismatch names.
fn kernel_release() -> KernelRelease {
    KernelRelease::from_str("7.0.0-29-generic").expect("a kernel release")
}

impl GuestTransportOps for RecordingOps {
    fn create_dummy_device(&mut self) -> io::Result<()> {
        self.io(Step::DummyDevice)
    }
    fn add_workload_address(&mut self, addr: Ipv4Addr) -> io::Result<()> {
        self.io(Step::AddressAdd(addr))
    }
    fn add_fwmark_rule(&mut self) -> io::Result<()> {
        self.io(Step::FwmarkRule)
    }
    fn add_fwmark_route(&mut self) -> io::Result<()> {
        self.io(Step::FwmarkRoute)
    }
    fn enable_tcp_fwmark_accept(&mut self) -> io::Result<()> {
        self.io(Step::TcpFwmarkAccept)
    }
    fn enable_fwmark_reflect(&mut self) -> io::Result<()> {
        self.io(Step::FwmarkReflect)
    }
    fn reserve_ports(&mut self, ports: RangeInclusive<u16>) -> io::Result<()> {
        self.io(Step::ReservePorts(ports))
    }
    fn bind_tcp_intake(&mut self, addr: SocketAddrV4) -> io::Result<()> {
        self.io(Step::BindTcpIntake(addr))
    }
    fn bind_udp_slot(&mut self, addr: SocketAddrV4) -> io::Result<()> {
        self.io(Step::BindUdpSlot(addr))
    }
    fn prepare_guest_programs(&mut self) -> Result<(), GuestTransportSetupError> {
        if !self.record(Step::PrepareGuestPrograms) {
            return Ok(());
        }
        let program = match self.prepare_failure.unwrap_or(PrepareFailure::RustObject) {
            PrepareFailure::RustObject => GuestProgram::TcpCorrelation,
            PrepareFailure::CandidateObject => GuestProgram::TcpCorrelationCandidate,
        };
        Err(GuestTransportSetupError::ProgramLoad { program, source: aya::EbpfError::NoBTF })
    }
    fn load_program(&mut self, program: GuestProgram) -> Result<(), aya::EbpfError> {
        if self.record(Step::Load(program)) { Err(aya::EbpfError::NoBTF) } else { Ok(()) }
    }
    fn attach_program(&mut self, program: GuestProgram) -> Result<(), aya::programs::ProgramError> {
        if self.record(Step::Attach(program)) {
            Err(aya::programs::ProgramError::NotLoaded)
        } else {
            Ok(())
        }
    }
    fn load_forwarder_module(&mut self) -> Result<(), GuestTransportSetupError> {
        if !self.record(Step::LoadForwarderModule) {
            return Ok(());
        }
        Err(match self.module_failure.unwrap_or(ModuleFailure::Load) {
            ModuleFailure::Missing => GuestTransportSetupError::ForwarderModuleMissing {
                path: PathBuf::from(GUEST_FORWARDER_MODULE_PATH),
            },
            ModuleFailure::Release => GuestTransportSetupError::ForwarderModuleRelease {
                module: module_release(),
                kernel: kernel_release(),
            },
            ModuleFailure::Load => GuestTransportSetupError::ForwarderModuleLoad {
                source: io::Error::from_raw_os_error(nix::libc::ENOEXEC),
            },
        })
    }
    fn open_forwarder(&mut self) -> Result<(), ForwarderError> {
        if self.record(Step::OpenForwarder) {
            Err(ForwarderError::Probe {
                stage: ForwarderProbeStage::DeviceOpen,
                source: Box::new(ForwarderError::DeviceAbsent {
                    source: io::Error::from(io::ErrorKind::NotFound),
                }),
            })
        } else {
            Ok(())
        }
    }
    fn bind_inbound_vsock(&mut self) -> io::Result<()> {
        self.io(Step::BindInboundVsock)
    }
}

/// The full pinned order for `token()`.
fn expected_order() -> Vec<Step> {
    let mut steps = vec![
        Step::DummyDevice,
        Step::AddressAdd(WL),
        Step::FwmarkRule,
        Step::FwmarkRoute,
        Step::TcpFwmarkAccept,
        Step::FwmarkReflect,
        Step::ReservePorts(RESERVED),
        Step::BindTcpIntake(SocketAddrV4::new(WL, 61_000)),
    ];
    steps.extend((61_001..=62_024).map(|port| Step::BindUdpSlot(SocketAddrV4::new(WL, port))));
    steps.push(Step::PrepareGuestPrograms);
    for program in ALL_PROGRAMS {
        steps.push(Step::Load(program));
        steps.push(Step::Attach(program));
    }
    steps.extend([Step::LoadForwarderModule, Step::OpenForwarder, Step::BindInboundVsock]);
    steps
}

// ---------------------------------------------------------------------------
// Plain data (real now): Display texts and labels
// ---------------------------------------------------------------------------

fn setup_error_display_cases() -> Vec<(GuestTransportSetupError, String)> {
    let e = || io::Error::other("boom");
    let addr = SocketAddrV4::new(WL, 61_001);
    vec![
        (
            GuestTransportSetupError::TokenMissing,
            "kernel command line has no overdrive.transport= token".into(),
        ),
        (
            GuestTransportSetupError::TokenDuplicate,
            "kernel command line has more than one overdrive.transport= token".into(),
        ),
        (
            GuestTransportSetupError::TokenMalformed { field: TransportTokenField::Launch },
            "overdrive.transport= field launch is malformed".into(),
        ),
        (
            GuestTransportSetupError::DummyDevice { source: e() },
            "creating the transport dummy device failed: boom".into(),
        ),
        (
            GuestTransportSetupError::AddressAdd { source: e() },
            "adding the workload address failed: boom".into(),
        ),
        (
            GuestTransportSetupError::FwmarkRule { source: e() },
            "adding the fwmark lookup rule failed: boom".into(),
        ),
        (
            GuestTransportSetupError::FwmarkRoute { source: e() },
            "adding the fwmark local route failed: boom".into(),
        ),
        (
            GuestTransportSetupError::TcpFwmarkAccept { source: e() },
            "setting net.ipv4.tcp_fwmark_accept failed: boom".into(),
        ),
        (
            GuestTransportSetupError::FwmarkReflect { source: e() },
            "setting net.ipv4.fwmark_reflect failed: boom".into(),
        ),
        (
            GuestTransportSetupError::ProgramLoad {
                program: GuestProgram::TcpCorrelation,
                source: aya::EbpfError::NoBTF,
            },
            "loading guest program tcp_correlation failed: no BTF parsed for object".into(),
        ),
        (
            GuestTransportSetupError::ProgramAttach {
                program: GuestProgram::TcpCorrelation,
                source: aya::programs::ProgramError::NotLoaded,
            },
            "attaching guest program tcp_correlation failed: the program is not loaded".into(),
        ),
        (
            GuestTransportSetupError::ProgramLoad {
                program: GuestProgram::TcpCorrelationCandidate,
                source: aya::EbpfError::NoBTF,
            },
            "loading guest program tcp_correlation_candidate failed: no BTF parsed for object"
                .into(),
        ),
        (
            GuestTransportSetupError::ProgramAttach {
                program: GuestProgram::TcpCorrelationCandidate,
                source: aya::programs::ProgramError::NotLoaded,
            },
            "attaching guest program tcp_correlation_candidate failed: the program is not loaded"
                .into(),
        ),
        (
            GuestTransportSetupError::InboundListenerBind { source: e() },
            "binding the inbound vsock listener failed: boom".into(),
        ),
        (
            GuestTransportSetupError::ReservedPorts { source: e() },
            "reserving the guest transport ports failed: boom".into(),
        ),
        (
            GuestTransportSetupError::IntakeBind { addr, source: e() },
            "binding the guest intake on 100.95.0.7:61001 failed: boom".into(),
        ),
        (
            GuestTransportSetupError::ForwarderModuleMissing {
                path: PathBuf::from("/lib/overdrive/ovd_fwd.ko"),
            },
            "the guest forwarder module /lib/overdrive/ovd_fwd.ko is missing; the VM's kernel \
             and root filesystem must come from one guest kit"
                .into(),
        ),
        (
            GuestTransportSetupError::ForwarderModuleLoad { source: e() },
            "loading the guest forwarder module failed: boom".into(),
        ),
        (
            GuestTransportSetupError::Forwarder {
                source: ForwarderError::AbiMismatch { expected: 1, found: 2 },
            },
            "opening the guest forwarder failed: forwarder ABI 2 is not the expected 1".into(),
        ),
    ]
}

/// Every `GuestTransportSetupError` variant whose fields are plain data
/// renders exactly the DG-5 table text.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn every_setup_error_renders_its_pinned_display_text() {
    for (error, expected) in setup_error_display_cases() {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

/// `ForwarderModuleRelease` names both releases and the guest-kit rule
/// (EVMOD-GUEST-KIT); its fields are `KernelRelease`s, whose codec DELIVER
/// step 5 lands.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_release_mismatch_renders_both_releases_and_the_guest_kit_rule() {
    let error = GuestTransportSetupError::ForwarderModuleRelease {
        module: module_release(),
        kernel: kernel_release(),
    };
    assert_eq!(
        error.to_string(),
        "the guest forwarder module was built for kernel 6.18.0-overdrive, but the guest runs \
         7.0.0-29-generic; the VM's kernel and root filesystem must come from one guest kit"
    );
}

/// `KernelRelease`: every non-empty string of at most 64 bytes with no
/// whitespace or NUL round-trips through `FromStr` / `Display`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_kernel_release_roundtrips() {
    proptest!(|(release in "[A-Za-z0-9._+~-]{1,64}")| {
        let parsed = KernelRelease::from_str(&release).expect("a valid release");
        prop_assert_eq!(parsed.to_string(), release);
    });
}

/// `KernelRelease` refuses the empty string, more than 64 bytes, and any
/// whitespace or NUL, each with its own variant.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_invalid_kernel_release_is_refused_by_kind() {
    assert_eq!(KernelRelease::from_str(""), Err(KernelReleaseError::Empty));
    assert_eq!(KernelRelease::from_str(&"a".repeat(65)), Err(KernelReleaseError::TooLong));
    assert!(KernelRelease::from_str(&"a".repeat(64)).is_ok(), "64 bytes is the ceiling");
    for bad in ["7.0.0 29", "7.0.0\t29", "7.0.0\n", "7.0\0.0", " 7.0.0"] {
        assert_eq!(
            KernelRelease::from_str(bad),
            Err(KernelReleaseError::InvalidCharacter),
            "{bad:?}"
        );
    }
}

/// The label enums own their lower snake case labels; the capture programs
/// are the only guest programs (decision B-2: no drain, classifier or unframe
/// programs).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn label_enums_render_lower_snake_case() {
    let fields = [
        (TransportTokenField::Version, "version"),
        (TransportTokenField::Addr, "addr"),
        (TransportTokenField::Dns, "dns"),
        (TransportTokenField::Launch, "launch"),
        (TransportTokenField::Beacon, "beacon"),
        (TransportTokenField::Stream, "stream"),
        (TransportTokenField::Control, "control"),
        (TransportTokenField::Extra, "extra"),
    ];
    for (field, label) in fields {
        assert_eq!(field.as_str(), label);
    }
    let labels = [
        "tcp_correlation",
        "tcp_correlation_candidate",
        "connect4",
        "sendmsg4",
        "getpeername4",
        "recvmsg4",
        "sock_release",
    ];
    for (program, label) in ALL_PROGRAMS.into_iter().zip(labels) {
        assert_eq!(program.as_str(), label);
    }
    assert_eq!(GuestProgram::ALL, ALL_PROGRAMS, "ALL lists every program in enum order");
}

// ---------------------------------------------------------------------------
// Token grammar (P6-S11 unit row)
// ---------------------------------------------------------------------------

/// A well-formed token among other kernel parameters parses to `(A, G, T)`;
/// the other parameters are ignored.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn well_formed_token_parses_and_other_parameters_are_ignored() {
    for cmdline in [
        VALID.to_owned(),
        format!("console=ttyS0 root=/dev/vda rw {VALID} quiet"),
        format!("overdrive.net=ignored {VALID} overdrive.transportx=1"),
    ] {
        let parsed = parse_transport_token(&cmdline).expect("well-formed token parses");
        assert_eq!(parsed, token(), "{cmdline}");
    }
}

/// No token → `TokenMissing`; two tokens → `TokenDuplicate` (even if equal).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn missing_and_duplicate_tokens_are_refused() {
    for cmdline in ["", "console=ttyS0 root=/dev/vda", "overdrive.net=10.0.0.2/30,10.0.0.1"] {
        assert!(
            matches!(parse_transport_token(cmdline), Err(GuestTransportSetupError::TokenMissing)),
            "{cmdline:?}"
        );
    }
    let other = VALID.replace("100.95.0.7", "100.95.0.8");
    for cmdline in [format!("{VALID} {VALID}"), format!("{VALID} console=ttyS0 {other}")] {
        assert!(
            matches!(
                parse_transport_token(&cmdline),
                Err(GuestTransportSetupError::TokenDuplicate)
            ),
            "{cmdline:?}"
        );
    }
}

/// The first position whose key or value is invalid names the field.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn first_invalid_position_names_the_field() {
    let launch = "launch=0123456789abcdef0123456789abcdef";
    let cases: Vec<(String, TransportTokenField)> = vec![
        // Version
        (
            format!("overdrive.transport=vsock1,addr=100.95.0.7,dns=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Version,
        ),
        (
            format!("overdrive.transport=,addr=100.95.0.7,dns=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Version,
        ),
        // Version invalid AND addr invalid: the first position wins
        (
            format!("overdrive.transport=vsock9,addr=x,dns=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Version,
        ),
        // Addr: bad value, bad key, reordered
        (
            format!("overdrive.transport=vsock2,addr=not-an-ip,dns=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Addr,
        ),
        (
            format!("overdrive.transport=vsock2,adr=100.95.0.7,dns=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Addr,
        ),
        (
            format!("overdrive.transport=vsock2,dns=100.95.0.1,addr=100.95.0.7,{launch}{PORTS}"),
            TransportTokenField::Addr,
        ),
        // Dns
        (
            format!("overdrive.transport=vsock2,addr=100.95.0.7,dns=300.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Dns,
        ),
        (
            format!("overdrive.transport=vsock2,addr=100.95.0.7,gw=100.95.0.1,{launch}{PORTS}"),
            TransportTokenField::Dns,
        ),
        // Launch: not hex, upper case, short, all zero, bad key
        (
            format!(
                "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=NOTHEX{PORTS}"
            ),
            TransportTokenField::Launch,
        ),
        (
            format!(
                "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789ABCDEF0123456789ABCDEF{PORTS}"
            ),
            TransportTokenField::Launch,
        ),
        (
            format!(
                "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef{PORTS}"
            ),
            TransportTokenField::Launch,
        ),
        (
            format!(
                "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=00000000000000000000000000000000{PORTS}"
            ),
            TransportTokenField::Launch,
        ),
        (
            format!(
                "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,token=0123456789abcdef0123456789abcdef{PORTS}"
            ),
            TransportTokenField::Launch,
        ),
        // Launch invalid AND a port invalid: the first position wins
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=NOTHEX,beacon=1,stream=50240,control=50243"
                .to_owned(),
            TransportTokenField::Launch,
        ),
        // Extra: an eighth field
        (format!("{VALID},extra=1"), TransportTokenField::Extra),
        (format!("{VALID},"), TransportTokenField::Extra),
    ];
    for (cmdline, field) in cases {
        assert_eq!(malformed(&cmdline), field, "{cmdline}");
    }
}

/// GUEST-PORTS (ADR-0217): a port field that is not a decimal `u32`, lies in
/// 0..=1023, is `u32::MAX` (`VMADDR_PORT_ANY`), carries a wrong key, is out
/// of order, or repeats an earlier port field is `TokenMalformed` naming that
/// field (for a repeat, the later field). Each refusal is reached before any
/// guest setup step: `parse_transport_token` is the only caller-visible step.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn an_invalid_host_port_field_names_that_field() {
    let field_cases: [(&str, TransportTokenField); 3] = [
        ("beacon", TransportTokenField::Beacon),
        ("stream", TransportTokenField::Stream),
        ("control", TransportTokenField::Control),
    ];
    let max = u32::MAX.to_string();
    let over = (u64::from(u32::MAX) + 1).to_string();
    // Every refusal class for each of the three fields, the others valid.
    for (index, (key, field)) in field_cases.iter().enumerate() {
        for bad in
            ["0", "1", "1023", max.as_str(), over.as_str(), "-1", "0x400", "", "12a", " 2000"]
        {
            let mut values = [BEACON.to_string(), STREAM.to_string(), CONTROL.to_string()];
            values[index] = bad.to_owned();
            let cmdline =
                format!("{HEAD},beacon={},stream={},control={}", values[0], values[1], values[2]);
            assert_eq!(malformed(&cmdline), *field, "{key}={bad:?} in {cmdline}");
        }
    }
    // Wrong key and wrong order name the position's field.
    for (cmdline, field) in [
        (format!("{HEAD},port=50234,stream=50240,control=50243"), TransportTokenField::Beacon),
        (format!("{HEAD},stream=50240,beacon=50234,control=50243"), TransportTokenField::Beacon),
        (format!("{HEAD},beacon=50234,flow=50240,control=50243"), TransportTokenField::Stream),
        (format!("{HEAD},beacon=50234,stream=50240,ctl=50243"), TransportTokenField::Control),
    ] {
        assert_eq!(malformed(&cmdline), field, "{cmdline}");
    }
    // A repeat names the later field.
    for (cmdline, field) in [
        (format!("{HEAD},beacon=50234,stream=50234,control=50243"), TransportTokenField::Stream),
        (format!("{HEAD},beacon=50234,stream=50240,control=50234"), TransportTokenField::Control),
        (format!("{HEAD},beacon=50234,stream=50240,control=50240"), TransportTokenField::Control),
    ] {
        assert_eq!(malformed(&cmdline), field, "{cmdline}");
    }
    // The boundaries that are valid: 1024 and u32::MAX - 1.
    let edge = format!("{HEAD},beacon=1024,stream={},control=50243", u32::MAX - 1);
    let parsed = parse_transport_token(&edge).expect("1024 and u32::MAX - 1 are valid ports");
    assert_eq!(parsed.ports.beacon, VsockPort::new(1_024));
    assert_eq!(parsed.ports.stream, VsockPort::new(u32::MAX - 1));
}

/// A missing trailing field names the first missing one.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn missing_trailing_field_names_the_first_missing_one() {
    let cases = [
        ("overdrive.transport=vsock2".to_owned(), TransportTokenField::Addr),
        ("overdrive.transport=vsock2,addr=100.95.0.7".to_owned(), TransportTokenField::Dns),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1".to_owned(),
            TransportTokenField::Launch,
        ),
        // GUEST-PORTS: the pre-#295-round-10 four-field token is now short.
        (HEAD.to_owned(), TransportTokenField::Beacon),
        (format!("{HEAD},beacon=50234"), TransportTokenField::Stream),
        (format!("{HEAD},beacon=50234,stream=50240"), TransportTokenField::Control),
    ];
    for (cmdline, field) in cases {
        assert_eq!(malformed(&cmdline), field, "{cmdline}");
    }
}

proptest! {
    /// Every rendering of a valid `(addr, gateway, launch)` parses back to it.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
        fn token_rendering_roundtrips(
        addr in any::<u32>(),
        gateway in any::<u32>(),
        bytes in any::<[u8; 16]>(),
        ports in proptest::collection::btree_set(1_024u32..u32::MAX, 3),
    ) {
        prop_assume!(bytes != [0u8; 16]);
        let launch = LaunchToken::from_bytes(bytes).expect("non-zero bytes are a launch token");
        let (addr, gateway) = (Ipv4Addr::from(addr), Ipv4Addr::from(gateway));
        // Three distinct valid ports, in a generated (not sorted) assignment.
        let mut ports: Vec<u32> = ports.into_iter().collect();
        ports.rotate_left(usize::from(bytes[0] % 3));
        let (beacon, stream, control) = (ports[0], ports[1], ports[2]);
        let cmdline = format!(
            "console=ttyS0 overdrive.transport=vsock2,addr={addr},dns={gateway},launch={launch},beacon={beacon},stream={stream},control={control} rw"
        );
        let parsed = parse_transport_token(&cmdline).expect("valid rendering parses");
        prop_assert_eq!(parsed, GuestTransportToken {
            workload_addr: addr,
            gateway,
            launch,
            ports: GuestHostPorts {
                beacon: VsockPort::new(beacon),
                stream: VsockPort::new(stream),
                control: VsockPort::new(control),
            },
        });
    }
}

// ---------------------------------------------------------------------------
// Setup order (G-V7 row 1) and stop at the first failure (row 2)
// ---------------------------------------------------------------------------

/// A clean run calls every step once, in the pinned order — two-object map
/// preparation, then the seven capture programs, then the forwarder module,
/// its context and the inbound listener — and returns `Ok`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn configure_runs_every_step_in_the_pinned_order() {
    let mut ops = RecordingOps::default();
    configure_guest_transport(&token(), &mut ops).expect("no step fails");
    assert_eq!(ops.calls.len(), 8 + 1_024 + 1 + 14 + 3);
    assert_eq!(ops.calls, expected_order());
    let prepare =
        ops.calls.iter().position(|s| *s == Step::PrepareGuestPrograms).expect("preparation");
    let first_load =
        ops.calls.iter().position(|s| matches!(s, Step::Load(_))).expect("program load");
    assert!(prepare < first_load, "both embedded objects are prepared before any program load");
    let module = ops.calls.iter().position(|s| *s == Step::LoadForwarderModule).expect("module");
    let open = ops.calls.iter().position(|s| *s == Step::OpenForwarder).expect("context");
    let last_attach =
        ops.calls.iter().rposition(|s| matches!(s, Step::Attach(_))).expect("programs attached");
    assert!(last_attach < module && module < open, "programs, then module, then context");
    assert_eq!(ops.calls.last(), Some(&Step::BindInboundVsock), "the inbound listener is last");
}

type SetupCheck = fn(&GuestTransportSetupError) -> bool;

fn network_setup_failure_cases() -> Vec<(Step, SetupCheck)> {
    vec![
        (Step::DummyDevice, |e| matches!(e, GuestTransportSetupError::DummyDevice { .. })),
        (Step::AddressAdd(WL), |e| matches!(e, GuestTransportSetupError::AddressAdd { .. })),
        (Step::FwmarkRule, |e| matches!(e, GuestTransportSetupError::FwmarkRule { .. })),
        (Step::FwmarkRoute, |e| matches!(e, GuestTransportSetupError::FwmarkRoute { .. })),
        (Step::TcpFwmarkAccept, |e| matches!(e, GuestTransportSetupError::TcpFwmarkAccept { .. })),
        (Step::FwmarkReflect, |e| matches!(e, GuestTransportSetupError::FwmarkReflect { .. })),
        (Step::ReservePorts(RESERVED), |e| {
            matches!(e, GuestTransportSetupError::ReservedPorts { .. })
        }),
        (
            Step::BindTcpIntake(SocketAddrV4::new(WL, 61_000)),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_000),
        ),
        (
            Step::BindUdpSlot(SocketAddrV4::new(WL, 61_001)),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_001),
        ),
        (
            Step::BindUdpSlot(SocketAddrV4::new(WL, 61_513)),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_513),
        ),
        (
            Step::BindUdpSlot(SocketAddrV4::new(WL, 62_024)),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 62_024),
        ),
    ]
}

fn program_setup_failure_cases() -> Vec<(Step, SetupCheck)> {
    vec![
        (Step::Load(GuestProgram::TcpCorrelation), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad { program: GuestProgram::TcpCorrelation, .. }
            )
        }),
        (Step::Attach(GuestProgram::TcpCorrelation), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach {
                    program: GuestProgram::TcpCorrelation,
                    ..
                }
            )
        }),
        (Step::Load(GuestProgram::TcpCorrelationCandidate), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad {
                    program: GuestProgram::TcpCorrelationCandidate,
                    ..
                }
            )
        }),
        (Step::Attach(GuestProgram::TcpCorrelationCandidate), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach {
                    program: GuestProgram::TcpCorrelationCandidate,
                    ..
                }
            )
        }),
        (Step::Load(GuestProgram::Connect4), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad { program: GuestProgram::Connect4, .. }
            )
        }),
        (Step::Attach(GuestProgram::Getpeername4), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach { program: GuestProgram::Getpeername4, .. }
            )
        }),
        (Step::Load(GuestProgram::SockRelease), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad { program: GuestProgram::SockRelease, .. }
            )
        }),
        (Step::Attach(GuestProgram::SockRelease), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach { program: GuestProgram::SockRelease, .. }
            )
        }),
        (Step::LoadForwarderModule, |e| {
            matches!(e, GuestTransportSetupError::ForwarderModuleLoad { .. })
        }),
        (Step::OpenForwarder, |e| {
            matches!(
                e,
                GuestTransportSetupError::Forwarder {
                    source: ForwarderError::Probe { stage: ForwarderProbeStage::DeviceOpen, .. }
                }
            )
        }),
        (Step::BindInboundVsock, |e| {
            matches!(e, GuestTransportSetupError::InboundListenerBind { .. })
        }),
    ]
}

fn assert_preparation_failures_stop_before_load(full: &[Step]) {
    let prepare_stop =
        full.iter().position(|s| *s == Step::PrepareGuestPrograms).expect("preparation step");
    for (failure, program) in [
        (PrepareFailure::RustObject, GuestProgram::TcpCorrelation),
        (PrepareFailure::CandidateObject, GuestProgram::TcpCorrelationCandidate),
    ] {
        let mut ops = RecordingOps::failing_prepare(failure);
        let error =
            configure_guest_transport(&token(), &mut ops).expect_err("program preparation fails");
        assert!(
            matches!(
                error,
                GuestTransportSetupError::ProgramLoad {
                    program: observed,
                    source: aya::EbpfError::NoBTF,
                } if observed == program
            ),
            "{failure:?} mapped to {error:?}"
        );
        assert_eq!(
            ops.calls,
            full[..=prepare_stop].to_vec(),
            "{failure:?}: no load or attach runs after preparation failure"
        );
    }
}

/// A failing step stops the run there (no later step is called) and maps to
/// exactly its variant.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn configure_stops_at_the_first_failure_with_that_steps_variant() {
    let full = expected_order();
    let cases = network_setup_failure_cases().into_iter().chain(program_setup_failure_cases());
    for (failing, check) in cases {
        let mut ops = RecordingOps::failing_on(failing.clone());
        let error =
            configure_guest_transport(&token(), &mut ops).expect_err("the scripted step fails");
        assert!(check(&error), "step {failing:?} mapped to {error:?}");
        let stop = full.iter().position(|s| *s == failing).expect("failing step is in the order");
        assert_eq!(ops.calls, full[..=stop].to_vec(), "no step after {failing:?} runs");
    }
    assert_preparation_failures_stop_before_load(&full);
}

/// EVMOD-GUEST-KIT: each of the three forwarder-module failures — no module
/// file at `GUEST_FORWARDER_MODULE_PATH`, a module built for another kernel
/// release, any other `finit_module` refusal — stops the setup at the module
/// step with its own variant, before the context is opened and before the
/// inbound listener is bound: a pre-READY start failure (the guest exits
/// before READY, the host sees the existing "guest exited before READY").
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn each_forwarder_module_failure_is_a_pre_ready_start_failure_of_its_own_kind() {
    let full = expected_order();
    let stop = full.iter().position(|s| *s == Step::LoadForwarderModule).expect("module step");
    type Check = fn(&GuestTransportSetupError) -> bool;
    let cases: [(ModuleFailure, Check); 3] = [
        (ModuleFailure::Missing, |e| {
            matches!(
                e,
                GuestTransportSetupError::ForwarderModuleMissing { path }
                    if path.as_path() == std::path::Path::new(GUEST_FORWARDER_MODULE_PATH)
            )
        }),
        (ModuleFailure::Release, |e| {
            matches!(
                e,
                GuestTransportSetupError::ForwarderModuleRelease { module, kernel }
                    if *module == module_release() && *kernel == kernel_release()
            )
        }),
        (ModuleFailure::Load, |e| {
            matches!(
                e,
                GuestTransportSetupError::ForwarderModuleLoad { source }
                    if source.raw_os_error() == Some(nix::libc::ENOEXEC)
            )
        }),
    ];
    for (failure, check) in cases {
        let mut ops = RecordingOps::failing_module(failure);
        let error =
            configure_guest_transport(&token(), &mut ops).expect_err("the module step fails");
        assert!(check(&error), "{failure:?} mapped to {error:?}");
        assert_eq!(ops.calls, full[..=stop].to_vec(), "{failure:?}: nothing after the module step");
        assert!(!ops.calls.contains(&Step::OpenForwarder), "no context without the module");
        assert!(!ops.calls.contains(&Step::BindInboundVsock), "no inbound listener: no READY");
    }
}
