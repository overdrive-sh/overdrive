//! Real-kernel checks of the guest transport (D-UP-18; feature-delta § *Guest
//! adaptation contract*, D25, D25-BIND, EVMOD-GUEST-KIT, EVMOD-LOAD; R5-15).
//!
//! **Lane.** These cells run *inside* an Overdrive microVM guest: the
//! production `overdrive-init` is PID 1 and has run `configure_guest_transport`
//! on the real guest kernel before READY; this test binary is the guest's
//! operator command, so it observes the configured guest from the
//! application's position. They are compiled source-locally under
//! `integration-tests` and run through `cargo xtask metal run --` with
//! `kvm-tests` on the native x86_64 metal host (`.claude/rules/testing.md`
//! § "Running tests — native x86_64 bare-metal KVM host"), never on a
//! developer host: every cell first proves it runs in a guest (the
//! `overdrive.transport=` token on `/proc/cmdline`) and fails closed
//! otherwise — a host run is not evidence.
//!
//! The guest lane — booting the fixture's guest kit with this test binary as
//! the operator command and reporting its result — is shared test
//! infrastructure (`overdrive-testing` VM fixture), requested alongside these
//! cells.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "real-kernel test cells: a failed precondition panics"
)]

use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::str::FromStr;

use nix::errno::Errno;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, VsockAddr, bind, socket};
use overdrive_core::guest_transport::{
    GUEST_FORWARDER_MODULE_PATH, GUEST_INBOUND_PORT, GUEST_TCP_INTAKE_PORT, GUEST_UDP_SLOT_BASE,
    GUEST_UDP_SLOTS, guest_reserved_ports,
};

use super::{GuestTransportToken, KernelRelease, parse_transport_token};

/// The guest's transport token, read from its kernel command line; panics
/// (fails closed) when this process does not run inside an Overdrive guest.
fn in_guest() -> GuestTransportToken {
    let cmdline = fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    assert!(
        cmdline.contains("overdrive.transport="),
        "not running inside an Overdrive guest (no overdrive.transport= token on /proc/cmdline): \
         these cells run as a microVM guest's operator command, never on a host"
    );
    parse_transport_token(&cmdline).expect("the guest's token parses")
}

fn read_trimmed(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")).trim().to_owned()
}

/// The guest transport is configured on the real guest kernel before READY,
/// with each step's observable postcondition: the workload address is local
/// (a bind to it succeeds), `net.ipv4.fwmark_reflect = 1`,
/// `net.ipv4.ip_local_reserved_ports` = `guest_reserved_ports()`, every intake
/// is bound (TCP 61,000 and the 1,024 UDP slots refuse a second bind), the
/// forwarder module is loaded and referenced and its device present, and the
/// inbound vsock listener holds (`VMADDR_CID_ANY`, `GUEST_INBOUND_PORT`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_transport_is_configured_on_the_real_guest_kernel() {
    let token = in_guest();
    // the workload address is assigned to a local device
    let local = TcpListener::bind(SocketAddrV4::new(token.workload_addr, 0));
    assert!(local.is_ok(), "workload_addr {} is local: {local:?}", token.workload_addr);
    assert_eq!(read_trimmed("/proc/sys/net/ipv4/fwmark_reflect"), "1");
    let reserved = guest_reserved_ports();
    assert_eq!(
        read_trimmed("/proc/sys/net/ipv4/ip_local_reserved_ports"),
        format!("{}-{}", reserved.start(), reserved.end())
    );
    // every intake is bound by overdrive-init
    let tcp = TcpListener::bind(SocketAddrV4::new(token.workload_addr, GUEST_TCP_INTAKE_PORT));
    assert_eq!(
        tcp.map(|_| ()).map_err(|e| e.kind()),
        Err(ErrorKind::AddrInUse),
        "the TCP intake is bound"
    );
    for n in 0..GUEST_UDP_SLOTS {
        let slot = SocketAddrV4::new(token.workload_addr, GUEST_UDP_SLOT_BASE + n);
        assert_eq!(
            UdpSocket::bind(slot).map(|_| ()).map_err(|e| e.kind()),
            Err(ErrorKind::AddrInUse),
            "slot {slot} is bound"
        );
    }
    // the forwarder module is loaded and referenced, and its device present:
    // the guest owner's open context holds a module reference
    // (MODULE-REFCOUNT), so on a kernel with module unloading the line's
    // reference count is at least 1; a kernel without unloading prints `-`
    // (kernel/module/procfs.c:32-45 @ v7.0).
    let modules = read_trimmed("/proc/modules");
    let line = modules.lines().find(|l| l.starts_with("ovd_fwd ")).expect("ovd_fwd is loaded");
    let refcount = line.split_whitespace().nth(2).expect("a /proc/modules line has a refcount");
    assert!(
        refcount == "-" || refcount.parse::<u64>().is_ok_and(|n| n >= 1),
        "ovd_fwd is referenced by the guest owner's context: {line}"
    );
    assert!(fs::metadata("/dev/ovd_fwd").is_ok(), "the forwarder device exists");
    // the inbound vsock listener is bound
    let fd = socket(AddressFamily::Vsock, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .expect("a vsock socket");
    let any = VsockAddr::new(nix::libc::VMADDR_CID_ANY, GUEST_INBOUND_PORT.as_u32());
    assert_eq!(
        bind(std::os::fd::AsRawFd::as_raw_fd(&fd), &any),
        Err(Errno::EADDRINUSE),
        "the inbound vsock listener holds GUEST_INBOUND_PORT"
    );
}

