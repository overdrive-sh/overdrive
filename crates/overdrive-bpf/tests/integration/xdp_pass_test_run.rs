//! Tier 2 BPF unit triptych for the no-op `xdp_pass` program.
//!
//! Per `.claude/rules/testing.md` § "Tier 2 — BPF Unit Tests" each
//! eBPF program ships a PKTGEN/SETUP/CHECK triptych under
//! `crates/overdrive-bpf/tests/`:
//!
//! - **PKTGEN** — synthesise a minimal Ethernet+IPv4+TCP packet.
//! - **SETUP** — clear the `LruHashMap<u32, u64>` `PKTS` packet
//!   counter map (default-isolation per testing.md).
//! - **CHECK** — drive `BPF_PROG_TEST_RUN` against the loaded
//!   `xdp_pass` program and assert (a) the returned action ==
//!   `XDP_PASS` (verdict assertion) AND (b) the counter map's
//!   value at key 0 transitioned 0 -> 1 (state assertion).
//!
//! Linux-only — `BPF_PROG_TEST_RUN` is a Linux syscall and aya's
//! userspace API requires libbpf-sys. macOS skips this test entirely
//! (the parent `tests/integration.rs` compiles fine; the test
//! function is `#[cfg(target_os = "linux")]`-gated).
//!
//! Implementation note on `test_run`: aya 0.13.1 does NOT expose a
//! safe `Xdp::test_run` wrapper. We drive the syscall directly via
//! `libc::syscall(SYS_bpf, BPF_PROG_TEST_RUN, &mut bpf_attr, ...)`
//! using the `bpf_attr` union from `aya_obj::generated` — the same
//! shape aya itself uses internally for `BPF_PROG_LOAD`,
//! `BPF_MAP_LOOKUP_ELEM`, etc. The program FD comes from
//! `aya::programs::Program::fd().as_fd()`.

#![cfg(target_os = "linux")]

use std::path::PathBuf;

use aya::{
    Ebpf,
    maps::HashMap,
    programs::{ProgramFd, Xdp},
};
use aya_obj::generated::{bpf_attr, bpf_cmd::BPF_PROG_TEST_RUN};
use serial_test::serial;

/// `XDP_PASS` from the Linux uapi `<bpf.h>` (`enum xdp_action`).
/// Hardcoded rather than pulled from `aya_ebpf_bindings` because
/// (a) `aya-ebpf` is a kernel-side dependency only — adding it as a
/// host-side dev-dep would dirty the dep graph just for one constant,
/// and (b) the value is part of the Linux kernel ABI and will not
/// change. Verified against
/// `aya-ebpf-bindings-0.1.2/src/<arch>/bindings.rs` (`pub const
/// XDP_PASS: Type = 2`).
const XDP_PASS: u32 = 2;

/// Walk up from `crates/overdrive-bpf`'s manifest dir to the
/// workspace root. The BPF artifact at
/// `target/xtask/bpf-objects/overdrive_bpf.o` is workspace-relative.
///
/// `crates/overdrive-bpf/` -> pop twice (crate name + `crates/`).
fn workspace_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest);
    p.pop(); // remove `overdrive-bpf`
    p.pop(); // remove `crates`
    p
}

fn bpf_artifact_path() -> PathBuf {
    workspace_root().join("target/xtask/bpf-objects/overdrive_bpf.o")
}

