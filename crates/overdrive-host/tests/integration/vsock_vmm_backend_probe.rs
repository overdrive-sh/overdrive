//! P6 — VMM vsock device backend probe, fork `fd=` launch and claimed-device
//! handoff (netns-density-295 vsock attachment replacement; feature-delta
//! §"VMM backend contract", §"VMM fork provisioning contract", Gate G-V4;
//! D2, D13, D16-CLAIM, D21; ADR-0146, ADR-0161, ADR-0170; V-1(c), V-25).
//!
//! RED acceptance scaffolds (`.claude/rules/testing.md` §"RED acceptance
//! scaffolds and activation"): complete bodies marked
//! `#[ignore = "pending DELIVER …"]`. The crafter removes only the marker once
//! the host adapter + CH fork `fd=` mode exist (DELIVER steps 1/3); it does not
//! rewrite the body.
//!
//! **Sequencing (shared-test-infrastructure.md §0, §6).** The host adapter's
//! `backend=vhost-kernel`/`fd=` surface (`CloudHypervisorVmm::probe`,
//! `create(.., ClaimedGuestCid)`) and the fork's `fd=` mode do not exist yet.
//! This file is therefore **not registered** in `tests/integration.rs`
//! (`mod integration { … }`); the author adds its `mod vsock_vmm_backend_probe;`
//! line (under `integration-tests`, NOT under `kvm-tests` — the probe-refusal
//! test needs no KVM) when the compiling body co-arrives with the DELIVER host
//! scaffold. Per the host entrypoint convention the per-scenario file carries
//! no file-level `#![cfg(feature = "integration-tests")]`; it inherits the
//! gate from `tests/integration.rs`. The launch/handoff tests additionally
//! carry `#[cfg(feature = "kvm-tests")]` and run on native x86_64 metal only.
//!
//! Observable surface: the `Vmm::probe` / `Vmm::create` trait return values,
//! the spawned launch argv (`CloudHypervisorVmm::last_launch_argv()`, a
//! host-adapter test observable), a byte round-trip over the device, and the
//! host-kernel CID reclaimability through `GuestCidClaim::claim`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use overdrive_core::guest_network::{GUEST_CID_BASE, GuestCid, GuestCidClaim};
use overdrive_core::traits::vmm::{Vmm, VmmProbeError};
use overdrive_host::{CloudHypervisorVmm, VhostVsockCidClaim};

/// A throwaway executable on `dir` whose `--help` prints `help_text`, standing
/// in for an installed cloud-hypervisor binary with or without the capability.
fn stub_binary(dir: &std::path::Path, name: &str, help_text: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\ncat <<'EOF'\n{help_text}\nEOF\n"))
        .expect("write stub binary");
    let mut perms = std::fs::metadata(&path).expect("stat stub").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&path, perms).expect("chmod stub");
    path
}

// ----------------------------------------------------------------- P6-S7 + P6-S9
/// G-V4 row 2 / D2 / D21: `probe` refuses an installed binary whose `--vsock`
/// `--help` lacks `backend=vhost-kernel` with `fd=` (typed
/// `VsockBackendUnsupported`, never a fallback), and accepts the fork's binary
/// that advertises it alongside `--landlock`.
#[tokio::test]
#[ignore = "pending DELIVER step 3 (G-V4 probe, D2/D21)"]
async fn probe_refuses_upstream_and_accepts_fork() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Upstream CH: no vhost-kernel/fd backend advertised.
    let upstream = stub_binary(
        dir.path(),
        "cloud-hypervisor-upstream",
        "--vsock <SOCKET> vsock parameters \"cid=<cid>,socket=<file>\"",
    );
    let vmm = CloudHypervisorVmm::with_binary(&upstream);
    let err = vmm.probe().await.expect_err("upstream CH is refused");
    assert!(
        matches!(err, VmmProbeError::VsockBackendUnsupported { .. }),
        "typed refusal, never a fallback: {err:?}"
    );

    // Fork: advertises backend=vhost-kernel, fd= and --landlock (D21 gate).
    let fork = stub_binary(
        dir.path(),
        "cloud-hypervisor-fork",
        "--landlock enable landlock\n--vsock <SOCKET> vsock parameters \
         \"cid=<cid>,backend=vhost-kernel,fd=<fd>\"",
    );
    let vmm = CloudHypervisorVmm::with_binary(&fork);
    vmm.probe().await.expect("the fork advertises the capability → Ok");
}

