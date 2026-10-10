//! P6-S11 — guest network configuration before READY: the transport-token
//! grammar, the `configure_guest_transport` order with stop-at-first-failure,
//! and every pinned `GuestTransportSetupError` `Display` text (G-V7 rows 1/2/4;
//! feature-delta § *Guest adaptation contract*, DG-5 "Transport token and
//! setup").
//!
//! Re-authored from the orphaned drafts at `91f9436ff`
//! (`tests/integration/vsock_guest_ready_config.rs`, the token half of
//! `tests/acceptance/vsock_guest_udp_slots.rs`): `overdrive-init` has no
//! library target, so the driving ports (`parse_transport_token`,
//! `configure_guest_transport`) are exercised source-locally, the kernel
//! command line is passed as a string, and the kernel steps go through a
//! test-local recording `GuestTransportOps`. Nothing here calls `main`'s live
//! path or reaches `reboot(RB_POWER_OFF)`. The genuine in-guest configuration
//! (dummy device, rule, sysctls, intakes, programs, seeded map, inbound
//! listener — observed with `ip`/`sysctl`/`ss` before READY) is the P6 walking
//! skeleton's Tier-3 cell, not a host test.

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
use std::str::FromStr;

use overdrive_core::guest_transport::LaunchToken;
use proptest::prelude::*;

use super::*;

const WL: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 7);
const GW: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
const LAUNCH_HEX: &str = "0123456789abcdef0123456789abcdef";
const VALID: &str = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef";

/// The literal reserved range (`GUEST_TCP_INTAKE_PORT ..= GUEST_UDP_SLOT_BASE +
/// GUEST_UDP_SLOTS - 1`), spelled out so the oracle does not depend on the
/// scaffolded `guest_reserved_ports()`.
const RESERVED: RangeInclusive<u16> = 61_000..=62_024;
const ALL_PROGRAMS: [GuestProgram; 12] = [
    GuestProgram::Connect4,
    GuestProgram::Sendmsg4,
    GuestProgram::Getpeername4,
    GuestProgram::Recvmsg4,
    GuestProgram::SockRelease,
    GuestProgram::SockOps,
    GuestProgram::Verdict,
    GuestProgram::StreamParser,
    GuestProgram::UnframeLo,
    GuestProgram::DrainCounter,
    GuestProgram::ListenStart,
    GuestProgram::ListenStop,
];

fn launch() -> LaunchToken {
    LaunchToken::from_str(LAUNCH_HEX).expect("fixture launch token is 32 lowercase hex")
}

fn token() -> GuestTransportToken {
    GuestTransportToken { workload_addr: WL, gateway: GW, launch: launch() }
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
    FwmarkReflect,
    ReservePorts(RangeInclusive<u16>),
    BindTcpIntake(SocketAddrV4),
    BindUdpSlot(SocketAddrV4),
    CheckLayout,
    Load(GuestProgram),
    Attach(GuestProgram),
    InstallUdpSlots,
    SeedListenerMap,
    BindInboundVsock,
}

/// Records every step and fails the one equal to `fail_on`.
#[derive(Default)]
struct RecordingOps {
    calls: Vec<Step>,
    fail_on: Option<Step>,
}