/// PKTGEN — synthesise a minimal Ethernet (14B) + IPv4 (20B) +
/// TCP (20B) frame. The `xdp_pass` program does not parse the
/// packet — it just increments `PKTS[0]` and returns `XDP_PASS` —
/// so any well-formed Ethernet-shaped buffer passes the kernel's
/// `BPF_PROG_TEST_RUN` minimum-size check (32 bytes for XDP).
fn synthesise_eth_ipv4_tcp() -> Vec<u8> {
    let mut pkt = Vec::with_capacity(54);
    // Ethernet (14B): dst MAC, src MAC, ethertype 0x0800 (IPv4)
    pkt.extend_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56]); // dst
    pkt.extend_from_slice(&[0x52, 0x54, 0x00, 0xab, 0xcd, 0xef]); // src
    pkt.extend_from_slice(&[0x08, 0x00]); // ethertype = IPv4
    // IPv4 (20B): version+IHL=0x45, TOS=0, total_len=40, id=0,
    //             flags+frag=0, TTL=64, proto=TCP(6), checksum=0,
    //             src=10.0.0.1, dst=10.0.0.2
    pkt.extend_from_slice(&[0x45, 0x00, 0x00, 0x28]); // ver/ihl, TOS, total_len
    pkt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // id, flags+frag
    pkt.extend_from_slice(&[0x40, 0x06, 0x00, 0x00]); // TTL, proto, checksum
    pkt.extend_from_slice(&[10, 0, 0, 1]); // src
    pkt.extend_from_slice(&[10, 0, 0, 2]); // dst
    // TCP (20B): src_port=12345, dst_port=80, seq=0, ack=0,
    //            data_off=0x50, flags=0x02 (SYN), win=8192, csum=0, urg=0
    pkt.extend_from_slice(&[0x30, 0x39, 0x00, 0x50]); // ports
    pkt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // seq
    pkt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // ack
    pkt.extend_from_slice(&[0x50, 0x02, 0x20, 0x00]); // data_off, flags, win
    pkt.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // csum, urg
    debug_assert_eq!(pkt.len(), 54);
    pkt
}

/// Drive `BPF_PROG_TEST_RUN` for the program identified by
/// `prog_fd`, feeding `data_in` as the packet bytes. Returns the
/// program's `retval` (the XDP action for an XDP program).
///
/// Mirrors the syscall path in libbpf's `bpf_prog_test_run_opts`
/// and aya's own internal `sys_bpf` calls, scoped to the test_run
/// command. We zero the `bpf_attr` union, populate the
/// `attr.test` arm (`bpf_attr__bindgen_ty_7`), and invoke the
/// `bpf(2)` syscall directly.
fn bpf_prog_test_run(prog_fd: &ProgramFd, data_in: &[u8]) -> Result<u32, std::io::Error> {
    use std::os::fd::AsFd;

    // SAFETY: `bpf_attr` is a `repr(C) union` of `repr(C) struct`s
    // with no destructor; zero-init is the canonical way to build
    // it (matches aya's internal helper `bpf_attr` zero-init).
    let mut attr: bpf_attr = unsafe { std::mem::zeroed() };

    // SAFETY: writing the `test` arm of the union — the only arm we
    // touch — and reading no other arm before the syscall.
    let test = unsafe { &mut attr.test };
    test.prog_fd = prog_fd.as_fd().as_raw_fd_u32();
    test.data_in = data_in.as_ptr() as u64;
    test.data_size_in = data_in
        .len()
        .try_into()
        .expect("packet larger than u32::MAX bytes (impossible for this test)");
    test.repeat = 1;
    // No data_out / ctx — kernel allocates output internally for
    // size queries; we don't read packet output, only retval.

    // SAFETY: `libc::syscall` for SYS_bpf with a valid command and
    // a properly-sized `bpf_attr` is the standard kernel ABI for
    // BPF operations. The size argument is `size_of::<bpf_attr>()`,
    // matching the kernel's expected layout.
    let ret = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_PROG_TEST_RUN as libc::c_int,
            &mut attr as *mut bpf_attr,
            std::mem::size_of::<bpf_attr>() as libc::c_uint,
        )
    };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: kernel populated `attr.test.retval` on success.
    Ok(unsafe { attr.test.retval })
}

/// Tiny extension trait so the syscall site reads cleanly. The
/// `aya::programs::Program::fd()` chain returns `&ProgramFd`;
/// `BorrowedFd::as_raw_fd` is i32, but the `bpf_attr.prog_fd`
/// field is `u32` per the kernel ABI.
trait AsRawFdU32 {
    fn as_raw_fd_u32(&self) -> u32;
}