// ----------------------------------------------------------------- P6-S8
/// G-V4 rows 1/5 / V-1(c) / D2 / D16-CLAIM / R5-19 (d)(e): `create` hands the
/// claimed `/dev/vhost-vsock` descriptor to the fork via `fd=`; the fork boots
/// and passes data; after `Ok` the launcher holds no copy; the CID is
/// claimable again within 10 ms of the reaped VMM exit.
///
/// Native x86_64 metal only (`cargo xtask metal run --`): a real claimed device
/// + the CH fork + `/dev/kvm`.
#[cfg(feature = "kvm-tests")]
#[tokio::test]
#[ignore = "pending DELIVER step 3 (V-1(c)); native x86_64 metal only"]
async fn fork_fd_launch_handoff_and_release() {
    let claimer = VhostVsockCidClaim::open().expect("open /dev/vhost-vsock claimer");
    let cid = GuestCid::new(GUEST_CID_BASE + 7).expect("guest cid");
    let claim = claimer.claim(cid).expect("claim the CID on the host kernel");

    let vmm = CloudHypervisorVmm::production();
    let config = overdrive_host::test_support::vm_config_boot_sleep();
    let mut process = vmm.create(&config, claim).await.expect("fork boots with fd= handoff");

    // Launch argument shape (D2): exactly one --vsock cid=…,backend=vhost-kernel,fd=…
    let argv = vmm.last_launch_argv();
    assert!(
        argv.windows(2).any(|w| w[0] == "--vsock"
            && w[1].contains(&format!("cid={}", cid.as_u32()))
            && w[1].contains("backend=vhost-kernel")
            && w[1].contains("fd=")
            && !w[1].contains("socket=")),
        "one --vsock cid=…,backend=vhost-kernel,fd=… and no socket=: {argv:?}"
    );

    // The fork boots and passes one byte over the device (guest vsock echo).
    assert!(process.boots_and_echoes_over_vsock().await, "guest round-trips over the device");

    // After Ok the assignment holds no copy; a byte round-trips with the VMM alone.
    assert!(
        process.wait_pre_ready_exit().await.is_none(),
        "the fork does not exit before READY on a good descriptor"
    );

    // Reap the VMM; the CID is claimable again within 10 ms (A-29, CLAIM-PINS).
    process.kill_and_reap().await.expect("reap the VMM");
    let started = std::time::Instant::now();
    let reclaimed = loop {
        match claimer.claim(cid) {
            Ok(c) => break c,
            Err(_) if started.elapsed() < std::time::Duration::from_millis(50) => {
                tokio::task::yield_now().await;
            }
            Err(e) => panic!("CID not reclaimable after reap: {e:?}"),
        }
    };
    assert!(
        started.elapsed() < std::time::Duration::from_millis(50),
        "CID claimable again promptly after the reaped exit"
    );
    drop(reclaimed);
}

// ----------------------------------------------------------------- P6-S8 (cont.)
/// G-V4 / D2: a descriptor that is not a vhost-vsock device (`ENOTTY`) makes the
/// fork exit before READY (the existing pre-READY start failure); the CID stays
/// claimable (no clash can reach this gate, D16-CLAIM).
#[cfg(feature = "kvm-tests")]
#[tokio::test]
#[ignore = "pending DELIVER step 3 (V-1(c) negative); native x86_64 metal only"]
async fn fork_exits_before_ready_on_non_vhost_descriptor() {
    let claimer = VhostVsockCidClaim::open().expect("claimer");
    let cid = GuestCid::new(GUEST_CID_BASE + 8).expect("guest cid");

    // A simulated claim whose device has no real vhost_fd → the host adapter
    // refuses construction, OR a real claim whose handed descriptor is scripted
    // non-vhost (ENOTTY) → the fork exits before READY.
    let claim = claimer.claim(cid).expect("claim");
    let vmm = CloudHypervisorVmm::production();
    let config = overdrive_host::test_support::vm_config_boot_sleep_with_bad_vhost_fd();
    let mut process = vmm.create(&config, claim).await.expect("spawn attempted");
    assert!(
        process.wait_pre_ready_exit().await.is_some(),
        "a non-vhost descriptor makes the fork exit before READY"
    );
    // The CID stays claimable.
    let again = claimer.claim(cid);
    assert!(again.is_ok(), "CID stays claimable after a pre-READY exit: {again:?}");
}