/// R5-15 (D25-BIND): an application TCP bind to 61,000 and a UDP bind to
/// 61,001 or 62,024 — on `workload_addr` and on `0.0.0.0` — fail `EADDRINUSE`
/// deterministically; a TCP bind to 61,001 and a UDP bind to 61,000 succeed
/// (an intake holds only its own protocol's port); and no ephemeral port the
/// kernel assigns falls in the reserved range.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_reserved_intake_ports_refuse_application_binds_of_their_protocol_only() {
    let token = in_guest();
    let last_slot = GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1;
    for ip in [token.workload_addr, Ipv4Addr::UNSPECIFIED] {
        let tcp = TcpListener::bind(SocketAddrV4::new(ip, GUEST_TCP_INTAKE_PORT))
            .map(|_| ())
            .map_err(|e| e.kind());
        assert_eq!(tcp, Err(ErrorKind::AddrInUse), "TCP {ip}:{GUEST_TCP_INTAKE_PORT}");
        for port in [GUEST_UDP_SLOT_BASE, last_slot] {
            let udp =
                UdpSocket::bind(SocketAddrV4::new(ip, port)).map(|_| ()).map_err(|e| e.kind());
            assert_eq!(udp, Err(ErrorKind::AddrInUse), "UDP {ip}:{port}");
        }
        let inverse_tcp = TcpListener::bind(SocketAddrV4::new(ip, GUEST_UDP_SLOT_BASE));
        assert!(
            inverse_tcp.is_ok(),
            "TCP {ip}:{GUEST_UDP_SLOT_BASE} is not an intake: {inverse_tcp:?}"
        );
        drop(inverse_tcp);
        let inverse_udp = UdpSocket::bind(SocketAddrV4::new(ip, GUEST_TCP_INTAKE_PORT));
        assert!(
            inverse_udp.is_ok(),
            "UDP {ip}:{GUEST_TCP_INTAKE_PORT} is not an intake: {inverse_udp:?}"
        );
    }
    // no ephemeral port in the reserved range: the kernel's own choice, for
    // binds and for connects
    let reserved = guest_reserved_ports();
    let listener =
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("a loopback listener");
    let target = listener.local_addr().expect("its address");
    // Every socket stays open so the kernel cannot reuse its port.
    #[allow(
        clippy::collection_is_never_read,
        reason = "the sockets are held so their ports stay taken"
    )]
    let mut held = Vec::new();
    for _ in 0..4_096 {
        let udp = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
            .expect("an ephemeral UDP bind");
        let port = udp.local_addr().expect("bound").port();
        assert!(!reserved.contains(&port), "ephemeral UDP port {port} is reserved");
        let tcp = TcpStream::connect(target).expect("a loopback connect");
        let port = tcp.local_addr().expect("connected").port();
        assert!(!reserved.contains(&port), "ephemeral TCP port {port} is reserved");
        held.push((udp, tcp, listener.accept().expect("accepted")));
    }
}

/// EVMOD-GUEST-KIT: the forwarder module at `GUEST_FORWARDER_MODULE_PATH` was
/// built for the running kernel — the first word of its `vermagic` equals
/// `uname(2)`'s release, both valid `KernelRelease`s — which is the check
/// `overdrive-init` makes before `finit_module` (a module of another release
/// is `ForwarderModuleRelease`, on every kernel configuration).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_guest_kits_module_release_matches_the_running_kernel() {
    in_guest();
    let module = fs::read(GUEST_FORWARDER_MODULE_PATH).expect("the guest kit's module is present");
    let marker = b"vermagic=";
    let at = module
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("the module carries a vermagic in .modinfo");
    let rest = &module[at + marker.len()..];
    let end = rest.iter().position(|b| *b == 0).expect("a NUL-terminated vermagic");
    let vermagic = std::str::from_utf8(&rest[..end]).expect("vermagic is UTF-8");
    let module_release = vermagic.split_whitespace().next().expect("vermagic names a release");
    // the running kernel's release (`uname(2)`'s, as `/proc` reports it)
    let kernel_release = read_trimmed("/proc/sys/kernel/osrelease");
    let module_release =
        KernelRelease::from_str(module_release).expect("the module's release is a KernelRelease");
    let kernel_release =
        KernelRelease::from_str(&kernel_release).expect("the kernel's release is a KernelRelease");
    assert_eq!(
        module_release, kernel_release,
        "the VM's kernel and root filesystem come from one guest kit"
    );
}