impl AsRawFdU32 for std::os::fd::BorrowedFd<'_> {
    fn as_raw_fd_u32(&self) -> u32 {
        use std::os::fd::AsRawFd;
        self.as_raw_fd() as u32
    }
}

/// Tier 2 BPF unit triptych — `xdp_pass` returns `XDP_PASS` and
/// the `PKTS` counter increments from 0 to 1 across one
/// `BPF_PROG_TEST_RUN` invocation.
///
/// Acceptance criteria 03-01 §3:
///   (a) returned action == `aya_obj::generated::xdp_action::XDP_PASS`
///   (b) `PKTS[0]` transitions 0 -> 1 (read pre and post)
///
/// `#[serial(env)]` because the BPF artifact path is process-global
/// state (per testing.md § "Tests that mutate process-global state");
/// other tests in this binary that load the same artifact must not
/// race with this one's load + program-attach lifecycle.
#[test]
#[serial(env)]
fn bpf_unit_runs_xdp_pass_triptych_via_bpf_prog_test_run() {
    let artifact = bpf_artifact_path();
    assert!(
        artifact.exists(),
        "BPF artifact missing at {} — run `cargo xtask bpf-build` first",
        artifact.display(),
    );

    // SETUP: load the BPF object and resolve the `xdp_pass` program
    // and `PKTS` map. `Ebpf::load_file` reads the ELF and creates
    // userspace handles; programs and maps are not yet loaded into
    // the kernel.
    let mut bpf = Ebpf::load_file(&artifact)
        .unwrap_or_else(|e| panic!("aya load_file({}): {e}", artifact.display()));

    // Resolve & load the program kernel-side, then take an owned
    // FD copy — `program_mut` and `map_mut` both borrow `bpf` so we
    // must release the program borrow before reaching for the map.
    let prog_fd = {
        let prog: &mut Xdp = bpf
            .program_mut("xdp_pass")
            .expect("xdp_pass program not found in BPF object")
            .try_into()
            .expect("xdp_pass program is not an Xdp program");
        prog.load().expect("xdp_pass.load");
        prog.fd().expect("xdp_pass.fd() before test_run").try_clone().expect("ProgramFd::try_clone")
    };

    // SETUP (cont.): clear PKTS[0] so the counter assertion starts
    // from a clean baseline. `remove` returns `KeyNotFound` on first
    // run; that's fine.
    {
        let mut pkts: HashMap<_, u32, u64> =
            HashMap::try_from(bpf.map_mut("PKTS").expect("PKTS map not found in BPF object"))
                .expect("PKTS HashMap::try_from");
        let _ = pkts.remove(&0_u32); // ignore KeyNotFound
        // Verify clean baseline — `get` returns `KeyNotFound` after
        // remove, which is the "0 packets seen" state.
        match pkts.get(&0_u32, 0) {
            Err(aya::maps::MapError::KeyNotFound) => {}
            other => panic!("expected KeyNotFound on cleared PKTS[0]; got {other:?}"),
        }
    }

    // PKTGEN: build the synthetic frame.
    let pkt = synthesise_eth_ipv4_tcp();

    // CHECK: drive BPF_PROG_TEST_RUN.
    let action = bpf_prog_test_run(&prog_fd, &pkt).expect("BPF_PROG_TEST_RUN syscall");

    // (a) verdict assertion — the program returns XDP_PASS.
    assert_eq!(action, XDP_PASS, "expected XDP_PASS verdict (=2), got action={action}",);

    // (b) state assertion — PKTS[0] transitioned 0 -> 1.
    let pkts: HashMap<_, u32, u64> =
        HashMap::try_from(bpf.map("PKTS").expect("PKTS map not found in BPF object"))
            .expect("PKTS HashMap::try_from (post)");
    let post = pkts.get(&0_u32, 0).expect("PKTS[0] should exist after one test_run invocation");
    assert_eq!(
        post, 1,
        "expected PKTS[0] to transition 0 -> 1 across one BPF_PROG_TEST_RUN; got {post}",
    );
}
