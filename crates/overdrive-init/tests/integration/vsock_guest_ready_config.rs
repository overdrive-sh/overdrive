//! P6 — `overdrive-init` guest network configuration before READY, driven as a
//! subprocess of the real `overdrive-init` binary (netns-density-295 vsock
//! attachment replacement; feature-delta §"Guest adaptation contract", Gate
//! G-V7; ADR-0082 §D4, ADR-0150; V-6).
//!
//! RED acceptance scaffolds (`.claude/rules/testing.md` §"RED acceptance
//! scaffolds and activation"): complete bodies marked
//! `#[ignore = "pending DELIVER …"]`. The crafter removes only the marker once
//! `overdrive-init`'s vsock2 transport-token parse + guest-network
//! configuration exist (DELIVER guest-init step); it does not rewrite the body.
//!
//! **Sequencing (shared-test-infrastructure.md §0, §6).** `overdrive-init` is a
//! binary crate with no library target ("Not a library any other crate
//! imports") and no `tests/` entrypoint today. Per §6 the first of P3/P6 to
//! need guest-side Rust tests creates `crates/overdrive-init/tests/integration.rs`
//! (`#![cfg(feature = "integration-tests")] mod integration { … }`) when a
//! compiling body co-arrives with the DELIVER guest-init scaffold; until then
//! this file is **not registered**, is not a cargo target, and is never
//! compiled, so the build stays green.
//!
//! **What this host file covers (§6):** the host-runnable slice of G-V7 — the
//! transport-token parse and the typed early-exit-before-READY behaviour,
//! exercised by spawning the real binary. The genuine in-guest configuration
//! (dummy device, fwmark rule + `fwmark_reflect`, reserved-port intakes,
//! attached guest programs, seeded listener map, inbound vsock listener) runs
//! on a real guest kernel and is the **Tier-3 cell inside the P6 walking
//! skeleton** (`tests/conformance/tests/vsock_walking_skeleton.rs`), never a
//! standalone host test. The pure transport-token parse PBT roundtrip belongs
//! in `overdrive-init/src/main.rs` `#[cfg(test)]` (DELIVER source work; outside
//! P6's file set — surfaced as a gap).
//!
//! **Testability surface P6 shapes:** `overdrive-init` reads its transport
//! token from the kernel command line (`/proc/cmdline`) as PID 1. Because a
//! host test is not PID 1, the binary honours `OVERDRIVE_INIT_CMDLINE` as the
//! command-line source **only when it is not PID 1** (a documented test-mode
//! override; DELIVER implements it in `main.rs`). Observable surface: the
//! process exit status and the typed `GuestTransportSetupError` the binary
//! renders to its console.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::process::Command;

/// Run the real `overdrive-init` binary with `cmdline` as its (overridden)
/// kernel command line, returning its captured stderr and whether it exited
/// non-zero before any READY beacon line.
fn run_init_with_cmdline(cmdline: &str) -> (bool, String) {
    let bin = env!("CARGO_BIN_EXE_overdrive-init");
    let output = Command::new(bin)
        .env("OVERDRIVE_INIT_CMDLINE", cmdline)
        // No beacon endpoint: the binary must fail at token parse / config,
        // before it would dial the beacon, so it never blocks on vsock.
        .env_remove("OVERDRIVE_INIT_BEACON")
        .output()
        .expect("spawn overdrive-init");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exited_nonzero_before_ready =
        !output.status.success() && !String::from_utf8_lossy(&output.stdout).contains("READY");
    (exited_nonzero_before_ready, stderr)
}

const VALID: &str = "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef";

// ----------------------------------------------------------------- P6-S11 (unit-ish, host subprocess)
/// G-V7 row 2: a missing transport token makes `overdrive-init` exit before
/// READY with `GuestTransportSetupError::TokenMissing`.
#[test]
#[ignore = "pending DELIVER guest-init step (G-V7, token missing)"]
fn missing_transport_token_exits_before_ready() {
    let (exited, stderr) = run_init_with_cmdline("console=ttyS0 root=/dev/vda");
    assert!(exited, "no transport token → exit before READY");
    assert!(stderr.contains("TokenMissing"), "typed TokenMissing rendered: {stderr}");
}

/// G-V7 row 2: a duplicate token → `TokenDuplicate`.
#[test]
#[ignore = "pending DELIVER guest-init step (G-V7, token duplicate)"]
fn duplicate_transport_token_exits_before_ready() {
    let (exited, stderr) = run_init_with_cmdline(&format!("{VALID} {VALID}"));
    assert!(exited, "duplicate token → exit before READY");
    assert!(stderr.contains("TokenDuplicate"), "typed TokenDuplicate rendered: {stderr}");
}

/// G-V7 row 2 / C6a: each malformed field and an all-zero launch token →
/// `TokenMalformed { field }` with the offending field named.
#[test]
#[ignore = "pending DELIVER guest-init step (G-V7, token malformed)"]
fn malformed_transport_token_names_the_field() {
    // Wrong version keyword.
    let (exited, stderr) = run_init_with_cmdline(
        "overdrive.transport=vsock1,addr=100.95.0.7,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
    );
    assert!(exited && stderr.contains("Version"), "version field named: {stderr}");

    // Malformed address.
    let (exited, stderr) = run_init_with_cmdline(
        "overdrive.transport=vsock2,addr=not-an-ip,dns=100.95.0.1,launch=0123456789abcdef0123456789abcdef",
    );
    assert!(exited && stderr.contains("Addr"), "addr field named: {stderr}");

    // All-zero launch token (the invalid encoding, ATTR-TOKEN).
    let (exited, stderr) = run_init_with_cmdline(
        "overdrive.transport=vsock2,addr=100.95.0.7,dns=100.95.0.1,launch=00000000000000000000000000000000",
    );
    assert!(
        exited && stderr.contains("Launch"),
        "all-zero launch token rejected, field named: {stderr}"
    );

    // An extra field after launch.
    let (exited, stderr) = run_init_with_cmdline(&format!("{VALID},extra=1"));
    assert!(exited && stderr.contains("Extra"), "extra field named: {stderr}");
}

/// G-V7 row 1 (host-runnable fragment): a well-formed token parses — the binary
/// gets **past** token parse and fails later, at a guest-kernel configuration
/// step that cannot run on the host (e.g. `DummyDevice` / `AddressAdd`), so the
/// rendered error is a configuration-stage variant, never a token variant. This
/// proves the parse succeeded; the genuine configuration is the WS Tier-3 cell.
#[test]
#[ignore = "pending DELIVER guest-init step (G-V7, valid token parses); genuine config = WS Tier-3 cell"]
fn valid_transport_token_parses_then_fails_at_the_guest_kernel_stage() {
    let (exited, stderr) = run_init_with_cmdline(VALID);
    assert!(exited, "on the host the guest-kernel config stage fails after a good parse");
    assert!(
        !stderr.contains("Token"),
        "a well-formed token does not produce a Token* error: {stderr}"
    );
    assert!(
        stderr.contains("DummyDevice")
            || stderr.contains("AddressAdd")
            || stderr.contains("FwmarkRule")
            || stderr.contains("ProgramLoad")
            || stderr.contains("ListenerSeed")
            || stderr.contains("InboundListenerBind")
            || stderr.contains("ReservedPorts")
            || stderr.contains("IntakeBind"),
        "failure is a guest-network configuration stage, not token parse: {stderr}"
    );
}