impl RecordingOps {
    fn failing_on(step: Step) -> Self {
        Self { calls: Vec::new(), fail_on: Some(step) }
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
    fn check_sock_common_layout(&mut self) -> Result<(), SockCommonField> {
        if self.record(Step::CheckLayout) { Err(SockCommonField::Num) } else { Ok(()) }
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
    fn install_udp_slots(&mut self) -> io::Result<()> {
        self.io(Step::InstallUdpSlots)
    }
    fn seed_listener_map(&mut self) -> io::Result<()> {
        self.io(Step::SeedListenerMap)
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
        Step::FwmarkReflect,
        Step::ReservePorts(RESERVED),
        Step::BindTcpIntake(SocketAddrV4::new(WL, 61_000)),
    ];
    steps.extend((61_001..=62_024).map(|port| Step::BindUdpSlot(SocketAddrV4::new(WL, port))));
    steps.push(Step::CheckLayout);
    for program in ALL_PROGRAMS {
        steps.push(Step::Load(program));
        steps.push(Step::Attach(program));
    }
    steps.extend([Step::InstallUdpSlots, Step::SeedListenerMap, Step::BindInboundVsock]);
    steps
}

// ---------------------------------------------------------------------------
// Plain data (real now): Display texts and labels
// ---------------------------------------------------------------------------

/// Every `GuestTransportSetupError` variant renders exactly the DG-5 table text.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn every_setup_error_renders_its_pinned_display_text() {
    let e = || io::Error::other("boom");
    let addr = SocketAddrV4::new(WL, 61_001);
    let cases: Vec<(GuestTransportSetupError, String)> = vec![
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
            GuestTransportSetupError::FwmarkReflect { source: e() },
            "setting net.ipv4.fwmark_reflect failed: boom".into(),
        ),
        (
            GuestTransportSetupError::KernelLayoutMismatch { field: SockCommonField::RcvSaddr },
            "guest kernel sock_common field rcv_saddr does not match the guest programs".into(),
        ),
        (
            GuestTransportSetupError::ProgramLoad {
                program: GuestProgram::SockOps,
                source: aya::EbpfError::NoBTF,
            },
            "loading guest program sock_ops failed: no BTF parsed for object".into(),
        ),
        (
            GuestTransportSetupError::ProgramAttach {
                program: GuestProgram::ListenStop,
                source: aya::programs::ProgramError::NotLoaded,
            },
            "attaching guest program listen_stop failed: the program is not loaded".into(),
        ),
        (
            GuestTransportSetupError::ListenerSeed { source: e() },
            "seeding the listener map from sock_diag failed: boom".into(),
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
            GuestTransportSetupError::SlotInstall { source: e() },
            "installing the UDP slot sockets failed: boom".into(),
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

/// The three label enums own their lower snake case labels.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn label_enums_render_lower_snake_case() {
    let fields = [
        (TransportTokenField::Version, "version"),
        (TransportTokenField::Addr, "addr"),
        (TransportTokenField::Dns, "dns"),
        (TransportTokenField::Launch, "launch"),
        (TransportTokenField::Extra, "extra"),
    ];
    for (field, label) in fields {
        assert_eq!(field.as_str(), label);
    }
    let layout = [
        (SockCommonField::RcvSaddr, "rcv_saddr"),
        (SockCommonField::Num, "num"),
        (SockCommonField::Family, "family"),
    ];
    for (field, label) in layout {
        assert_eq!(field.as_str(), label);
    }
    let labels = [
        "connect4",
        "sendmsg4",
        "getpeername4",
        "recvmsg4",
        "sock_release",
        "sock_ops",
        "verdict",
        "stream_parser",
        "unframe_lo",
        "drain_counter",
        "listen_start",
        "listen_stop",
    ];
    for (program, label) in ALL_PROGRAMS.into_iter().zip(labels) {
        assert_eq!(program.as_str(), label);
    }
}

// ---------------------------------------------------------------------------
// Token grammar (P6-S11 unit row)
// ---------------------------------------------------------------------------

/// A well-formed token among other kernel parameters parses to `(A, G, T)`;
/// the other parameters are ignored.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
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
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
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
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn first_invalid_position_names_the_field() {
    let cases = [
        // Version
        (
            "overdrive.transport=vsock1,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Version,
        ),
        (
            "overdrive.transport=,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Version,
        ),
        // Version invalid AND addr invalid: the first position wins
        (
            "overdrive.transport=vsock9,addr=x,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Version,
        ),
        // Addr: bad value, bad key, reordered
        (
            "overdrive.transport=vsock2,addr=not-an-ip,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Addr,
        ),
        (
            "overdrive.transport=vsock2,adr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Addr,
        ),
        (
            "overdrive.transport=vsock2,dns=100.95.0.1,addr=100.95.0.7,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Addr,
        ),
        // Dns
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=300.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Dns,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,gw=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
            TransportTokenField::Dns,
        ),
        // Launch: not hex, upper case, short, all zero, bad key
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=NOTHEX",
            TransportTokenField::Launch,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789ABCDEF0123456789ABCDEF",
            TransportTokenField::Launch,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef",
            TransportTokenField::Launch,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=00000000000000000000000000000000",
            TransportTokenField::Launch,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,token=0123456789abcdef0123456789abcdef",
            TransportTokenField::Launch,
        ),
        // Extra: a fifth field
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef,extra=1",
            TransportTokenField::Extra,
        ),
        (
            "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef,",
            TransportTokenField::Extra,
        ),
    ];
    for (cmdline, field) in cases {
        assert_eq!(malformed(cmdline), field, "{cmdline}");
    }
}

/// A missing trailing field names the first missing one.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn missing_trailing_field_names_the_first_missing_one() {
    let cases = [
        ("overdrive.transport=vsock2", TransportTokenField::Addr),
        ("overdrive.transport=vsock2,addr=100.95.0.7", TransportTokenField::Dns),
        ("overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1", TransportTokenField::Launch),
    ];
    for (cmdline, field) in cases {
        assert_eq!(malformed(cmdline), field, "{cmdline}");
    }
}

proptest! {
    /// Every rendering of a valid `(addr, gateway, launch)` parses back to it.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
    fn token_rendering_roundtrips(addr in any::<u32>(), gateway in any::<u32>(), bytes in any::<[u8; 16]>()) {
        prop_assume!(bytes != [0u8; 16]);
        let launch = LaunchToken::from_bytes(bytes).expect("non-zero bytes are a launch token");
        let (addr, gateway) = (Ipv4Addr::from(addr), Ipv4Addr::from(gateway));
        let cmdline = format!("console=ttyS0 overdrive.transport=vsock2,addr={addr},dns={gateway},launch={launch} rw");
        let parsed = parse_transport_token(&cmdline).expect("valid rendering parses");
        prop_assert_eq!(parsed, GuestTransportToken { workload_addr: addr, gateway, launch });
    }
}

// ---------------------------------------------------------------------------
// Setup order (P6-S11 G-V7 row 1) and stop at the first failure (row 2)
// ---------------------------------------------------------------------------

/// A clean run calls every step once, in the pinned order, and returns `Ok`.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn configure_runs_every_step_in_the_pinned_order() {
    let mut ops = RecordingOps::default();
    configure_guest_transport(&token(), &mut ops).expect("no step fails");
    assert_eq!(ops.calls.len(), 6 + 1_024 + 1 + 24 + 3);
    assert_eq!(ops.calls, expected_order());
}

/// A failing step stops the run there (no later step is called) and maps to
/// exactly its variant.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 5 (guest adaptation in overdrive-init)"]
fn configure_stops_at_the_first_failure_with_that_steps_variant() {
    let tcp = SocketAddrV4::new(WL, 61_000);
    let first_udp = SocketAddrV4::new(WL, 61_001);
    let mid_udp = SocketAddrV4::new(WL, 61_513);
    let last_udp = SocketAddrV4::new(WL, 62_024);
    type Check = fn(&GuestTransportSetupError) -> bool;
    let cases: Vec<(Step, Check)> = vec![
        (Step::DummyDevice, |e| matches!(e, GuestTransportSetupError::DummyDevice { .. })),
        (Step::AddressAdd(WL), |e| matches!(e, GuestTransportSetupError::AddressAdd { .. })),
        (Step::FwmarkRule, |e| matches!(e, GuestTransportSetupError::FwmarkRule { .. })),
        (Step::FwmarkReflect, |e| matches!(e, GuestTransportSetupError::FwmarkReflect { .. })),
        (Step::ReservePorts(RESERVED), |e| {
            matches!(e, GuestTransportSetupError::ReservedPorts { .. })
        }),
        (
            Step::BindTcpIntake(tcp),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_000),
        ),
        (
            Step::BindUdpSlot(first_udp),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_001),
        ),
        (
            Step::BindUdpSlot(mid_udp),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 61_513),
        ),
        (
            Step::BindUdpSlot(last_udp),
            |e| matches!(e, GuestTransportSetupError::IntakeBind { addr, .. } if addr.port() == 62_024),
        ),
        (Step::CheckLayout, |e| {
            matches!(
                e,
                GuestTransportSetupError::KernelLayoutMismatch { field: SockCommonField::Num }
            )
        }),
        (Step::Load(GuestProgram::Connect4), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad { program: GuestProgram::Connect4, .. }
            )
        }),
        (Step::Attach(GuestProgram::Verdict), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach { program: GuestProgram::Verdict, .. }
            )
        }),
        (Step::Load(GuestProgram::ListenStop), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramLoad { program: GuestProgram::ListenStop, .. }
            )
        }),
        (Step::Attach(GuestProgram::ListenStop), |e| {
            matches!(
                e,
                GuestTransportSetupError::ProgramAttach { program: GuestProgram::ListenStop, .. }
            )
        }),
        (Step::InstallUdpSlots, |e| matches!(e, GuestTransportSetupError::SlotInstall { .. })),
        (Step::SeedListenerMap, |e| matches!(e, GuestTransportSetupError::ListenerSeed { .. })),
        (Step::BindInboundVsock, |e| {
            matches!(e, GuestTransportSetupError::InboundListenerBind { .. })
        }),
    ];
    let full = expected_order();
    for (failing, check) in cases {
        let mut ops = RecordingOps::failing_on(failing.clone());
        let error =
            configure_guest_transport(&token(), &mut ops).expect_err("the scripted step fails");
        assert!(check(&error), "step {failing:?} mapped to {error:?}");
        let stop = full.iter().position(|s| *s == failing).expect("failing step is in the order");
        assert_eq!(ops.calls, full[..=stop].to_vec(), "no step after {failing:?} runs");
    }
}
